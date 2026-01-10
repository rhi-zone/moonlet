# spore

Agentic AI framework with Lua scripting - spun out from [moss](https://github.com/rhizome-lab/moss).

## Features

- **Multi-provider LLM client** - Anthropic, OpenAI, Gemini, Cohere, Groq, Mistral, and more via `rig-core`
- **Memory store** - SQLite-backed persistent context with metadata queries
- **Agent scripts** - Lua-based agent implementation with state machine orchestration

## Crates

| Crate | Description |
|-------|-------------|
| `spore-core` | LLM client and memory store infrastructure |

## Quick Start

```rust
use spore_core::{LlmClient, Provider, MemoryStore};

// Create LLM client
let client = LlmClient::new("anthropic", Some("claude-sonnet-4-5"))?;
let response = client.complete("Hello, world!", 1000).await?;

// Use memory store
let memory = MemoryStore::open(&project_root)?;
memory.store("context", Some("agent"), 1.0, serde_json::json!({}))?;
let items = memory.recall("context", 10)?;
```

## Agent Scripts

The `scripts/` directory contains Lua agent implementations:

- `agent.lua` - Main state machine agent with planner/explorer/evaluator roles
- `agent/` - Submodules for risk assessment, command parsing, session management

These scripts are designed to run within a Lua runtime (like mlua) and integrate with the spore-core infrastructure.

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
cd docs && bun dev # Local docs
```

## License

MIT OR Apache-2.0
