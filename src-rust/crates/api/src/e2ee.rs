use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::Aead};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use getrandom::getrandom;
use hkdf::Hkdf;
use pqcrypto_kyber::kyber768::{self, PublicKey as KyberPublicKey, SecretKey as KyberSecretKey, Ciphertext as KyberCiphertext};
use pqcrypto_traits::kem::{PublicKey as PublicKeyTrait, Ciphertext as CiphertextTrait, SharedSecret as SharedSecretTrait};
use sha2::Sha256;
use tracing::debug;

/// ML-KEM-768 constant sizes
pub const ML_KEM_768_PUBKEY_SIZE: usize = 1184;
pub const ML_KEM_768_SECRETKEY_SIZE: usize = 2400;
pub const ML_KEM_768_CIPHERTEXT_SIZE: usize = 1088;
pub const ML_KEM_768_SHARED_SECRET_SIZE: usize = 32;

/// AEAD constants
pub const CHACHA_NONCE_SIZE: usize = 12;
pub const CHACHA_TAG_SIZE: usize = 16;

/// HKDF context strings for protocol versioning
const HKDF_INFO_REQ: &[u8] = b"e2e-req-v1";
const HKDF_INFO_RESP: &[u8] = b"e2e-resp-v1";
const HKDF_INFO_STREAM: &[u8] = b"e2e-stream-v1";
const HKDF_SALT_LEN: usize = 16;

/// Full protocol state for a single request/response cycle
pub struct E2eeSession {
    /// My ephemeral ML-KEM keypair (for decrypting response)
    pub my_response_sk: KyberSecretKey,
    /// My ephemeral ML-KEM public key (encrypted to server in payload)
    pub my_response_pk: KyberPublicKey,
    
    /// ML-KEM ciphertext for server to decapsulate
    pub request_kem_ct: Vec<u8>,
    
    /// Shared secret from first ML-KEM encapsulation
    shared_secret: Vec<u8>,
}

impl std::fmt::Debug for E2eeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("E2eeSession")
            .field("request_kem_ct_len", &self.request_kem_ct.len())
            .field("shared_secret_len", &self.shared_secret.len())
            .finish()
    }
}

impl E2eeSession {
    /// Generate a fresh session, encapsulate to instance's pub_key
    pub fn new(instance_pubkey: &KyberPublicKey) -> Self {
        // 1. Generate ephemeral response keypair
        let (my_response_pk, my_response_sk) = kyber768::keypair();
        
        // 2. Encapsulate to server's public key (creates shared secret for request)
        let (shared_secret, ciphertext) = kyber768::encapsulate(instance_pubkey);
        
        Self {
            my_response_sk,
            my_response_pk,
            request_kem_ct: ciphertext.as_bytes().to_vec(),
            shared_secret: shared_secret.as_bytes().to_vec(),
        }
    }

    /// Derive symmetric key using HKDF-SHA256
    /// 
    /// Formula from e2ee-proxy:
    /// HKDF-SHA256(ikm=shared_secret, salt=mlkem_ct[0:16], info=INFO_STR)
    fn derive_key(shared_secret: &[u8], salt: &[u8], info: &[u8]) -> Vec<u8> {
        let hkdf = Hkdf::<Sha256>::new(Some(salt), shared_secret);
        let mut key = vec![0u8; 32];
        hkdf.expand(info, &mut key).unwrap();
        key
    }

    /// Encrypt complete request payload using full protocol
    pub fn encrypt_request(&self, json_payload: &str) -> Result<Vec<u8>, E2eeError> {
        use base64::Engine;
        
        // 1. Augment payload with my response pub key (base64)
        let response_pk_b64 = base64::engine::general_purpose::STANDARD.encode(
            PublicKeyTrait::as_bytes(&self.my_response_pk)
        );
        
        let mut payload: serde_json::Value = serde_json::from_str(json_payload)
            .map_err(|e| E2eeError::EncryptionFailed(format!("JSON parse: {}", e)))?;
        payload["e2e_response_pk"] = serde_json::Value::String(response_pk_b64);
        let augmented_json = payload.to_string();

        // 2. Gzip compress
        let compressed = compress_gzip(augmented_json.as_bytes())?;

        // 3. Derive request encryption key
        //    salt = first 16 bytes of ML-KEM ciphertext
        let salt = &self.request_kem_ct[..HKDF_SALT_LEN];
        let req_key = Self::derive_key(&self.shared_secret, salt, HKDF_INFO_REQ);

        // 4. Generate random nonce for AEAD
        let mut nonce = [0u8; CHACHA_NONCE_SIZE];
        getrandom(&mut nonce)
            .map_err(|e| E2eeError::EncryptionFailed(format!("getrandom: {}", e)))?;

        // 5. Encrypt with ChaCha20-Poly1305
        let cipher = ChaCha20Poly1305::new_from_slice(&req_key)
            .map_err(|e| E2eeError::EncryptionFailed(format!("invalid key: {}", e)))?;
        let ciphertext = cipher.encrypt(&nonce.into(), compressed.as_ref())
            .map_err(|e| E2eeError::EncryptionFailed(format!("AEAD: {}", e)))?;

        // 6. Build final blob: [KEM_CT (1088)] + [NONCE (12)] + [CIPHERTEXT+TAG (N)]
        //    ciphertext already includes 16-byte tag at the end
        let mut blob = Vec::with_capacity(
            ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE + ciphertext.len()
        );
        blob.extend_from_slice(&self.request_kem_ct);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);

        debug!(
            send_orig_len = augmented_json.len(),
            send_compressed_len = compressed.len(),
            send_ct_len = ciphertext.len(),
            send_total_blob_len = blob.len(),
            "E2EE request built"
        );

        Ok(blob)
    }

    /// Decrypt a non-streaming response blob
    pub fn decrypt_response(&self, response_blob: &[u8]) -> Result<Vec<u8>, E2eeError> {
        if response_blob.len() < ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE + CHACHA_TAG_SIZE {
            return Err(E2eeError::DecryptionFailed(format!(
                "response too short: expected at least {}, got {}",
                ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE + CHACHA_TAG_SIZE,
                response_blob.len()
            )));
        }

        let mlkem_ct = &response_blob[..ML_KEM_768_CIPHERTEXT_SIZE];
        let nonce = &response_blob[ML_KEM_768_CIPHERTEXT_SIZE..ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE];
        let ct_with_tag = &response_blob[ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE..];

        let ct_obj = KyberCiphertext::from_bytes(mlkem_ct)
            .map_err(|e| E2eeError::CryptoError(format!("invalid ML-KEM ciphertext: {:?}", e)))?;
        
        let response_ss = kyber768::decapsulate(&ct_obj, &self.my_response_sk);
        let ss_bytes = response_ss.as_bytes();

        let salt = &mlkem_ct[..HKDF_SALT_LEN];
        let resp_key = Self::derive_key(ss_bytes, salt, HKDF_INFO_RESP);

        let cipher = ChaCha20Poly1305::new_from_slice(&resp_key)
            .map_err(|e| E2eeError::DecryptionFailed(format!("invalid response key: {}", e)))?;
        let plaintext = cipher.decrypt(nonce.into(), ct_with_tag)
            .map_err(|e| E2eeError::DecryptionFailed(format!("decrypt auth: {}", e)))?;

        let decompressed = decompress_gzip(&plaintext)?;

        debug!(
            recv_orig_len = decompressed.len(),
            recv_ct_len = ct_with_tag.len(),
            recv_total_blob_len = response_blob.len(),
            "E2EE response decrypted"
        );

        Ok(decompressed)
    }

    /// Stream mode: decrypt the e2e_init payload to get stream key
    pub fn decrypt_stream_init(&self, mlkem_ct_b64: &str) -> Result<Vec<u8>, E2eeError> {
        use base64::Engine;
        
        let mlkem_ct = base64::engine::general_purpose::STANDARD.decode(mlkem_ct_b64)
            .map_err(|e| E2eeError::CryptoError(format!("invalid base64 for e2e_init: {}", e)))?;
        
        if mlkem_ct.len() != ML_KEM_768_CIPHERTEXT_SIZE {
            return Err(E2eeError::CryptoError(format!(
                "invalid e2e_init ciphertext length: {}", mlkem_ct.len()
            )));
        }

        let ct_obj = KyberCiphertext::from_bytes(&mlkem_ct)
            .map_err(|e| E2eeError::CryptoError(format!("invalid ML-KEM ciphertext: {:?}", e)))?;
        
        let shared_secret = kyber768::decapsulate(&ct_obj, &self.my_response_sk);
        let ss_bytes = shared_secret.as_bytes();

        // Derive
        let stream_key = Self::derive_key(ss_bytes, &mlkem_ct[..HKDF_SALT_LEN], HKDF_INFO_STREAM);
        Ok(stream_key)
    }

    /// Stream mode: decrypt one encrypted chunk (base64-encoded)
    pub fn decrypt_stream_chunk(&self, enc_chunk_b64: &str, stream_key: &[u8]) -> Result<Vec<u8>, E2eeError> {
        use base64::Engine;
        
        let raw = base64::engine::general_purpose::STANDARD.decode(enc_chunk_b64)
            .map_err(|e| E2eeError::DecryptionFailed(format!("base64 chunk: {}", e)))?;

        if raw.len() < CHACHA_NONCE_SIZE + CHACHA_TAG_SIZE {
            return Err(E2eeError::DecryptionFailed("stream chunk too short".into()));
        }

        let nonce = &raw[..CHACHA_NONCE_SIZE];
        let ct_with_tag = &raw[CHACHA_NONCE_SIZE..];

        let cipher = ChaCha20Poly1305::new_from_slice(stream_key)
            .map_err(|e| E2eeError::DecryptionFailed(format!("invalid stream key: {}", e)))?;
        
        cipher.decrypt(nonce.into(), ct_with_tag)
            .map_err(|e| E2eeError::DecryptionFailed(format!("stream decrypt: {}", e)))
    }
}

/// Gzip compress data
pub fn compress_gzip(data: &[u8]) -> Result<Vec<u8>, E2eeError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut encoder, data)
        .map_err(|e| E2eeError::CompressionFailed(format!("gzip write: {}", e)))?;
    encoder.finish()
        .map_err(|e| E2eeError::CompressionFailed(format!("gzip finish: {}", e)))
}

/// Gzip decompress data
pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, E2eeError> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut decompressed)
        .map_err(|e| E2eeError::DecompressionFailed(format!("gzip: {}", e)))?;
    Ok(decompressed)
}

#[derive(Debug, thiserror::Error)]
pub enum E2eeError {
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("compression failed: {0}")]
    CompressionFailed(String),
    #[error("decompression failed: {0}")]
    DecompressionFailed(String),
    #[error("crypto error: {0}")]
    CryptoError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn full_protocol_roundtrip() {
        // Generate keys
        let (server_pk, server_sk) = kyber768::keypair();
        
        // 1. Client builds session
        let session = E2eeSession::new(&server_pk);
        assert_eq!(session.request_kem_ct.len(), ML_KEM_768_CIPHERTEXT_SIZE);
        
        // 2. Client encrypts request
        let request_json = r#"{"model":"test","messages":[{"role":"user","content":"hello"}],"max_tokens":5,"e2e_response_pk":"will-be-added"}"#;
        let client_blob = session.encrypt_request(request_json).unwrap();
        
        // 3. Server receives blob (simulated proxy)
        // Parse blob structure
        let server_ml_kem_ct = &client_blob[..ML_KEM_768_CIPHERTEXT_SIZE];
        let server_nonce = &client_blob[ML_KEM_768_CIPHERTEXT_SIZE..ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE];
        let server_encrypted = &client_blob[ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE..];
        
        // Server decapsulates
        use pqcrypto_traits::kem::Ciphertext as CiphertextTrait;
        let ct_obj = KyberCiphertext::from_bytes(server_ml_kem_ct).unwrap();
        let server_ss = kyber768::decapsulate(&ct_obj, &server_sk);
        
        // Server derives request key
        let salt = &server_ml_kem_ct[..HKDF_SALT_LEN];
        let hkdf = Hkdf::<Sha256>::new(Some(salt), server_ss.as_bytes());
        let mut server_req_key = vec![0u8; 32];
        hkdf.expand(HKDF_INFO_REQ, &mut server_req_key).unwrap();
        
        // Server decrypts
        let cipher = ChaCha20Poly1305::new_from_slice(&server_req_key).unwrap();
        let plaintext = cipher.decrypt(server_nonce.into(), server_encrypted).unwrap();
        let decompressed = decompress_gzip(&plaintext).unwrap();
        
        // Verify augment payload
        let payload: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        assert!(payload.get("e2e_response_pk").is_some());
        let response_pk_b64 = payload["e2e_response_pk"].as_str().unwrap();
        
        // 4. Server encrypts response
        let response_data = r#"{"choices":[{"message":{"content":"world"}}]}"#;
        let compressed = compress_gzip(response_data.as_bytes()).unwrap();
        
        // Server needs client's response PK that was in payload
        use base64::Engine;
        let client_response_pk_bytes = base64::engine::general_purpose::STANDARD.decode(response_pk_b64).unwrap();
        let client_response_pk = KyberPublicKey::from_bytes(&client_response_pk_bytes).unwrap();
        
        // Server ML-KEM encapsulates for response
        let (server_response_ss, response_ml_kem_ct) = kyber768::encapsulate(&client_response_pk);
        let mut response_nonce = [0u8; CHACHA_NONCE_SIZE];
        getrandom(&mut response_nonce).unwrap();
        
        // Server derives response key
        let salt = &response_ml_kem_ct.as_bytes()[..HKDF_SALT_LEN];
        let hkdf = Hkdf::<Sha256>::new(Some(salt), server_response_ss.as_bytes());
        let mut server_resp_key = vec![0u8; 32];
        hkdf.expand(HKDF_INFO_RESP, &mut server_resp_key).unwrap();
        
        // Server encrypts
        let resp_cipher = ChaCha20Poly1305::new_from_slice(&server_resp_key).unwrap();
        let resp_encrypted = resp_cipher.encrypt(&response_nonce.into(), compressed.as_ref()).unwrap();
        
        // Build response blob
        let mut response_blob = Vec::new();
        response_blob.extend_from_slice(response_ml_kem_ct.as_bytes());
        response_blob.extend_from_slice(&response_nonce);
        response_blob.extend_from_slice(&resp_encrypted);
        
        // 5. Client decrypts response
        let final_decrypted = session.decrypt_response(&response_blob).unwrap();
        let final_json: serde_json::Value = serde_json::from_slice(&final_decrypted).unwrap();
        
        assert_eq!(
            final_json["choices"][0]["message"]["content"].as_str(),
            Some("world")
        );
    }

    #[test]
    fn stream_roundtrip() {
        use base64::Engine;
        use pqcrypto_traits::kem::{Ciphertext as CiphertextTrait, PublicKey as PublicKeyTrait};

        let (server_pk, server_sk) = kyber768::keypair();
        let session = E2eeSession::new(&server_pk);

        let request_json = r#"{"model":"test","messages":[{"role":"user","content":"hello"}],"max_tokens":5}"#;
        let client_blob = session.encrypt_request(request_json).unwrap();

        let server_ml_kem_ct = &client_blob[..ML_KEM_768_CIPHERTEXT_SIZE];
        let ct_obj = KyberCiphertext::from_bytes(server_ml_kem_ct).unwrap();
        let server_ss = kyber768::decapsulate(&ct_obj, &server_sk);

        let salt = &server_ml_kem_ct[..HKDF_SALT_LEN];
        let hkdf = Hkdf::<Sha256>::new(Some(salt), server_ss.as_bytes());
        let mut server_req_key = vec![0u8; 32];
        hkdf.expand(HKDF_INFO_REQ, &mut server_req_key).unwrap();

        let server_nonce = &client_blob[ML_KEM_768_CIPHERTEXT_SIZE..ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE];
        let server_encrypted = &client_blob[ML_KEM_768_CIPHERTEXT_SIZE + CHACHA_NONCE_SIZE..];

        let cipher = ChaCha20Poly1305::new_from_slice(&server_req_key).unwrap();
        let plaintext = cipher.decrypt(server_nonce.into(), server_encrypted).unwrap();
        let decompressed = decompress_gzip(&plaintext).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&decompressed).unwrap();
        let response_pk_b64 = payload["e2e_response_pk"].as_str().unwrap();

        let client_response_pk_bytes = base64::engine::general_purpose::STANDARD.decode(response_pk_b64).unwrap();
        let client_response_pk = KyberPublicKey::from_bytes(&client_response_pk_bytes).unwrap();

        // Server: simulate stream init (encapsulate to client's response PK)
        let (stream_ss, stream_mlkem_ct) = kyber768::encapsulate(&client_response_pk);
        let stream_ct_bytes = stream_mlkem_ct.as_bytes().to_vec();
        let stream_salt = &stream_ct_bytes[..HKDF_SALT_LEN];
        let stream_hkdf = Hkdf::<Sha256>::new(Some(stream_salt), stream_ss.as_bytes());
        let mut stream_key_server = vec![0u8; 32];
        stream_hkdf.expand(HKDF_INFO_STREAM, &mut stream_key_server).unwrap();

        // Client: decrypt stream init
        let stream_ct_b64 = base64::engine::general_purpose::STANDARD.encode(&stream_ct_bytes);
        let stream_key_client = session.decrypt_stream_init(&stream_ct_b64).unwrap();
        assert_eq!(stream_key_server, stream_key_client);

        // Server: encrypt a stream chunk
        let sse_line = r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","model":"test","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let mut chunk_nonce = [0u8; CHACHA_NONCE_SIZE];
        getrandom(&mut chunk_nonce).unwrap();

        let chunk_cipher = ChaCha20Poly1305::new_from_slice(&stream_key_server).unwrap();
        let chunk_encrypted = chunk_cipher.encrypt(&chunk_nonce.into(), sse_line.as_bytes()).unwrap();

        let mut chunk_blob = Vec::with_capacity(CHACHA_NONCE_SIZE + chunk_encrypted.len());
        chunk_blob.extend_from_slice(&chunk_nonce);
        chunk_blob.extend_from_slice(&chunk_encrypted);
        let chunk_b64 = base64::engine::general_purpose::STANDARD.encode(&chunk_blob);

        // Client: decrypt stream chunk
        let decrypted = session.decrypt_stream_chunk(&chunk_b64, &stream_key_client).unwrap();
        let decrypted_str = String::from_utf8(decrypted).unwrap();
        assert!(decrypted_str.contains("Hello"));
        assert!(decrypted_str.starts_with("data:"));
    }
}
