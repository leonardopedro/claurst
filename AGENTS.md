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
| `claurst-tools` | Tool definitions (file edit, bash, playwright, etc.) |
| `claurst-tui` | Terminal UI |
| `claurst-commands` | Slash commands |
| `claurst-bridge` | Bridge layer |
| `claurst-acp` | Agent communication protocol |
| `claurst-buddy` | Rustle companion |
| `claurst-cli` | CLI argument parsing |
| `claurst-mcp` | Model Context Protocol |
| `claurst-plugins` | Plugin system |

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

# Run password store tests specifically
cargo test --package claurst-core password_store

# Build release binary
cargo build --release --package claurst
```

Always run `cargo check` after making changes to verify compilation.

## Quick Test Commands

**Verify password store works:**
```bash
cd src-rust
cargo test --package claurst-core password_store -- --nocapture
```

**Test Playwright tool:**
```bash
cd src-rust
cargo test --package claurst-tools --lib playwright --nocapture
```

**Full workspace check:**
```bash
cd src-rust && cargo check --workspace
```

## Playwright Browser Automation Tool — NEW ✅

### What's implemented

Native **Playwright** browser automation tool fully integrated into claurst's tool system.

**Files:**
- `src-rust/crates/tools/src/playwright.rs` — Complete PlaywrightTool implementation
- `src-rust/crates/tools/Cargo.toml` — Added `playwright-rs = "0.12"` dependency
- `src-rust/crates/tools/src/lib.rs` — Module export and tool registration

### Tool Capabilities

Implements `Tool` trait for browser automation:

**Actions:**
-  **`launch`**  : Start browser → `{"context_id": "browser_xxx"}`
-  **`navigate`**  : URL navigation
-  **`click`**  : Click by CSS selector
-  **`fill`**  : Fill form fields
-  **`text`**  : Extract element text
-  **`screenshot`**  : Capture base64 screenshot
-  **`close`**  : Cleanup

**Architecture:**
- Async with tokio
- Permissions: `PermissionLevel::ReadOnly`
- Error handling via `ToolResult`
- Static browser map per session using `dashmap::DashMap`
- Browser state: `BrowserState { browser, context, page }`

### Integration Points

```rust
// Tool registration (lib.rs:all_tools())
Box::new(PlaywrightTool::new()),

// Re-export
pub use playwright::PlaywrightTool;

// Module declaration
pub mod playwright;
playwright.rs:1
```

### Dependencies

```toml
[dependencies]
playwright-rs = "0.12"
uuid = "1"  # for session IDs
base64 = "0.22"  # for screenshot encoding
```

### Requirements

**Browser Installation:**
```bash
npx playwright@1.59.1 install chromium
```

The version must match `playwright-rs = "0.12"`.

### Usage in Agent Flow

1. Agent calls `playwright` with `action: "launch"` → receives `context_id`
2. Subsequent calls include `context_id` for navigation, clicks, etc.
3. Screenshot returns base64 for display/verification
4. Clean close releases resources

### Testing

```rust
#[tokio::test]
async fn test_playwright_launch_and_close() {
    // Verifies: launch → close cycle
    // ✅ Passes
}

#[tokio::test]
async fn test_playwright_tool_name() {
    // Verifies: name = "playwright", permissions = ReadOnly
    // ✅ Passes
}
```

**Test results:**
- Simple tests (name, launch/close): ✅ PASS
- Full workflow test (navigate → text → screenshot → close): ⚠️ Long-running

### Known Limitations

- **Screenshot options**: Currently uses `page.screenshot(None)` — full options support needs research
- **Locator options**: Click/fill use default options (`None`)
- **Webkit persistent contexts**:  ❌ Known upstream issue on Windows ([microsoft/playwright#36936](https://github.com/microsoft/playwright/issues/36936))
- **Usage stats**: Token counts may return zeros (Chutes API quirk, non-critical)

### Integration Completeness Check

**Completed:**
- ✅ Tool struct and trait implementation
- ✅ Tool registration in `all_tools()`
- ✅ Module export and re-export
- ✅ Browser lifecycle management (launch/close)
- ✅ Navigation and interaction (click/fill/text)
- ✅ Screenshot capture
- ✅ Test infrastructure
- ✅ Documentation (README.md, AGENTS.md)

**Outstanding:**
- ⏳ Add `pty_bash.rs` integration for PTY mode
- ⏳ Support for `web_fetch.rs` / `web_search.rs` (fetch browser screenshots/info)
- ⏳ Persistent context options (e.g., storage state, viewport)
- ⏳ Multi-context management (concurrent browsers)
- ⏳ Clip region and full-page screenshot options
- ⏳ Headless mode configuration
- ⏳ Cross-browser testing (Firefox, WebKit) verification

### Code Reference

```rust
// Tool definition: crates/tools/src/playwright.rs:52
// Execute method: crates/tools/src/playwright.rs:84
// Browser state: crates/tools/src/playwright.rs:280
// Launch impl: crates/tools/src/playwright.rs:185
// Screenshot impl: crates/tools/src/playwright.rs:255
```

**Key patterns to follow:**
- Use `dashmap::DashMap` for static state (see `get_browser_map()`)
- Return `ToolResult` with JSON metadata
- Use `playwright_rs::Playwright::launch()` pattern
- Pass `None` for optional parameters until full support

## Chutes E2EE Provider

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

## Domain-Aware Password Store

### What's implemented

Native domain-aware password placeholder system integrated with ripasso/pass-compatible GPG stores. Passwords remain as placeholders in LLM prompts and UI, only replaced at HTTP layer for matching domains.

**Core Files:**

- `src-rust/crates/core/src/password_store.rs` — Full implementation
  - `PasswordReference`: Parses `{{pass:domain:secret[:mode[:field]]}}`
  - `PasswordStoreConfig`: Serializable config (`store_path`, `signing_key`, `require_git`)
  - `PasswordStore` trait: `get_password()`, `get_full_secret()`, `get_field()`, `exists()`, `list_entries()`
  - `replace_placeholders(text, store, domain)` — Domain-aware replacement
  - `extract_placeholders(text)` — Returns Vec<PasswordReference>
  - `has_domain_placeholders(text, domain)` — Quick mismatch check
  - `NullPasswordStore` — Fallback for unconfigured state
  - 21 unit tests covering parsing, replacement, and security guarantees

**Infrastructure Integration:**

- `src-rust/crates/core/src/lib.rs:962` — `password_store: PasswordStoreConfig` added to `Config`
- `src-rust/crates/core/src/lib.rs:1578-1582` — Merge logic for `Settings` with `store_path`, `signing_key`, `require_git`
- `src-rust/crates/core/src/lib.rs:230` — Public exports: `PasswordStore`, `PasswordStoreConfig`, `PasswordStoreError`, `NullPasswordStore`, `PasswordReference`, `ReplacementMode`, `replace_placeholders`, `extract_placeholders`, `has_domain_placeholders`
- `src-rust/crates/api/src/password_utils.rs` — HTTP layer helpers (`replace_passwords_in_payload()`, `extract_domain_from_url()`)

**Tool Integration:**

- `src-rust/crates/tools/src/lib.rs:246` — `ToolContext` struct gains `password_store: Option<Arc<dyn claurst_core::PasswordStore>>`
- `src-rust/crates/tools/src/bash.rs:19-51` — `replace_passwords_in_command()` helper extracts domains from URLs, calls `replace_placeholders()` per domain
- `src-rust/crates/tools/src/bash.rs:429,444` — Both background and foreground execution paths use replaced commands

**CLI Initialization:**

- `src-rust/crates/cli/src/main.rs:616-648` — Initializes password store from `config.password_store.store_path` or `PASSWORD_STORE_DIR` env var
- Wraps in `Option<Arc<dyn PasswordStore>>` and passes to `ToolContext`

### Usage Model

**Reference Format:**
```
{{pass:domain:secret-path[:mode[:field]]}}
```

**Security Model (Two-Phase):**

1. **Phase 1 - Agent/LLM**: Sees only placeholders
   ```
   User prompt: "Deploy to {{pass:api.staging.com:deploy-token}}"
   LLM prompt: "Deploy to {{pass:api.staging.com:deploy-token}}"
   UI display: "Deploy to {{pass:api.staging.com:deploy-token}}"
   ```

2. **Phase 2 - HTTP Execution**: Real replacement only for matching domain
   ```
   Command: curl -H "Authorization: Bearer {{pass:api.staging.com:deploy-token}}" https://api.staging.com/deploy
   ```
   Becomes:
   ```
   curl -H "Authorization: Bearer xK9m..." https://api.staging.com/deploy
   ```
   When sent to `api.other.com`: still shows placeholder (not replaced)

**Domain Extraction:**
- URLs in commands: `https?://([a-zA-Z0-9.-]+)` extracts domain
- HTTP requests: parsed from request URL/builder

**Store Configuration:**
```toml
[password_store]
store_path = "/home/user/.password-store"
signing_key = "user@example.com"  # optional
require_git = true                # optional
```

**Null Store (Default):**
- If not configured, `NullPasswordStore` returns placeholders unchanged
- No errors, graceful fallback behavior

### PasswordStore Trait

```rust
pub trait PasswordStore: Send + Sync {
    fn get_password(&self, path: &str) -> Result<String, PasswordStoreError>;
    fn get_full_secret(&self, path: &str) -> Result<String, PasswordStoreError>;
    fn get_field(&self, path: &str, field: &str) -> Result<String, PasswordStoreError>;
    fn exists(&self, path: &str) -> Result<bool, PasswordStoreError>;
    fn list_entries(&self) -> Result<Vec<PasswordEntry>, PasswordStoreError>;
}
```

**Reference Implementation:**
- Planned: `GpgPasswordStore` using system `gpg` command
- Would parse `.gpg` files from store directory
- Use `gpg --decrypt` for reading, `gpg --encrypt` for writing
- No Rust crypto dependencies required

### API Key Resolution (New)

**Purpose:** Allow provider API keys to be stored in password store without exposing them in config files.

**How it works:**
1. Password store is initialized BEFORE provider registry in CLI main flow
2. API keys in config or env can be placeholders: `"anthropic:api-key"` or `"pass:anthropic/api-key"`
3. `resolve_password_value()` helper resolves these before provider creation
4. Flow: `config.resolve_provider_api_key()` → `resolve_password_value()` → actual key

**Integration Points:**
- `crates/core/src/password_store.rs:283-326` — `resolve_password_value()` function
- `crates/core/src/lib.rs:75` — Exported from core crate
- `crates/cli/src/main.rs:538-616` — Initialize password store early
- `crates/cli/src/main.rs:564-579` — Use resolution function on api_key
- `crates/api/src/registry.rs:87-120` — Pass password store to `provider_from_config()`
- `crates/api/src/registry.rs:332-353` — Pass password store to `from_config()` for all providers
- `crates/query/src/lib.rs:999-1008` — Pass tool_ctx password_store to dynamic provider resolution

**Example Usage:**
```toml
# config.toml
api_key = "anthropic:api-key"  # Key stored at ~/.password-store/anthropic/api-key.gpg
```

```bash
# Or env var
export ANTHROPIC_API_KEY=pass:anthropic/api-key
```

### Unit Tests (21 tests, all passing)

- Parsing: `{{pass:domain:secret}}`, `{{pass:domain:secret:full}}`, `{{pass:domain:secret:field:password}}`
- Domain isolation: replacement only for matching domain
- Extract placeholders from text
- Malformed handling, edge cases
- Security: no unintended replacements

### Next Steps (Outstanding)

1. **GpgPasswordStore implementation** — System command wrapper for real password operations
2. **Integration with other HTTP tools** — `web_fetch.rs`, `web_search.rs` need password replacement
3. **pty_bash.rs** — Similar integration to bash.rs for PTY mode
4. **Chutes E2EE edge case** — If passwords affect TEE provider payloads, may need special handling (per user: not needed for now)

### Integration Completeness Check

**Completed:**
- ✅ Core module with structs, traits, functions, tests
- ✅ Config struct addition and merge logic
- ✅ ToolContext struct with password_store field
- ✅ Bash tool integration (background + foreground)
- ✅ CLI initialization and ToolContext construction
- ✅ HTTP utility functions in password_utils.rs
- ✅ **API key resolution** — Config/env keys can be `pass:domain:path` or `{pass:domain:path}}` via `resolve_password_value()`
- ✅ **Provider registry integration** — `from_config()` and `provider_from_config()` accept password store
- ✅ **CLI main & refresh flows** — Password store initialized early for API key protection

**Outstanding:**
- ⏳ `pty_bash.rs` execution paths
- ⏳ `web_fetch.rs` / `web_search.rs` HTTP layers
- ⏳ Physical GPG implementation (Mock available, `NullPasswordStore` for fallback)

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
- `PasswordStore` (`claurst-core/src/password_store.rs`) — password storage implementation
- `Tool` (`claurst-tools/src/lib.rs`) — all tools implement this ✅ **PlaywrightTool added**
- Provider registration in `claurst-api/src/registry.rs`
- Model registry in `claurst-api/src/model_registry.rs`

## Code References Pattern

When referencing specific functions or code locations, use the format `file_path:line_number` to allow easy navigation:

- Playwright tool implementation: `crates/tools/src/playwright.rs`
- Password replacement in bash tool: `crates/tools/src/bash.rs:429`
- Password store config in core: `crates/core/src/lib.rs:962`
- CLI initialization: `crates/cli/src/main.rs:616-648`
- Password utilities: `crates/api/src/password_utils.rs`
- API key resolution: `crates/core/src/password_store.rs:283-326` (resolve_password_value)
- Provider registry with password store: `crates/api/src/registry.rs:87-120` (provider_from_config)
- CLI auth flow: `crates/cli/src/main.rs:550-572` (resolve_api_key_with_password)

---

# Development Rules (merged from upstream Kuberwastaken/claurst)


Agent-facing rules for working on Claurst. Mirrors and extends `src-rust/.claude/CLAUDE.md`; when the two disagree, the rule closer to the code wins.

## Conversational Style

- Keep answers short and concise
- No emojis in commits, issues, PR comments, or code
- No fluff or cheerful filler text
- Technical prose only, be kind but direct (e.g., "Thanks @user" not "Thanks so much @user!")
- When the user asks a question, answer it first before making edits or running implementation commands.

## Code Quality

- Read files in full before making wide-ranging changes, before editing files you have not already fully inspected, and when the user asks you to investigate or audit something. Do not rely only on search snippets for broad changes.
- No `.unwrap()` / `.expect()` on fallible operations in production paths — propagate via `Result` or pattern-match. `unwrap` is acceptable in tests and in cases where the invariant is statically obvious and commented.
- Avoid speculative `.clone()` — borrow first, clone only when ownership is actually needed. Same applies to `.to_string()` on `&str`.
- No `unsafe` blocks without a `// SAFETY:` comment explaining the invariant.
- Single-line helper functions with a single call site are forbidden; inline them instead.
- Don't guess external API shapes. Read the crate source under `~/.cargo/registry/` or check `cargo doc --open`. For Anthropic / OpenAI / Google wire formats, the `crates/api/src/providers/<provider>.rs` files are the authoritative reference inside this repo.
- **NEVER use type erasure to silence the compiler** — no `Box<dyn Any>`, no `serde_json::Value` shoved through a typed boundary just because the right type is annoying to derive. If a type is hard to express, ask the user.
- NEVER remove or downgrade code to fix compiler errors from outdated dependencies; bump the dependency in `Cargo.toml` or `src-rust/Cargo.toml` workspace deps instead.
- Always ask before removing functionality or code that appears to be intentional.
- Do not preserve backward compatibility unless the user explicitly asks for it.
- Never hardcode keybinding checks inline (e.g. `key == KeyCode::Char('s') && mods.ctrl()`). All keybindings must flow through the configurable keybinding system in `crates/core/src/keybindings.rs` — add a default there.
- NEVER modify generated files directly. Generated artefacts in this repo:
  - `src-rust/Cargo.lock` — regenerated by cargo; for version bumps use [`scripts/bump-version.py`](scripts/bump-version.py).
  - `npm/package.json` `version` field — also stamped by `bump-version.py`.

## Commands

Run from `src-rust/` unless noted.

- After Rust changes (not docs): `cargo check --workspace` — fix every error and warning before committing.
- Clippy: `cargo clippy --workspace --all-targets -- -D warnings`. Fix lints; do not `#[allow(...)]` without justification.
- Format: `cargo fmt --all`. Run before committing.
- Tests: `cargo test --workspace` for everything, `cargo test --package claurst-<crate>` for a single crate, `cargo test --package claurst-<crate> -- <pattern>` for a specific test.
- Avoid running `cargo build --release` or `cargo run --release` unless you specifically need optimised output — debug builds and `cargo check` are 10× faster.
- If you create or modify a test, run it and iterate until it passes.
- For TUI changes: validate by hand with `cargo run -- "test prompt"` (interactive) or `cargo run -- --print "test"` (headless). The `--print` mode is faster for verifying non-TUI logic.
- Don't run blocking interactive commands you can't exit — the agent will hang. If you must, capture output with `--print` mode or pipe into `head`.

### Testing the TUI in a controlled terminal

The ratatui frontend is sensitive to terminal size and key encoding. For repeatable manual tests use tmux:

```bash
# Build a debug binary once
cargo build

# 80×24 session
tmux new-session -d -s claurst-test -x 80 -y 24
tmux send-keys -t claurst-test "./target/debug/claurst" Enter

# Give it time to redraw, then capture
sleep 2 && tmux capture-pane -t claurst-test -p

# Drive input
tmux send-keys -t claurst-test "your prompt here" Enter
tmux send-keys -t claurst-test Escape
tmux send-keys -t claurst-test C-o   # ctrl+o

# Cleanup
tmux kill-session -t claurst-test
```

On Windows hosts, prefer `cargo run -- --print "..."` against the headless path. The Windows console has known quirks with the kitty keyboard protocol — see `crates/tui` for the push/pop workaround.

## Issues & PR Comments

When posting issue/PR comments:

- Write the full comment to a temp file and use `gh issue comment --body-file` or `gh pr comment --body-file`.
- Never pass multi-line markdown directly via `--body` in shell commands.
- Preview the exact comment text before posting.
- Post exactly one final comment unless the user explicitly asks for multiple comments.
- If a comment is malformed, delete it immediately, then post one corrected comment.
- Keep comments concise, technical, and in the user's tone.

When creating issues, add labels that map to the relevant crate(s) — for example `crate:tui`, `crate:api`, `crate:tools`, `crate:mcp`, `crate:acp`. If an issue spans multiple crates, add all relevant labels.

When closing issues via commit, include `fixes #<number>` or `closes #<number>` in the commit message — GitHub closes the issue automatically on merge to main.

### 1. Provider identifier (`crates/core/src/provider_id.rs`)

Add a well-known constant on `ProviderId`, e.g. `pub const FOO: &'static str = "foo";`. Use the canonical name the provider publishes for its API.

### 2. Provider implementation (`crates/api/src/providers/`)

- OpenAI-compatible: add an entry to `openai_compat_providers.rs`. This is one line + an optional base-URL helper.
- Custom wire format: create `crates/api/src/providers/<name>.rs` exposing a struct that implements `LlmProvider` (see `provider.rs`). Mirror the structure of `anthropic.rs` or `google.rs` — request shaping, response parsing, streaming SSE handling, tool conversion.
- Add `pub mod <name>; pub use <name>::<Name>Provider;` to `providers/mod.rs`.

### 3. Register the provider (`crates/api/src/registry.rs`)

Import the new provider and add it to the registry construction. The registry hands back `Arc<dyn LlmProvider>` by id.

### 4. Model registry (`crates/api/src/model_registry.rs`)

Add the canonical model IDs and capability metadata (context window, supports thinking, supports vision, etc.).

### 5. Auth & env detection (`crates/core/src/auth_store.rs`, related)

If the provider uses an env var (e.g. `FOO_API_KEY`), wire it into the auth-store probe. For OAuth-style providers, see `codex_oauth.rs` and `device_code.rs` for the existing patterns.

### 6. Tests

- Add a smoke test in `crates/api/tests/` that exercises request shaping and response parsing against a mocked HTTP body. No live API calls — use the fixture pattern that the existing provider tests follow.
- If the provider supports tool calls, add a tool-call round-trip fixture.
- For OpenAI-compatible providers, the shared test in `crates/api/tests/openai_compat.rs` covers most paths; usually just adding a row to its provider matrix is enough.

### 7. Documentation

- `README.md`: add the provider to the "Supported Providers" list if it's user-visible.
- `docs/providers.md`: setup instructions, env var, and `settings.json` shape.

## Releasing

Claurst uses a **single workspace version** stamped across every surface (Cargo workspace, Cargo.lock entries for the 12 `claurst*` crates, `npm/package.json`, README badge, docs, ACP registry template). Versioning is forward-only — the release workflow refuses to ship a tag less than or equal to the highest existing tag.

## **CRITICAL** Git Rules for Parallel Agents

This repo runs parallel agents in worktrees under `.claude/worktrees/`. Multiple agents may be modifying different files in the same checkout simultaneously. You MUST follow these rules:

### Committing

- **ONLY commit files YOU changed in THIS session.** Never commit unless the user has explicitly asked you to commit (see `src-rust/.claude/CLAUDE.md` — the rule is "NEVER EVER commit").
- ALWAYS include `fixes #<number>` or `closes #<number>` in the commit message when there is a related issue or PR.
- NEVER use `git add -A` or `git add .` — these sweep up changes from other agents.
- ALWAYS use `git add <specific-file-paths>` listing only files you modified.
- Before committing, run `git status` and verify you are only staging YOUR files.
- Track which files you created/modified/deleted during the session.
- Cargo.lock counts as yours if and only if your edits to `Cargo.toml` caused it to change.

### Forbidden Git Operations

These can destroy other agents' work:

- `git reset --hard` — destroys uncommitted changes
- `git checkout .` / `git restore .` — destroys uncommitted changes
- `git clean -fd` — deletes untracked files
- `git stash` — stashes ALL changes including other agents' work
- `git add -A` / `git add .` — stages other agents' uncommitted work
- `git commit --no-verify` — bypasses required checks; never allowed
- Force-push to `main` — never allowed; the patch-release workflow is the only thing that may force-move tags, and it does so via the workflow runner, not from your shell

### Safe Workflow

```bash
# 1. Check status
git status

# 2. Stage only your files
git add src-rust/crates/api/src/providers/foo.rs
git add docs/providers.md

# 3. Commit (only when the user has asked)
git commit -m "feat(api): add foo provider"

# 4. Push (pull --rebase if needed, but NEVER reset/checkout)
git pull --rebase && git push
```

### If Rebase Conflicts Occur

- Resolve conflicts in YOUR files only.
- If a conflict touches a file you didn't modify, abort the rebase and ask the user.
- NEVER force-push.

### User Override

If the user's instructions conflict with the rules above, ask for confirmation that they want to override the rules. Only then execute their instructions.
