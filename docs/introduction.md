# Introduction

**spore** is an agentic AI framework spun out from [moss](https://github.com/rhizome-lab/moss). It provides the infrastructure for building AI coding agents.

## Components

### spore-core

Core infrastructure for AI agents:

- **LLM Client** - Multi-provider support via rig-core
  - Anthropic, OpenAI, Azure, Gemini, Cohere, DeepSeek
  - Groq, Mistral, Ollama, OpenRouter, Perplexity, Together, XAI
- **Memory Store** - SQLite-backed key-value store
  - Persistent context across sessions
  - Metadata-based queries
  - Weight-based relevance

### spore-sessions

Session log parsing for various AI coding agents:

- Claude Code (JSONL)
- Gemini CLI (JSON)
- Codex
- Moss Agent (JSONL)

Provides unified `SessionAnalysis` with:
- Tool usage statistics
- Token consumption metrics
- Error pattern analysis
- File access tracking

### Agent Scripts

Lua-based agent implementation:

- State machine with planner/explorer/evaluator roles
- Risk assessment for proposed changes
- Checkpoint/resume support
- Loop detection

## Quick Example

```rust
use spore_core::{LlmClient, MemoryStore};

// Create LLM client
let client = LlmClient::new("anthropic", Some("claude-sonnet-4-5"))?;

// Complete a prompt
let response = client.complete("Explain this code: ...", 1000).await?;

// Store context in memory
let memory = MemoryStore::open(&project_root)?;
memory.store("explanation", Some("code-review"), 1.0, json!({}))?;
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
spore-core = { git = "https://github.com/rhizome-lab/spore" }
spore-sessions = { git = "https://github.com/rhizome-lab/spore" }
```
