# Introduction

**moonlet** is an agentic AI framework spun out from [moss](https://github.com/rhizome-lab/moss). It provides the infrastructure for building and running AI agents.

## Philosophy

- **moonlet** = agency/execution (LLM calls, memory, running agents)
- **moss** = intelligence (code analysis, session parsing, understanding)

The projects are intentionally not hard-linked. Moss can optionally extend spore via the Integration trait.

## Components

### moonlet-core

Core infrastructure for AI agents:

- **LLM Client** - Multi-provider support via rig-core
  - Anthropic, OpenAI, Azure, Gemini, Cohere, DeepSeek
  - Groq, Mistral, Ollama, OpenRouter, Perplexity, Together, XAI
- **Memory Store** - SQLite-backed key-value store
  - Persistent context across sessions
  - Metadata-based queries
  - Weight-based relevance

### moonlet-lua

Lua runtime for agent execution:

- Hosts the agent scripts
- Integration trait for plugins
- Bindings to moonlet-core (LLM, memory)

### Integrations

Ecosystem plugins that add domain-specific capabilities:

- **moonlet-moss** - Code intelligence (view, edit, analyze, search)
- Future: moonlet-lotus, moonlet-resin, etc.

### Agent Scripts

Lua-based agent implementation:

- State machine with planner/explorer/evaluator roles
- Risk assessment for proposed changes
- Checkpoint/resume support
- Loop detection

## Quick Example

```rust
use moonlet_core::{LlmClient, MemoryStore};
use moonlet_lua::Runtime;

// Create LLM client
let client = LlmClient::new("anthropic", Some("claude-sonnet-4-5"))?;

// Complete a prompt
let response = client.complete(None, "Explain this code: ...", Some(1000))?;

// Store context in memory
let memory = MemoryStore::open(&project_root)?;
memory.store("explanation", Some("code-review"), 1.0, json!({}))?;

// Run agent
let runtime = Runtime::new()?;
runtime.run_file(Path::new("scripts/agent.lua"))?;
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
rhizome-moonlet-core = { git = "https://github.com/rhi-zone/moonlet" }
rhizome-moonlet-lua = { git = "https://github.com/rhi-zone/moonlet" }

# Optional: moss integration
rhizome-moonlet-moss = { git = "https://github.com/rhi-zone/moonlet" }
```
