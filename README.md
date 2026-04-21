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
