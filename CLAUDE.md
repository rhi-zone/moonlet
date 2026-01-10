# CLAUDE.md

Behavioral rules for Claude Code in the spore repository.

## Project Overview

spore is an agentic AI framework spun out from moss. It provides:
- Multi-provider LLM client (via rig-core)
- SQLite-backed memory store
- Lua-based agent scripts

**Key distinction:**
- **spore** = agency/execution (LLM calls, memory, running agents)
- **moss** = intelligence (code analysis, session parsing, understanding)

The projects are intentionally not hard-linked. They can work independently or together via loose coupling.

## Architecture

```
crates/
└── spore-core/       # LLM client, memory store

scripts/
├── agent.lua         # Main agent state machine
└── agent/            # Agent submodules
    ├── risk.lua      # Risk assessment
    ├── parser.lua    # Command parsing
    ├── session.lua   # Session/checkpoint management
    ├── context.lua   # Context building
    ├── commands.lua  # Batch edit execution
    └── roles.lua     # Role-specific configs
```

## Key Types

### spore-core
- `Provider` - LLM provider enum (Anthropic, OpenAI, Gemini, etc.)
- `LlmClient` - Multi-provider LLM client with complete/chat methods
- `MemoryStore` - SQLite-backed key-value store with metadata
- `MemoryItem` - Stored memory with content, context, weight, metadata

## Supported LLM Providers

Anthropic, OpenAI, Azure, Gemini, Cohere, DeepSeek, Groq, Mistral, Ollama, OpenRouter, Perplexity, Together, XAI

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
```

## Environment Variables

- `SPORE_INSECURE_SSL` - Bypass SSL verification (for local proxies)
- `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc. - Provider API keys

## Conventions

- Crate names: `spore-{name}`
- Memory stored in `.spore/` directory
