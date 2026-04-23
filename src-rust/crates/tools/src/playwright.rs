use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct PlaywrightTool;



#[derive(Debug, Serialize, Deserialize)]
pub struct PlaywrightInput {
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub text: Option<String>,
    pub context_id: Option<String>,
}

#[async_trait]
impl Tool for PlaywrightTool {
    fn name(&self) -> &'static str {
        "playwright"
    }

    fn description(&self) -> &'static str {
        "Browser automation using Playwright for web testing, screenshots, and interaction. Use actions: launch, navigate, click, fill, text, screenshot, close. Requires: npx playwright@1.59.1 install"
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["launch", "navigate", "click", "fill", "text", "screenshot", "close"]},
                "url": {"type": "string"},
                "selector": {"type": "string"},
                "text": {"type": "string"},
                "context_id": {"type": "string"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
        let input: PlaywrightInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        match input.action.as_str() {
            "launch" => self.launch().await,
            "navigate" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                let url = match input.url {
                    Some(v) => v,
                    None => return ToolResult::error("url required".to_string()),
                };
                self.navigate(&ctx_id, &url).await
            }
            "click" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                let selector = match input.selector {
                    Some(v) => v,
                    None => return ToolResult::error("selector required".to_string()),
                };
                self.click(&ctx_id, &selector).await
            }
            "fill" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                let selector = match input.selector {
                    Some(v) => v,
                    None => return ToolResult::error("selector required".to_string()),
                };
                let text = match input.text {
                    Some(v) => v,
                    None => return ToolResult::error("text required".to_string()),
                };
                self.fill(&ctx_id, &selector, &text).await
            }
            "text" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                let selector = match input.selector {
                    Some(v) => v,
                    None => return ToolResult::error("selector required".to_string()),
                };
                self.text(&ctx_id, &selector).await
            }
            "screenshot" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                self.screenshot(&ctx_id).await
            }
            "close" => {
                let ctx_id = match input.context_id {
                    Some(v) => v,
                    None => return ToolResult::error("context_id required".to_string()),
                };
                self.close(&ctx_id).await
            }
            _ => ToolResult::error(format!("Unknown action: {}", input.action)),
        }
    }
}

impl PlaywrightTool {
    pub fn new() -> Self {
        Self {}
    }

    fn get_browser_map() -> &'static dashmap::DashMap<String, BrowserState> {
        use once_cell::sync::Lazy;
        static BROWSER_MAP: Lazy<dashmap::DashMap<String, BrowserState>> = 
            Lazy::new(|| dashmap::DashMap::new());
        &BROWSER_MAP
    }

    async fn launch(&self) -> ToolResult {
        use playwright_rs::Playwright;
        
        debug!("Launching Playwright");
        let play = match Playwright::launch().await {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Launch failed: {}", e)),
        };
        
        let browser = match play.chromium().launch().await {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Browser launch failed: {}", e)),
        };
        
        let context = match browser.new_context().await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Context failed: {}", e)),
        };
        
        let id = format!("br_{}", uuid::Uuid::new_v4());
        Self::get_browser_map().insert(id.clone(), BrowserState {
            _browser: Some(browser),
            context: Some(context),
            page: None,
        });
        
        ToolResult::success(format!(r#"{{"context_id":"{}"}}"#, id))
    }

    async fn navigate(&self, id: &str, url: &str) -> ToolResult {
        let map = Self::get_browser_map();
        let mut state = match map.get_mut(id) {
            Some(s) => s,
            None => return ToolResult::error("Context not found".to_string()),
        };
        
        let context = match state.context.as_ref() {
            Some(c) => c,
            None => return ToolResult::error("Context None".to_string()),
        };
        
        let page = match context.new_page().await {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Page failed: {}", e)),
        };
        
        if let Err(e) = page.goto(url, None).await {
            return ToolResult::error(format!("Navigate failed: {}", e));
        }
        
        state.page = Some(page);
        ToolResult::success(format!(r#"{{"navigated":"{}"}}"#, url))
    }

    async fn click(&self, id: &str, selector: &str) -> ToolResult {
        let map = Self::get_browser_map();
        let state = match map.get(id) {
            Some(s) => s,
            None => return ToolResult::error("Context not found".to_string()),
        };
        
        let page = match state.page.as_ref() {
            Some(p) => p,
            None => return ToolResult::error("No page".to_string()),
        };
        
        let locator = page.locator(selector).await;
        if let Err(e) = locator.click(None).await {
            return ToolResult::error(format!("Click failed: {}", e));
        }
        
        ToolResult::success(format!(r#"{{"clicked":"{}"}}"#, selector))
    }

    async fn fill(&self, id: &str, selector: &str, text: &str) -> ToolResult {
        let map = Self::get_browser_map();
        let state = match map.get(id) {
            Some(s) => s,
            None => return ToolResult::error("Context not found".to_string()),
        };
        
        let page = match state.page.as_ref() {
            Some(p) => p,
            None => return ToolResult::error("No page".to_string()),
        };
        
        let locator = page.locator(selector).await;
        if let Err(e) = locator.fill(text, None).await {
            return ToolResult::error(format!("Fill failed: {}", e));
        }
        
        ToolResult::success(format!(r#"{{"filled":"{}"}}"#, selector))
    }

    async fn text(&self, id: &str, selector: &str) -> ToolResult {
        let map = Self::get_browser_map();
        let state = match map.get(id) {
            Some(s) => s,
            None => return ToolResult::error("Context not found".to_string()),
        };
        
        let page = match state.page.as_ref() {
            Some(p) => p,
            None => return ToolResult::error("No page".to_string()),
        };
        
        let locator = page.locator(selector).await;
        let text_opt = match locator.text_content().await {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("Text failed: {}", e)),
        };
        
        let text = text_opt.unwrap_or_default();
        ToolResult::success(format!(r#"{{"text":"{}"}}"#, text))
    }

    async fn screenshot(&self, id: &str) -> ToolResult {
        let map = Self::get_browser_map();
        let state = match map.get(id) {
            Some(s) => s,
            None => return ToolResult::error("Context not found".to_string()),
        };
        
        let page = match state.page.as_ref() {
            Some(p) => p,
            None => return ToolResult::error("No page".to_string()),
        };
        
        let bytes = match page.screenshot(None).await {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Screenshot failed: {}", e)),
        };
        
        use base64::Engine;
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        ToolResult::success(format!(r#"{{"screenshot":"{}"}}"#, base64))
    }

    async fn close(&self, id: &str) -> ToolResult {
        Self::get_browser_map().remove(id);
        ToolResult::success(r#"{"closed":"true"}"#.to_string())
    }
}

struct BrowserState {
    _browser: Option<playwright_rs::Browser>,
    context: Option<playwright_rs::BrowserContext>,
    page: Option<playwright_rs::Page>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::config::Config;
    use std::sync::Arc;
    use claurst_core::permissions::AutoPermissionHandler;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    fn create_test_context() -> ToolContext {
        let handler = AutoPermissionHandler {
            mode: claurst_core::config::PermissionMode::Default,
        };
        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            permission_mode: claurst_core::config::PermissionMode::Default,
            permission_handler: Arc::new(handler),
            cost_tracker: claurst_core::cost::CostTracker::new(),
            session_id: "test_playwright".to_string(),
            file_history: Arc::new(parking_lot::Mutex::new(
                claurst_core::file_history::FileHistory::new(),
            )),
            current_turn: Arc::new(AtomicUsize::new(0)),
            non_interactive: true,
            mcp_manager: None,
            config: Config::default(),
            managed_agent_config: None,
            completion_notifier: None,
            password_store: None,
        }
    }

    #[tokio::test]
    async fn test_playwright_tool_name() {
        let tool = PlaywrightTool::new();
        assert_eq!(tool.name(), "playwright");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[tokio::test]
    async fn test_playwright_launch_and_close() {
        let tool = PlaywrightTool::new();
        let ctx = create_test_context();
        
        let input = serde_json::json!({"action": "launch"});
        let result = tool.execute(input, &ctx).await;
        
        assert!(!result.is_error, "Launch should succeed: {}", result.content);
        
        let json_val: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let context_id = json_val["context_id"].as_str().unwrap();
        
        // Close
        let close_input = serde_json::json!({"action": "close", "context_id": context_id});
        let close_result = tool.execute(close_input, &ctx).await;
        assert!(!close_result.is_error);
    }

    #[tokio::test]
    async fn test_playwright_full_flow() {
        let tool = PlaywrightTool::new();
        let ctx = create_test_context();
        
        // Launch
        let launch_json = tool.execute(serde_json::json!({"action": "launch"}), &ctx).await;
        assert!(!launch_json.is_error);
        
        let context_id = serde_json::from_str::<serde_json::Value>(&launch_json.content)
            .unwrap()["context_id"].as_str().unwrap().to_string();
        
        // Navigate to example.com
        let nav_json = tool.execute(
            serde_json::json!({"action": "navigate", "context_id": &context_id, "url": "https://example.com"}),
            &ctx
        ).await;
        assert!(!nav_json.is_error, "Navigate failed: {}", nav_json.content);
        
        // Get h1 text
        let text_json = tool.execute(
            serde_json::json!({"action": "text", "context_id": &context_id, "selector": "h1"}),
            &ctx
        ).await;
        assert!(!text_json.is_error, "Text failed: {}", text_json.content);
        
        // Verify we got something
        let text_val: serde_json::Value = serde_json::from_str(&text_json.content).unwrap();
        let text = text_val["text"].as_str().unwrap();
        assert!(!text.is_empty(), "Should have text content");
        
        // Take screenshot
        let shot_json = tool.execute(
            serde_json::json!({"action": "screenshot", "context_id": &context_id}),
            &ctx
        ).await;
        assert!(!shot_json.is_error, "Screenshot failed: {}", shot_json.content);
        
        // Verify screenshot is base64
        let shot_val: serde_json::Value = serde_json::from_str(&shot_json.content).unwrap();
        let base64 = shot_val["screenshot"].as_str().unwrap();
        assert!(!base64.is_empty(), "Screenshot should have base64 data");
        
        // Close
        tool.execute(serde_json::json!({"action": "close", "context_id": &context_id}), &ctx).await;
    }
}
