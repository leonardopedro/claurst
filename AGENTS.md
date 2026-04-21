# AGENTS.md — Project Status for AI Agents

This file provides context for AI agents working on the Claurst codebase. Read this before making changes.

## Project Overview

Claurst is a multi-provider terminal coding agent written in Rust. The workspace is at `src-rust/` and contains these crates:

| Crate | Purpose |
|-------|---------|
| `claurst` | Binary entry point (CLI) |
| `claurst-core` | Shared types, config, provider IDs, error types |
| `claurst-api` | Provider adapters (LlmProvider trait), model registry, E2EE crypto |
| `claurst-query` | Query loop, agent tool, orchestration |
| `claurst-tools` | Tool definitions (file edit, bash, etc.) |
| `claurst-tui` | Terminal UI |
| `claurst-commands` | Slash commands |
| `claurst-bridge` | Bridge layer |
| `claurst-acp` | Agent communication protocol |
| `claurst-buddy` | Rustle companion |
| `claurst-mcp` | Model Context Protocol |
| `claurst-plugins` | Plugin system |
| `claurst-cli` | CLI argument parsing |

## Build & Check Commands

```bash
cd src-rust

# Full build
cargo build

# Check only (faster)
cargo check

# Check a specific crate
cargo check --package claurst-api

# Run tests
cargo test

# Build release binary
cargo build --release --package claurst
```

Always run `cargo check` after making changes to verify compilation.

## Chutes E2EE Provider — Current Status

### What's implemented

The Chutes E2EE provider is fully implemented and compiles cleanly. It provides native end-to-end encryption for Chutes TEE models without requiring an external proxy.

**Files:**

- `src-rust/crates/api/src/e2ee.rs` — Pure Rust E2EE crypto module
- `src-rust/crates/api/src/providers/chutes.rs` — ChutesProvider implementing LlmProvider trait

**e2ee.rs capabilities:**
- `E2eeSession::new(instance_pubkey)` — generates ephemeral ML-KEM-768 keypair, encapsulates to instance
- `encrypt_request(json_payload)` — augments payload with `e2e_response_pk`, gzip compresses, derives key via HKDF-SHA256(salt=ct[0:16], info="e2e-req-v1"), ChaCha20-Poly1305 encrypts, builds binary blob `[KEM_CT(1088) | NONCE(12) | CT+TAG(N)]`
- `decrypt_response(response_blob)` — decapsulates response ML-KEM CT with ephemeral secret key, derives key via HKDF(info="e2e-resp-v1"), decrypts, decompresses
- `decrypt_stream_init(mlkem_ct_b64)` — handles `{"e2e_init": "..."}` SSE event, derives stream key via HKDF(info="e2e-stream-v1")
- `decrypt_stream_chunk(enc_chunk_b64, stream_key)` — handles `{"e2e": "..."}` SSE events, decrypts nonce+ciphertext+tag
- `compress_gzip()` / `decompress_gzip()` — GZIP helpers
- `E2eeError` — error type with `From<E2eeError> for ProviderError` conversion
- `full_protocol_roundtrip` test — simulates full client-server encrypt/decrypt cycle

**chutes.rs capabilities:**
- Instance discovery: `GET https://api.chutes.ai/e2e/instances/{chute_id}` returns `{instances: [{instance_id, e2e_pubkey, nonces: [...]}], nonce_expires_in}`
- Model resolution: `GET https://llm.chutes.ai/v1/models` resolves model name → chute_id (cached)
- E2EE streaming: detects `e2e_init` → derives stream key → decrypts `e2e` chunks → re-parses decrypted SSE
- Handles `e2e_error` events from server
- `process_openai_chunk()` extracted as reusable function for both encrypted and unencrypted paths
- Registry integration: provider_from_key, provider_from_config, with_available_providers
- Config: `CHUTES_API_KEY` env var, model IDs like `chutes/zai-org/GLM-5.1-TEE`

**Other modified files:**
- `src-rust/Cargo.toml` — workspace crypto deps (pqcrypto-kyber, pqcrypto-traits, chacha20poly1305, flate2, getrandom, hkdf, base64, hex)
- `src-rust/crates/api/Cargo.toml` — crate-level crypto deps
- `src-rust/crates/core/src/provider_id.rs` — `CHUTES` constant
- `src-rust/crates/core/src/lib.rs` — env var resolution (CHUTES_API_KEY, CHUTES_API_BASE)
- `src-rust/crates/api/src/providers/mod.rs` — chutes module export
- `src-rust/crates/api/src/lib.rs` — e2ee module, ChutesProvider re-export
- `src-rust/crates/api/src/registry.rs` — provider registration
- `src-rust/crates/api/src/provider_error.rs` — `From<E2eeError> for ProviderError`
- `src-rust/crates/api/src/model_registry.rs` — Chutes model entries

### Protocol details (from e2ee-proxy source analysis)

The protocol was reverse-engineered from `../e2ee-proxy/` (OpenResty Lua + C native library):

1. **Discovery API** returns multiple instances with pre-generated nonces (single-use, ~55s expiry)
2. **X-E2E-Nonce header** sends the raw nonce string from discovery (hex format)
3. **Request encryption** uses HKDF-SHA256 with context "e2e-req-v1", salt = first 16 bytes of ML-KEM ciphertext
4. **Response blob** contains a NEW ML-KEM ciphertext (not the request one) — the server encapsulates to the client's `e2e_response_pk`
5. **Stream init** is `{"e2e_init": "base64-mlkem-ct"}` — a separate ML-KEM encapsulation for stream key
6. **Stream chunks** are `{"e2e": "base64-nonce+ciphertext+tag"}` — each chunk is independently encrypted
7. **Decrypted chunks** contain raw SSE text (may be multi-line) that needs re-parsing
8. **403 on nonce expiry** — the proxy retries with `invalidate_nonces()` and fresh discovery

### Live Testing Status

**Both streaming and non-streaming E2EE paths are verified against the live Chutes API.**

Live integration tests are in `src-rust/crates/api/tests/e2ee_live.rs` (marked `#[ignore]` so they don't run in CI):

```bash
cd src-rust
CHUTES_API_KEY="your-key" cargo test --package claurst-api --test e2ee_live -- --ignored --nocapture
```

Results (Apr 2026):
- `test_e2ee_streaming_simple` — **PASS**: Full E2EE streaming roundtrip with model `zai-org/GLM-5.1-TEE`. Discovery → encrypt → invoke → stream init → chunk decrypt → text output all working. Model produced reasoning + text, response = "Hello World!".
- `test_e2ee_non_streaming` — **PASS**: Full E2EE non-streaming (uses streaming internally, then accumulates). Response = "4" for "What is 2+2?".

### Known issues / TODO

- **Usage info returns zeros**: `UsageInfo { input_tokens: 0, output_tokens: 0 }` — the Chutes API response doesn't include usage data in streaming chunks, or it's not being parsed. Non-critical.

### Completed (previously TODO)

- **Nonce retry on 403**: Now retries up to 2 attempts with fresh discovery on nonce-related 403 errors (matching e2ee-proxy behavior). See `e2ee_invoke_attempt` + retry loop in `create_message_stream_e2ee`.
- **Model ID format**: `chutes/` prefix is now stripped from the model name in the E2EE request payload before encryption. The registry uses `chutes/zai-org/GLM-5.1-TEE` internally, but the API receives `zai-org/GLM-5.1-TEE`.
- **Unit test bugs fixed**: `KyberPublicKey::from_bytes` → `KyberCiphertext::from_bytes` for decapsulation, `Hkdf::new(salt, ...)` → `Hkdf::new(Some(salt), ...)`, added `use base64::Engine` import.
- **Stream roundtrip test**: Added `stream_roundtrip` test covering stream init (e2e_init) and chunk encryption/decryption.

### pqcrypto API notes

- `from_bytes()` and `as_bytes()` are trait methods on `pqcrypto_traits::kem::{PublicKey, Ciphertext, SharedSecret}`, NOT inherent methods. Always import the trait.
- `kyber768::keypair()` returns `(PublicKey, SecretKey)`.
- `kyber768::encapsulate(&pk)` returns `(SharedSecret, Ciphertext)`.
- `kyber768::decapsulate(&ct, &sk)` returns `SharedSecret`.
- `KyberSecretKey` does NOT implement `Debug` — custom Debug impl needed.
- `ChaCha20Poly1305::new_from_slice(&key)` requires the `KeyInit` trait.

## Code Style

- No comments unless explicitly requested
- Follow existing patterns in each crate
- Use `thiserror` for error types
- Use `async_trait` for async trait implementations
- Provider adapters follow the OpenAI provider pattern (see `openai.rs`)
- SSE parsing uses `async_stream::stream!` macro with `futures::StreamExt`

## Key Traits

- `LlmProvider` (`claurst-api/src/provider.rs`) — all providers implement this
- `SlashCommand` (`claurst-commands/`) — slash commands implement this
- Provider registration in `claurst-api/src/registry.rs`
- Model registry in `claurst-api/src/model_registry.rs`
