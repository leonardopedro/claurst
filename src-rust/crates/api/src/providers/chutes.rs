use std::pin::Pin;
use std::time::Duration;

use async_stream::stream;
use async_trait::async_trait;
use base64::Engine;
use claurst_core::provider_id::{ModelId, ProviderId};
use claurst_core::types::{ContentBlock, UsageInfo};
use futures::Stream;
use pqcrypto_traits::kem::PublicKey as PublicKeyTrait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::e2ee::{E2eeSession, ML_KEM_768_PUBKEY_SIZE};
use crate::error_handling::parse_error_response;
use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus,
    StreamEvent, SystemPromptStyle,
};
use super::openai::OpenAiProvider;

const CHUTES_API_BASE: &str = "https://api.chutes.ai";
const CHUTES_LLM_BASE: &str = "https://llm.chutes.ai/v1";
const CHUTES_E2E_INVOKE: &str = "https://api.chutes.ai/e2e/invoke";

#[derive(Debug, Deserialize)]
struct InstancesResponse {
    instances: Vec<InstanceInfo>,
    nonce_expires_in: u64,
}

#[derive(Debug, Deserialize, Clone)]
struct InstanceInfo {
    instance_id: String,
    e2e_pubkey: String,
    nonces: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelListResponse {
    data: Vec<ModelDetail>,
}

#[derive(Debug, Deserialize)]
struct ModelDetail {
    id: String,
    chute_id: String,
    confidential_compute: bool,
}

pub struct ChutesProvider {
    api_key: Option<String>,
    http_client: reqwest::Client,
    id: ProviderId,
    model_cache: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl ChutesProvider {
    pub fn new() -> Self {
        let api_key = std::env::var("CHUTES_API_KEY").ok().filter(|k| !k.is_empty());
        Self::with_config(api_key)
    }

    pub fn with_config(api_key: Option<String>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .expect("failed to build reqwest client");

        Self {
            api_key,
            http_client,
            id: ProviderId::new(ProviderId::CHUTES),
            model_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = if key.is_empty() { None } else { Some(key) };
        self
    }

    fn has_no_key(&self) -> bool {
        self.api_key.is_none()
    }

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            builder.header("Authorization", format!("Bearer {}", key))
        } else {
            builder
        }
    }

    async fn resolve_chute_id(&self, model: &str) -> Result<String, ProviderError> {
        {
            let cache = self.model_cache.lock().unwrap();
            if let Some(chute_id) = cache.get(model) {
                return Ok(chute_id.clone());
            }
        }

        let model_path = model.strip_prefix("chutes/").unwrap_or(model);

        let url = format!("{}/models", CHUTES_LLM_BASE);
        let builder = self.apply_auth(self.http_client.get(&url));

        debug!("Resolving model {} via {}", model, url);

        let resp = builder.send().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Model discovery failed: {}", e),
            status: None,
            body: None,
        })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Model lookup failed: {}", status),
                status: Some(status.as_u16()),
                body: None,
            });
        }

        let model_list: ModelListResponse = resp.json().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Model list parse failed: {}", e),
            status: Some(status.as_u16()),
            body: None,
        })?;

        let found = model_list.data.iter()
            .find(|m| m.id == model_path || m.id == model)
            .ok_or_else(|| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Model {} not found", model),
                status: Some(404),
                body: None,
            })?;

        if !found.confidential_compute {
            warn!("Model {} not marked confidential_compute", model);
        }

        {
            let mut cache = self.model_cache.lock().unwrap();
            cache.insert(model.to_string(), found.chute_id.clone());
        }

        Ok(found.chute_id.clone())
    }

    async fn discover_instances(&self, chute_id: &str) -> Result<InstancesResponse, ProviderError> {
        let url = format!("{}/e2e/instances/{}", CHUTES_API_BASE, chute_id);

        let builder = self.apply_auth(
            self.http_client.get(&url)
                .timeout(Duration::from_secs(30))
                .header("Cache-Control", "no-cache, no-store")
        );

        let resp = builder.send().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("E2EE instance discovery failed: {}", e),
            status: None,
            body: None,
        })?;

        let status = resp.status();
        if status == 404 {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: "Chute does not support E2EE".to_string(),
                status: Some(404),
                body: None,
            });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: format!("Discovery failed: {} - {}", status, body),
                status: Some(status.as_u16()),
                body: None,
            });
        }

        let info: InstancesResponse = resp.json().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Discovery parse failed: {}", e),
            status: Some(status.as_u16()),
            body: None,
        })?;

        debug!(
            chute_id = %chute_id,
            num_instances = info.instances.len(),
            nonce_expires = info.nonce_expires_in,
            "E2EE instances discovered"
        );

        Ok(info)
    }

    fn parse_public_key(pk_b64: &str) -> Result<pqcrypto_kyber::kyber768::PublicKey, ProviderError> {
        let pk_bytes = base64::engine::general_purpose::STANDARD.decode(pk_b64)
            .map_err(|e| ProviderError::Other {
                provider: ProviderId::new(ProviderId::CHUTES),
                message: format!("Invalid public key base64: {}", e),
                status: None,
                body: None,
            })?;

        if pk_bytes.len() != ML_KEM_768_PUBKEY_SIZE {
            return Err(ProviderError::Other {
                provider: ProviderId::new(ProviderId::CHUTES),
                message: format!("Invalid pubkey size: expected {}, got {}",
                    ML_KEM_768_PUBKEY_SIZE, pk_bytes.len()),
                status: None,
                body: None,
            });
        }

        PublicKeyTrait::from_bytes(&pk_bytes)
            .map_err(|e| ProviderError::Other {
                provider: ProviderId::new(ProviderId::CHUTES),
                message: format!("Invalid ML-KEM pubkey bytes: {:?}", e),
                status: None,
                body: None,
            })
    }

    fn build_messages(&self, request: &ProviderRequest) -> Vec<Value> {
        OpenAiProvider::to_openai_messages_pub(
            &request.messages,
            request.system_prompt.as_ref(),
        )
    }

    async fn e2ee_invoke_attempt(
        &self,
        request: &ProviderRequest,
        chute_id: &str,
    ) -> Result<(E2eeSession, reqwest::Response), ProviderError> {
        let instances = self.discover_instances(chute_id).await?;

        let instance = instances.instances.first()
            .ok_or_else(|| ProviderError::Other {
                provider: self.id.clone(),
                message: "No E2EE instances available".to_string(),
                status: None,
                body: None,
            })?;

        let nonce = instance.nonces.first()
            .ok_or_else(|| ProviderError::Other {
                provider: self.id.clone(),
                message: "No nonces available for instance".to_string(),
                status: None,
                body: None,
            })?;

        let instance_pubkey = Self::parse_public_key(&instance.e2e_pubkey)?;

        let model_name = request.model.strip_prefix("chutes/").unwrap_or(&request.model);

        let mut request_json = json!({
            "model": model_name,
            "max_tokens": request.max_tokens,
            "messages": self.build_messages(request),
            "stream": true,
        });

        if let Some(t) = request.temperature {
            request_json["temperature"] = json!(t);
        }
        if let Some(p) = request.top_p {
            request_json["top_p"] = json!(p);
        }
        if !request.stop_sequences.is_empty() {
            request_json["stop"] = json!(request.stop_sequences);
        }
        let tools = OpenAiProvider::to_openai_tools_pub(&request.tools);
        if !tools.is_empty() {
            request_json["tools"] = json!(tools);
        }

        let session = E2eeSession::new(&instance_pubkey);
        let encrypted_blob = session.encrypt_request(&request_json.to_string())?;

        debug!(
            instance_id = %instance.instance_id,
            nonce_prefix = &nonce[..nonce.len().min(12)],
            chute_id = %chute_id,
            "E2EE invoke attempt"
        );

        let req = self.http_client.post(CHUTES_E2E_INVOKE)
            .header("Content-Type", "application/octet-stream")
            .header("X-Chute-Id", chute_id)
            .header("X-Instance-Id", &instance.instance_id)
            .header("X-E2E-Nonce", nonce)
            .header("X-E2E-Stream", "true")
            .header("X-E2E-Path", "/v1/chat/completions")
            .body(encrypted_blob);
        let resp = self.apply_auth(req)
            .send()
            .await
            .map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("E2EE invoke failed: {}", e),
                status: None,
                body: None,
            })?;

        Ok((session, resp))
    }

    async fn create_message_stream_e2ee(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError> {
        let chute_id = self.resolve_chute_id(&request.model).await?;

        let mut last_err = None;
        for attempt in 1..=2 {
            let (session, resp) = match self.e2ee_invoke_attempt(&request, &chute_id).await {
                Ok(r) => r,
                Err(e) => return Err(e),
            };

            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                return Self::build_e2ee_stream(session, resp, self.id.clone());
            }

            let text = resp.text().await.unwrap_or_default();

            if status == 403 && attempt < 2 {
                let is_nonce_err = text.to_lowercase().contains("nonce");
                warn!(
                    attempt,
                    is_nonce_err,
                    body = &text[..text.len().min(200)],
                    "E2EE invoke 403, will retry"
                );
                if is_nonce_err {
                    last_err = Some(parse_error_response(status, &text, &self.id));
                    continue;
                }
            }

            return Err(parse_error_response(status, &text, &self.id));
        }

        Err(last_err.unwrap_or_else(|| ProviderError::Other {
            provider: self.id.clone(),
            message: "E2EE invoke failed after retries".to_string(),
            status: Some(403),
            body: None,
        }))
    }

    fn build_e2ee_stream(
        session: E2eeSession,
        resp: reqwest::Response,
        provider_id: ProviderId,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError> {
        let stream = stream! {
            use futures::StreamExt;

            let mut byte_stream = resp.bytes_stream();
            let mut leftover = String::new();
            let mut stream_key: Option<Vec<u8>> = None;
            let mut message_started = false;
            let mut message_id = String::from("unknown");
            let mut model_name = String::new();
            let mut tool_call_buffers: std::collections::HashMap<
                usize,
                (String, String, String),
            > = std::collections::HashMap::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("Stream read error: {}", e),
                            partial_response: None,
                        });
                        return;
                    }
                };

                let text = String::from_utf8_lossy(&chunk);
                let combined = if leftover.is_empty() {
                    text.to_string()
                } else {
                    let mut s = std::mem::take(&mut leftover);
                    s.push_str(&text);
                    s
                };

                let mut lines: Vec<&str> = combined.split('\n').collect();
                if !combined.ends_with('\n') {
                    leftover = lines.pop().unwrap_or("").to_string();
                }

                for line in lines {
                    let line = line.trim_end_matches('\r').trim();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    let Some(data) = line.strip_prefix("data:") else { continue; };
                    let data = data.trim();

                    if data == "[DONE]" {
                        if message_started {
                            yield Ok(StreamEvent::MessageStop);
                        }
                        return;
                    }

                    let chunk_json: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => {
                            debug!("Malformed SSE data, skipping: {}", &data[..data.len().min(100)]);
                            continue;
                        }
                    };

                    if let Some(e2e_init) = chunk_json.get("e2e_init").and_then(|v| v.as_str()) {
                        match session.decrypt_stream_init(e2e_init) {
                            Ok(key) => {
                                stream_key = Some(key);
                                debug!("E2EE stream initialized");
                                continue;
                            },
                            Err(e) => {
                                yield Err(ProviderError::StreamError {
                                    provider: provider_id.clone(),
                                    message: format!("E2EE init decrypt failed: {}", e),
                                    partial_response: None,
                                });
                                return;
                            }
                        }
                    }

                    if let Some(e2e_chunk) = chunk_json.get("e2e").and_then(|v| v.as_str()) {
                        if let Some(ref key) = stream_key {
                            match session.decrypt_stream_chunk(e2e_chunk, key) {
                                Ok(decrypted) => {
                                    let decrypted_str = String::from_utf8_lossy(&decrypted);
                                    for dec_line in decrypted_str.split('\n') {
                                        let dec_line = dec_line.trim_end_matches('\r').trim();
                                        if dec_line.is_empty() || dec_line.starts_with(':') {
                                            continue;
                                        }
                                        let Some(dec_data) = dec_line.strip_prefix("data:") else { continue; };
                                        let dec_data = dec_data.trim();
                                        if dec_data == "[DONE]" {
                                            if message_started {
                                                yield Ok(StreamEvent::MessageStop);
                                            }
                                            return;
                                        }
                                        let dec_json: Value = match serde_json::from_str(dec_data) {
                                            Ok(v) => v,
                                            Err(_) => continue,
                                        };
                                        if let Some(events) = process_openai_chunk(
                                            &dec_json,
                                            &mut message_started,
                                            &mut message_id,
                                            &mut model_name,
                                            &mut tool_call_buffers,
                                        ) {
                                            for evt in events {
                                                yield Ok(evt);
                                            }
                                        }
                                    }
                                },
                                Err(e) => {
                                    yield Err(ProviderError::StreamError {
                                        provider: provider_id.clone(),
                                        message: format!("E2EE chunk decrypt failed: {}", e),
                                        partial_response: None,
                                    });
                                    return;
                                }
                            }
                        } else {
                            warn!("Received e2e chunk before e2e_init");
                            continue;
                        }
                        continue;
                    }

                    if let Some(e2e_error) = chunk_json.get("e2e_error") {
                        let error_data = serde_json::to_string(&serde_json::json!({"error": e2e_error}))
                            .unwrap_or_default();
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("Server E2EE error: {}", error_data),
                            partial_response: None,
                        });
                        return;
                    }

                    if let Some(events) = process_openai_chunk(
                        &chunk_json,
                        &mut message_started,
                        &mut message_id,
                        &mut model_name,
                        &mut tool_call_buffers,
                    ) {
                        for evt in events {
                            yield Ok(evt);
                        }
                    }
                }
            }

            if message_started {
                yield Ok(StreamEvent::MessageStop);
            }
        };

        Ok(Box::pin(stream))
    }
}

fn process_openai_chunk(
    chunk_json: &Value,
    message_started: &mut bool,
    message_id: &mut String,
    model_name: &mut String,
    tool_call_buffers: &mut std::collections::HashMap<usize, (String, String, String)>,
) -> Option<Vec<StreamEvent>> {
    let mut events = Vec::new();

    if !*message_started {
        if let Some(id) = chunk_json.get("id").and_then(|v| v.as_str()) {
            *message_id = id.to_string();
        }
        if let Some(m) = chunk_json.get("model").and_then(|v| v.as_str()) {
            *model_name = m.to_string();
        }
        events.push(StreamEvent::MessageStart {
            id: message_id.clone(),
            model: model_name.clone(),
            usage: UsageInfo::default(),
        });
        events.push(StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Text { text: String::new() },
        });
        *message_started = true;
    }

    let choices = match chunk_json.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => {
            if let Some(usage_val) = chunk_json.get("usage") {
                let usage = OpenAiProvider::parse_usage_pub(Some(usage_val));
                events.push(StreamEvent::MessageDelta {
                    stop_reason: None,
                    usage: Some(usage),
                });
            }
            if events.is_empty() { return None; }
            return Some(events);
        }
    };

    let choice = match choices.first() {
        Some(c) => c,
        None => {
            if events.is_empty() { return None; }
            return Some(events);
        }
    };

    let delta = match choice.get("delta") {
        Some(d) => d,
        None => {
            if events.is_empty() { return None; }
            return Some(events);
        }
    };

    const REASONING_FIELDS: &[&str] = &["reasoning_content", "reasoning_text", "reasoning"];
    for field in REASONING_FIELDS {
        if let Some(reasoning) = delta.get(*field).and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                events.push(StreamEvent::ReasoningDelta {
                    index: 0,
                    reasoning: reasoning.to_string(),
                });
                break;
            }
        }
    }

    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            events.push(StreamEvent::TextDelta {
                index: 0,
                text: content.to_string(),
            });
        }
    }

    if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let tc_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if let Some(tc_id) = tc.get("id").and_then(|v| v.as_str()) {
                let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let block_index = 1 + tc_index;
                tool_call_buffers.insert(block_index, (tc_id.to_string(), name.clone(), String::new()));
                events.push(StreamEvent::ContentBlockStart {
                    index: block_index,
                    content_block: ContentBlock::ToolUse {
                        id: tc_id.to_string(),
                        name,
                        input: json!({}),
                    },
                });
            }
            if let Some(args_frag) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()) {
                if !args_frag.is_empty() {
                    let block_index = 1 + tc_index;
                    if let Some((_, _, buf)) = tool_call_buffers.get_mut(&block_index) {
                        buf.push_str(args_frag);
                    }
                    events.push(StreamEvent::InputJsonDelta {
                        index: block_index,
                        partial_json: args_frag.to_string(),
                    });
                }
            }
        }
    }

    if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
        if !finish_reason.is_empty() && finish_reason != "null" {
            events.push(StreamEvent::ContentBlockStop { index: 0 });
            let mut tc_indices: Vec<usize> = tool_call_buffers.keys().cloned().collect();
            tc_indices.sort();
            for idx in tc_indices {
                events.push(StreamEvent::ContentBlockStop { index: idx });
            }

            let stop_reason = OpenAiProvider::map_finish_reason_pub(finish_reason);
            let usage_val = chunk_json.get("usage");
            let usage = usage_val.map(|u| OpenAiProvider::parse_usage_pub(Some(u)));

            events.push(StreamEvent::MessageDelta {
                stop_reason: Some(stop_reason),
                usage,
            });
        }
    }

    if events.is_empty() { None } else { Some(events) }
}

#[async_trait]
impl LlmProvider for ChutesProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "Chutes E2EE"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let mut stream = self.create_message_stream(request).await?;
        let mut id = String::from("unknown");
        let mut model = String::new();
        let mut text_parts: Vec<(usize, String)> = Vec::new();
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut stop_reason = crate::provider_types::StopReason::EndTurn;
        let mut usage = UsageInfo::default();
        let mut tool_buffers: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new();

        use futures::StreamExt;
        while let Some(result) = stream.next().await {
            match result {
                Err(e) => return Err(e),
                Ok(evt) => match evt {
                    StreamEvent::MessageStart { id: msg_id, model: msg_model, usage: msg_usage } => {
                        id = msg_id;
                        model = msg_model;
                        usage = msg_usage;
                    }
                    StreamEvent::ContentBlockStart { index, content_block } => match content_block {
                        ContentBlock::Text { text } => {
                            text_parts.push((index, text));
                        }
                        ContentBlock::ToolUse { id: tool_id, name, input: _ } => {
                            tool_buffers.insert(index, (tool_id, name, String::new()));
                        }
                        other => {
                            content_blocks.push(other);
                        }
                    },
                    StreamEvent::TextDelta { index, text } => {
                        if let Some(entry) = text_parts.iter_mut().find(|(i, _)| *i == index) {
                            entry.1.push_str(&text);
                        }
                    }
                    StreamEvent::InputJsonDelta { index, partial_json } => {
                        if let Some((_, _, buf)) = tool_buffers.get_mut(&index) {
                            buf.push_str(&partial_json);
                        }
                    }
                    StreamEvent::ContentBlockStop { index } => {
                        if let Some((tool_id, name, json_buf)) = tool_buffers.remove(&index) {
                            let input = serde_json::from_str(&json_buf)
                                .unwrap_or(Value::Object(Default::default()));
                            content_blocks.push(ContentBlock::ToolUse {
                                id: tool_id,
                                name,
                                input,
                            });
                        }
                    },
                    StreamEvent::MessageDelta { stop_reason: sr, usage: delta_usage } => {
                        if let Some(r) = sr {
                            stop_reason = r;
                        }
                        if let Some(u) = delta_usage {
                            usage.output_tokens += u.output_tokens;
                        }
                    },
                    StreamEvent::MessageStop => break,
                    StreamEvent::Error { error_type, message } => {
                        return Err(ProviderError::StreamError {
                            provider: self.id.clone(),
                            message: format!("[{}] {}", error_type, message),
                            partial_response: None,
                        });
                    }
                    _ => {}
                },
            }
        }

        text_parts.sort_by_key(|(i, _)| *i);
        let mut all_blocks: Vec<(usize, ContentBlock)> = text_parts
            .into_iter()
            .map(|(i, text)| (i, ContentBlock::Text { text }))
            .collect();
        for block in content_blocks {
            all_blocks.push((usize::MAX, block));
        }
        let final_content: Vec<ContentBlock> = all_blocks.into_iter().map(|(_, b)| b).collect();

        Ok(ProviderResponse {
            id,
            content: final_content,
            stop_reason,
            usage,
            model,
        })
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError> {
        if self.has_no_key() {
            return Err(ProviderError::Other {
                provider: self.id.clone(),
                message: "CHUTES_API_KEY required for E2EE mode".to_string(),
                status: None,
                body: None,
            });
        }

        match self.create_message_stream_e2ee(request).await {
            Ok(stream) => Ok(stream),
            Err(e) => {
                info!("E2EE failed: {}", e);
                Err(e)
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        if self.api_key.is_none() {
            return Ok(vec![
                ModelInfo {
                    id: ModelId::new("chutes/zai-org/GLM-5.1-TEE"),
                    provider_id: self.id.clone(),
                    name: "GLM-5.1 TEE (Chutes E2EE)".to_string(),
                    context_window: 200_000,
                    max_output_tokens: 32_000,
                },
            ]);
        }

        match self.discover_chutes_models().await {
            Ok(models) => Ok(models),
            Err(_) => Ok(vec![]),
        }
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        if self.has_no_key() {
            return Ok(ProviderStatus::Unavailable {
                reason: "No CHUTES_API_KEY configured".to_string(),
            });
        }

        match self.discover_instance_any().await {
            Ok(_) => Ok(ProviderStatus::Healthy),
            Err(e) => Ok(ProviderStatus::Unavailable {
                reason: format!("{}", e),
            }),
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: true,
            image_input: true,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: true,
            system_prompt_style: SystemPromptStyle::SystemMessage,
        }
    }
}

impl ChutesProvider {
    async fn discover_chutes_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let url = format!("{}/models", CHUTES_LLM_BASE);
        let builder = self.apply_auth(self.http_client.get(&url));

        let resp = builder.send().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Model discovery failed: {}", e),
            status: None,
            body: None,
        })?;

        let status = resp.status();
        let model_list: ModelListResponse = resp.json().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Model list parse failed: {}", e),
            status: Some(status.as_u16()),
            body: None,
        })?;

        Ok(model_list.data
            .iter()
            .filter(|m| m.confidential_compute)
            .map(|m| ModelInfo {
                id: ModelId::new(&format!("chutes/{}", m.id)),
                provider_id: self.id.clone(),
                name: format!("{} (Chutes TEE)", m.id),
                context_window: 200_000,
                max_output_tokens: 32_000,
            })
            .collect())
    }

    async fn discover_instance_any(&self) -> Result<(), ProviderError> {
        let chute_id = self.resolve_chute_id("zai-org/GLM-5.1-TEE").await?;
        self.discover_instances(&chute_id).await?;
        Ok(())
    }
}
