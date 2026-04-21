use claurst_api::ChutesProvider;
use claurst_api::provider::LlmProvider;
use claurst_api::provider_types::{ProviderRequest, StreamEvent};
use claurst_core::types::Message;
use futures::StreamExt;

fn get_api_key() -> Option<String> {
    std::env::var("CHUTES_API_KEY").ok().filter(|k| !k.is_empty())
}

#[tokio::test]
#[ignore]
async fn test_e2ee_streaming_simple() {
    let key = get_api_key().expect("CHUTES_API_KEY must be set");
    let provider = ChutesProvider::with_api_key(ChutesProvider::new(), key);

    let request = ProviderRequest {
        model: "chutes/zai-org/GLM-5.1-TEE".to_string(),
        messages: vec![Message::user("Say exactly: Hello World!")],
        system_prompt: None,
        tools: vec![],
        max_tokens: 512,
        temperature: Some(0.0),
        top_p: None,
        top_k: None,
        stop_sequences: vec![],
        thinking: None,
        provider_options: serde_json::Value::Object(Default::default()),
    };

    eprintln!("Creating E2EE stream...");
    let stream_result = provider.create_message_stream(request).await;

    match &stream_result {
        Ok(_) => eprintln!("Stream created successfully!"),
        Err(e) => eprintln!("Stream creation failed: {}", e),
    }
    assert!(stream_result.is_ok(), "Failed to create E2EE stream");

    let mut stream = stream_result.unwrap();
    let mut full_text = String::new();
    let mut event_count = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                event_count += 1;
                match &event {
                    StreamEvent::MessageStart { id, model, .. } => {
                        eprintln!("[MessageStart] id={}, model={}", id, model);
                    }
                    StreamEvent::TextDelta { text, .. } => {
                        eprint!("{}", text);
                        full_text.push_str(text);
                    }
                    StreamEvent::ReasoningDelta { reasoning, .. } => {
                        eprintln!("[Reasoning] {}...", &reasoning[..reasoning.len().min(80)]);
                    }
                    StreamEvent::ContentBlockStart { index, .. } => {
                        eprintln!("\n[ContentBlockStart] index={}", index);
                    }
                    StreamEvent::ContentBlockStop { index } => {
                        eprintln!("\n[ContentBlockStop] index={}", index);
                    }
                    StreamEvent::MessageDelta { stop_reason, .. } => {
                        eprintln!("[MessageDelta] stop_reason={:?}", stop_reason);
                    }
                    StreamEvent::MessageStop => {
                        eprintln!("[MessageStop]");
                    }
                    StreamEvent::Error { error_type, message } => {
                        eprintln!("[Error] {}: {}", error_type, message);
                    }
                    _ => {
                        eprintln!("[Other] {:?}", event);
                    }
                }
            }
            Err(e) => {
                eprintln!("\nStream error: {}", e);
                panic!("Stream error: {}", e);
            }
        }
    }

    eprintln!("\n\nFull response text: {}", full_text);
    eprintln!("Total events: {}", event_count);
    assert!(!full_text.is_empty(), "Response text should not be empty");
}

#[tokio::test]
#[ignore]
async fn test_e2ee_non_streaming() {
    let key = get_api_key().expect("CHUTES_API_KEY must be set");
    let provider = ChutesProvider::with_api_key(ChutesProvider::new(), key);

    let request = ProviderRequest {
        model: "chutes/zai-org/GLM-5.1-TEE".to_string(),
        messages: vec![Message::user("What is 2+2? Answer with just the number.")],
        system_prompt: None,
        tools: vec![],
        max_tokens: 512,
        temperature: Some(0.0),
        top_p: None,
        top_k: None,
        stop_sequences: vec![],
        thinking: None,
        provider_options: serde_json::Value::Object(Default::default()),
    };

    eprintln!("Creating E2EE non-streaming request...");
    let result = provider.create_message(request).await;

    match &result {
        Ok(response) => {
            eprintln!("Response id: {}", response.id);
            eprintln!("Response model: {}", response.model);
            eprintln!("Response stop_reason: {:?}", response.stop_reason);
            eprintln!("Response usage: {:?}", response.usage);
            for (i, block) in response.content.iter().enumerate() {
                eprintln!("Block {}: {:?}", i, block);
            }
        }
        Err(e) => {
            eprintln!("Request failed: {}", e);
        }
    }
    assert!(result.is_ok(), "E2EE non-streaming request failed");
}
