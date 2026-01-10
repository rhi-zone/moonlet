# spore

Agentic AI framework with Lua scripting - spun out from [moss](https://github.com/rhizome-lab/moss).

## Features

- **Multi-provider LLM client** - Anthropic, OpenAI, Gemini, Cohere, Groq, Mistral, and more via `rig-core`
- **Memory store** - SQLite-backed persistent context with metadata queries
- **Lua runtime** - Agent execution environment with plugin support
- **Agent scripts** - Lua-based agent implementation with state machine orchestration

## Crates

| Crate | Description |
|-------|-------------|
| `rhizome-spore-core` | LLM client and memory store infrastructure |
| `rhizome-spore-lua` | Lua runtime with Integration trait for plugins |

### Integrations

| Crate | Description |
|-------|-------------|
| `rhizome-spore-moss` | [Moss](https://github.com/rhizome-lab/moss) code intelligence integration |

## Quick Start

```rust
use rhizome_spore_core::{LlmClient, MemoryStore};
use rhizome_spore_lua::Runtime;

// Create LLM client
let client = LlmClient::new("anthropic", Some("claude-sonnet-4-5"))?;
let response = client.complete(None, "Hello, world!", Some(1000))?;

// Use memory store
let memory = MemoryStore::open(&project_root)?;
memory.store("context", Some("agent"), 1.0, serde_json::json!({}))?;

// Run Lua agent scripts
let runtime = Runtime::new()?;
runtime.run_file(Path::new("scripts/agent.lua"))?;
```

## Agent Scripts

The `scripts/` directory contains Lua agent implementations:

- `agent.lua` - Main state machine agent with planner/explorer/evaluator roles
- `agent/` - Submodules for risk assessment, command parsing, session management

## Integrations

Spore supports ecosystem integrations that add domain-specific capabilities to the Lua runtime:

```rust
use rhizome_spore_lua::Runtime;
use rhizome_spore_moss::MossIntegration;

let runtime = Runtime::new()?;
runtime.register(&MossIntegration::new("."))?;

// Now Lua scripts can use moss.view(), moss.edit(), moss.analyze.*, etc.
```

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
cd docs && bun dev # Local docs
```

## License

MIT OR Apache-2.0
