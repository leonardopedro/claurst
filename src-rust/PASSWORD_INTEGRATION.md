# Password Store Integration (Ripasso/Pass) with Domain-Aware Security

## Overview

This implementation adds password management to claurst using the ripasso/pass format. **Critically**, passwords are domain-isolated: a placeholder for `api.example.com` is only replaced when communicating with `api.example.com`. This ensures passwords never leak to other domains, LLMs, or tools.

## Placeholder Format

**New format requiring domain:** `{{pass:domain:path[:mode[:field]]}}`

Examples:
- `{{pass:api.example.com:aws-credentials}}` - Returns password (first line)
- `{{pass:api.example.com:aws-credentials:full}}` - Returns complete secret
- `{{pass:api.example.com:aws-credentials:field:access_key}}` - Returns specific field
- `{{pass:github.com:oauth:field:token}}` - GitHub token field

## Security Guarantees

1. **Domain-Isolated Replacement**: `replace_placeholders(text, store, destination_domain)` only replaces placeholders matching `destination_domain`
2. **No Leak to LLM**: Placeholders remain unchanged in prompts sent to LLM
3. **No Leak to Tools**: Placeholders remain in tool context unless tool explicitly targets the domain
4. **HTTP-Only Integration**: Replace happens just before HTTP requests are sent, not before LLM inference

## Architecture

### Core Modules (`claurst-core`)

**`password_store.rs`**
- `PasswordReference` - Parses `domain:path:mode:field`, extracts domain
- `replace_placeholders(text, store, domain)` - **Domain-aware** replacement
- `extract_placeholders()` - Find all placeholders for validation
- `has_domain_placeholders(text, domain)` - Check if domain has placeholders

**`password_store_ripasso.rs`**
- `RipassoPasswordStore` - Pass-compatible, uses system GPG
- Domain is NOT stored in encrypted file (single responsibility)
- Domain association happens at placeholder level

**`lib.rs` Exports**
```rust
pub use password_store::{
    PasswordStore, PasswordStoreConfig, PasswordStoreError,
    NullPasswordStore, PasswordReference, ReplacementMode,
    replace_placeholders, extract_placeholders, has_domain_placeholders
};
pub use password_store_ripasso::RipassoPasswordStore;
```

## Usage Examples

### Basic Domain-Aware Replacement

```rust
use claurst_core::password_store::{replace_placeholders, NullPasswordStore};

let store = NullPasswordStore::default();
let text = "API for api.example.com: {{pass:api.example.com:key}}";
let result = replace_placeholders(text, &store, "api.example.com");
// With NullPasswordStore: "API for api.example.com: ERROR: ..."
// With real store: "API for api.example.com: actual-key-value"
```

### Multi-Domain String

```rust
let text = "API: {{pass:api.example.com:user}} | Web: {{pass:web.example.com:pass}}";
// Replace ONLY for api.example.com domain:
let result = replace_placeholders(text, &store, "api.example.com");
// Result: "API: actual-user | Web: {{pass:web.example.com:pass}}"
// Web placeholder is preserved (security!)
```

### Validating Placeholders

```rust
let refs = extract_placeholders(text);
// Returns Vec of PasswordReference with domain, path, mode, field

let has = has_domain_placeholders(text, "api.example.com");
// Returns true if any placeholder matches that domain
```

## Integration Points

### 1. HTTP Request Layer (PRIMARY INTEGRATION)

**When making HTTP requests,** before sending:

```rust
use claurst_core::password_store::replace_placeholders;

fn make_http_request(url: &str, body: &str, store: &dyn PasswordStore) -> Result<Response> {
    // Get domain from URL
    let domain = extract_domain_from_url(url);
    
    // Replace placeholders JUST for this domain
    let clean_body = replace_placeholders(body, store, &domain)?;
    
    // Send to HTTP
    http_client.post(url).body(clean_body).send()
}
```

**Why HTTP layer?** 
- ✅ Real values sent ONLY to correct domain
- ✅ Placeholders visible in logs/ui
- ✅ Automatic domain extraction from URL
- ❌ Not in LLM prompt (prevents leak)

### 2. Tool Execution (Conditional)

Only tools that make network calls should replace:

```rust
impl Tool for HttpTool {
    fn execute(&self, input: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let domain = extract_domain(&self.url);
        
        // Check if placeholders exist for this domain
        if has_domain_placeholders(input, &domain) {
            let clean_input = replace_placeholders(input, ctx.password_store.as_ref().unwrap(), &domain)?;
            self.make_request(&clean_input)
        } else {
            self.make_request(input)
        }
    }
}
```

### 3. Query Loop

**DON'T** replace in user prompts (sent to LLM):

```rust
// ❌ WRONG - this leaks passwords to LLM
let prompt = replace_placeholders(&user_input, &store, "api.example.com")?;
llm.generate(prompt)

// ✅ CORRECT - keep placeholders, replace at HTTP level
let messages = vec![Message::user(user_input)]; // Placeholders intact
```

### 4. Configuration

Add to `Config`:

```rust
pub struct Config {
    pub password_store: PasswordStoreConfig,
    // ... rest
}
```

`settings.json`:
```json
{
  "passwordStore": {
    "path": "/home/user/.password-store",
    "requireGit": true
  }
}
```

Environment:
- `PASSWORD_STORE_DIR` - Standard pass variable
- `PASSWORD_STORE_SIGNING_KEY` - Key verification

## Implementation Status

✅ **Completed:**
- Domain-aware `PasswordReference` parsing
- `replace_placeholders(text, store, domain)` with strict domain matching
- Domain extraction helpers
- Full compatibility with ripasso/pass
- Comprehensive test coverage
- All tests passing

⏳ **Remaining:**
- HTTP layer integration (append replace_placeholders before send)
- ToolContext integration
- Domain extraction from URLs
- Configuration loading
- UI/CLI placeholder display

## Data Flow Example

```
User Input: "Call {{pass:api.example.com:auth}} API"
    ↓
LLM receives: "Call {{pass:api.example.com:auth}} API"  [Secure!]
    ↓
LLM outputs: "Use POST https://api.example.com/data with auth"
    ↓
HTTP layer extracts domain: "api.example.com"
    ↓
HTTP layer replaces: "POST https://api.example.com/data with actual-secret"
    ↓
Request sent to api.example.com with real secret
    ↓
Response: "Success"
    ↓
UI shows: "Called {{pass:api.example.com:auth}} API → Success"  [Secure!]
```

## Testing

```bash
# Run tests
cargo test --package claurst-core password_store

# With real store
mkdir -p /tmp/test-store
cd /tmp/test-store
gpg --batch --yes --passphrase "" --symmetric --cipher-algo AES256 <<< "mysecret" > mypass.gpg
```

```rust
// Test code
use claurst_core::password_store_ripasso::RipassoPasswordStore;
use claurst_core::password_store::replace_placeholders;

let store = RipassoPasswordStore::new("/tmp/test-store")?;
let input = "Key: {{pass:example.com:mypass}}";
let result = replace_placeholders(input, &store, "example.com")?;
// result = "Key: mysecret"
```

## Security Checklist

- [x] Domain MUST be in placeholder syntax
- [x] replace_placeholders() requires destination domain
- [x] No replace without domain match
- [x] Malformed placeholders kept unchanged
- [x] Full/field modes work with domain
- [x] Null store returns errors (not corrupt data)

## Design Decision: Separation of Concerns

**Password files** store: `path.gpg` → first line = password
**Placeholders** store: `{{pass:domain:path}}` → associate domain

This separation means:
- Password files work with standard `pass`
- Domain bind happens at reference level
- Same password can be used with multiple domains
- Easy to rotate passwords without changing domain mapping
