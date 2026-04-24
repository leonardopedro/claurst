# Claurst

Your Favorite Terminal Coding Agent, now in Rust.

Claurst is an **open-source, multi-provider terminal coding agent** built from the ground up in Rust. It started as a clean-room reimplementation of Claude Code's behavior and has evolved into a full TUI pair programmer with multi-provider support, rich UI, plugin system, chat forking, memory consolidation, and native E2EE for TEE models.

It's fast, memory-efficient, yours to run however you want, and has no tracking or telemetry.

## Getting Started

### Download a release binary

Grab the latest binary for your platform from [GitHub Releases](https://github.com/kuberwastaken/claurst/releases):

| Platform | Binary |
|----------|--------|
| **Windows** x86_64 | `claurst-windows-x86_64.zip` |
| **Linux** x86_64 | `claurst-linux-x86_64.tar.gz` |
| **Linux** aarch64 | `claurst-linux-aarch64.tar.gz` |
| **macOS** Intel | `claurst-macos-x86_64.tar.gz` |
| **macOS** Apple Silicon | `claurst-macos-aarch64.tar.gz` |

### Build from source

```bash
git clone https://github.com/kuberwastaken/claurst.git
cd claurst/src-rust
cargo build --release --package claurst

# Binary is at target/release/claurst
```

**Raspberry Pi / systems without ALSA** (e.g. Debian Trixie, headless servers):

```bash
cargo build --release --package claurst --no-default-features
```

### First run

```bash
export ANTHROPIC_API_KEY=sk-ant-...
claurst

# Or run a one-shot headless query
claurst -p "explain this codebase"
```

## Supported Providers

30+ providers including: Anthropic, OpenAI, Google, GitHub Copilot, Ollama, DeepSeek, Groq, Mistral, Cohere, **Chutes (E2EE)**, and more. Run `/connect` inside Claurst to configure.

## Browser Automation 🆕

Claurst now includes native **Playwright** browser automation tools out of the box. No separate processes or proxies required.

### Installation

**Prerequisites:**
- Install **libjpeg-turbo** for image support: Download from [libjpeg-turbo releases](https://github.com/libjpeg-turbo/libjpeg-turbo/releases)
- Install Playwright browsers (one-time setup)

**Playwright browsers installation:**

```bash
# Install Chromium (recommended for most users)
npx playwright@1.59.1 install chromium

# Or install all browsers (Chromium, Firefox, WebKit)
npx playwright@1.59.1 install
```

**Note**: On Linux, you may also need system dependencies:
```bash
# Debian/Ubuntu
sudo apt-get install libjpeg-turbo8 libnss3 libatk1.0-0 libatk-bridge2.0-0 libcups2 libxkbcommon0 libxcomposite1 libxrandr2 libgbm1 libasound2

# Fedora
sudo dnf install nss atk at-spi2-atk cups-libs libxkbcommon libxrandr gbm alsa-lib
```

### Usage

The tools automatically integrate with the agent. Just ask it to:

```bash
# Navigate and interact
claurst -p "Go to example.com, find the h1 title, and take a screenshot"

# Fill forms
claurst -p "Navigate to https://example.com/login, fill the username field with 'test', and click submit"

# Testing workflow
claurst -p "Test the login form at myapp.com and verify it loads"
```

### Available Actions

The `playwright` tool supports these operations:

-  **`launch`**  : Start a new browser instance → returns `{"context_id": "uuid"}`
-  **`navigate`**  : Go to a URL
-  **`click`**  : Click elements by CSS selector
-  **`fill`**  : Fill form fields
-  **`text`**  : Extract text content from elements
-  **`screenshot`**  : Take screenshot (returns base64)
-  **`close`**  : Clean up browser instance

### Requirements

- Node.js (for browser installation)
- Rust toolchain (for claurst build)
- Playwright browsers matching version `0.12` of `playwright-rs`

### Architecture

Playwright tool is a **first-class tool** in claurst's toolset:
- Works with any provider (Anthropic, OpenAI, etc.)
- Respects permission modes
- Integrates with the TUI and CLI
- Fully async with tokio
- No separate proxy needed

**Note**: Browser automation runs locally. Ensure you have sufficient resources (RAM/CPU) when running tests.

## Domain-Aware Secret Management

Claurst includes native domain-aware password placeholders that integrate with existing GPG-encrypted password stores (ripasso/pass-compatible). Passwords remain as placeholders in prompts and UI, and are only replaced at the HTTP layer when sending requests to matching domains.

### Reference Format

```
{{pass:domain:secret-path[:mode[:field]]}}
```

- `domain` - Required. Only matching HTTP requests get real values
- `secret-path` - Path in password store (e.g., `api/api-key` → `~/.password-store/api/api-key.gpg`)
- `mode` (optional): `password` (default, first line), `full`, `field`
- `field` (optional): Field name for `field` mode (e.g., `{{pass:example.com:service-cred:field:password}}`)

### Usage in Prompts

```bash
# Configure password store location
export PASSWORD_STORE_DIR="$HOME/.password-store"

# User prompt - sees placeholders only
"Deploy to {{pass:api.staging.com:deploy-token}} and test"

# What LLM sees - same placeholders
"User requested: Deploy to {{pass:api.staging.com:deploy-token}}"

# What gets sent to matching domain - real value
curl -H "Authorization: Bearer AKIA-SECRET..." https://api.staging.com/deploy
```

### Usage for API Keys

You can also store your provider API keys in the password store:

```bash
# Store your API key
echo "sk-ant-..." | pass insert anthropic/api-key

# In config, use placeholder
# ~/.config/claurst/config.toml
api_key = "anthropic:api-key"
```

Or use environment variables directly:
```bash
export ANTHROPIC_API_KEY=pass:anthropic/api-key
```

The API key placeholder will be resolved when initializing the provider.

### Security Model

| Layer | What happens | Example |
|-------|--------------|---------|
| **LLM/Agent** | Only sees placeholders | `{{pass:api.staging.com:deploy-token}}` |
| **UI/CLI** | Shows placeholders | User sees `{{pass:api.staging.com:deploy-token}}` |
| **HTTP Tool** | Replaces ONLY for matching domain | Becomes `xK9m...` when sending to `api.staging.com` |
| **API Key Init** | Resolved at provider creation | `"anthropic:api-key"` → actual key for API |

### Configuration

```toml
# ~/.config/claurst/config.toml
[password_store]
store_path = "/home/user/.password-store"  # defaults to PASSWORD_STORE_DIR
signing_key = "user@example.com"           # optional, for signing
require_git = true                         # optional, require git repo

# Can also use placeholders directly
api_key = "anthropic:api-key"              # resolved at startup
```

Or use environment variables:
```bash
# Set API key as password reference
export ANTHROPIC_API_KEY="pass:anthropic/api-key"

# Traditional env var still works
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Implementation

- **Module**: `crates/core/src/password_store.rs`
- **Direct Resolution**: `resolve_password_value()` for API keys
- **HTTP Integration**: Replaces placeholders in commands and payloads
- **GPG**: Uses system `gpg` command with no Rust crypto dependencies
- **Zero-knowledge**: Only passwords relevant to destination domain are processed
- **Fallback**: NullPasswordStore means graceful operation without password store

## Chutes E2EE Provider

Claurst includes native end-to-end encryption for Chutes TEE (Trusted Execution Environment) models. This is a pure Rust reimplementation of the Chutes E2EE protocol — no external proxy required.

### How it works

```
Claurst ──[ML-KEM-768 + ChaCha20-Poly1305]──> api.chutes.ai/e2e/invoke ──> TEE GPU Instance
                                                    │
                         (load balancer sees only encrypted blobs)
```

All data is encrypted before leaving the Claurst process. Even the Chutes load balancer only sees opaque ciphertext. Only the GPU instance inside the TEE can decrypt.

### Protocol

| Step | Operation |
|------|-----------|
| 1. Discovery | `GET /e2e/instances/{chute_id}` → instance info, ML-KEM-768 pubkey, nonces |
| 2. Key exchange | ML-KEM-768 encapsulate to instance pubkey → shared secret |
| 3. Key derivation | HKDF-SHA256(shared_secret, salt=ct[0:16], info="e2e-req-v1") |
| 4. Encrypt | ChaCha20-Poly1305(gzip(augmented_payload)) |
| 5. Send | Binary blob: `[KEM_CT(1088)] + [NONCE(12)] + [CT+TAG(N)]` to `POST /e2e/invoke` |
| 6. Stream init | `{"e2e_init": "base64-kem-ct"}` → derive stream key via HKDF(info="e2e-stream-v1") |
| 7. Stream chunks | `{"e2e": "base64-nonce+ct+tag"}` → decrypt each chunk, parse as SSE |

### Configuration

```bash
export CHUTES_API_KEY="cpk_..."
```

Then use model `chutes/zai-org/GLM-5.1-TEE` (or any Chutes TEE model).

### Cryptographic primitives

| Algorithm | Purpose |
|-----------|---------|
| ML-KEM-768 (Kyber-768) | Post-quantum key encapsulation |
| ChaCha20-Poly1305 | AEAD symmetric encryption |
| HKDF-SHA256 | Key derivation with context separation |
| GZIP | Payload compression |

## Documentation

For more info on how to configure Claurst, [head over to our docs](https://claurst.kuber.studio/docs).

## Contributing

Claurst is built for the community, by the community. [Open an issue](https://github.com/Kuberwastaken/claurst/issues/new) for bugs or ideas, or [raise a PR](https://github.com/Kuberwastaken/claurst/pulls/new) to contribute.

## Important Notice

This repository does not hold a copy of the proprietary Claude Code TypeScript source code. This is a **clean-room Rust reimplementation** of Claude Code's behavior.

The process was explicitly two-phase:

**Specification** `spec/` — An AI agent analyzed the source and produced exhaustive behavioral specifications. No source code was carried forward.

**Implementation** `src-rust/` — A separate AI agent implemented from the spec alone, never referencing the original TypeScript. The output is idiomatic Rust that reproduces the behavior, not the expression.
