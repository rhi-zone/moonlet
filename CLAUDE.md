# CLAUDE.md

Behavioral rules for Claude Code in the spore repository.

## Project Overview

spore is an agentic AI framework spun out from moss. It provides:
- Multi-provider LLM client (via rig-core)
- SQLite-backed memory store
- Lua runtime with plugin support
- Agent scripts for autonomous task execution

**Key distinction:**
- **spore** = agency/execution (LLM calls, memory, running agents)
- **moss** = intelligence (code analysis, session parsing, understanding)

The projects are intentionally not hard-linked. Moss extends spore via dynamically loaded C ABI plugins.

## Architecture

```
crates/
├── spore-core/           # Memory store
├── spore-lua/            # Lua runtime, plugin loader
└── plugins/              # Dynamic C ABI plugins (cdylib)
    ├── spore-fs/         # Filesystem with capability-based security
    ├── spore-llm/        # Multi-provider LLM client
    ├── spore-embed/      # Multi-provider embedding generation
    ├── spore-libsql/     # LibSQL/SQLite with vector support
    ├── spore-moss/       # Code intelligence (view, search, analyze, edit)
    ├── spore-sessions/   # AI session parsing
    ├── spore-tools/      # Dev tools (linters, formatters, test runners)
    └── spore-packages/   # Package ecosystem queries

scripts/
├── agent.lua             # Main agent state machine
└── agent/                # Agent submodules
    ├── risk.lua          # Risk assessment
    ├── parser.lua        # Command parsing
    ├── session.lua       # Session/checkpoint management
    ├── context.lua       # Context building
    ├── commands.lua      # Batch edit execution
    └── roles.lua         # Role-specific configs
```

## Plugin System

Plugins are dynamically loaded shared libraries (`.so`/`.dylib`/`.dll`) that export:
- `spore_plugin_info()` - Version and ABI info
- `luaopen_spore_{name}()` - Lua module entry point

Plugins use capability-based security. Scripts receive capabilities via `caps` table:
```lua
-- In main.lua, with caps.fs.project injected
local file = caps.fs.project:open("src/main.rs", "r")
local content = file:read("*a")
```

## Key Types

### spore-core
- `MemoryStore` - SQLite-backed key-value store with metadata
- `MemoryItem` - Stored memory with content, context, weight, metadata

### spore-lua
- `Runtime` - Lua execution environment
- `PluginLoader` - Dynamic plugin discovery and loading

### Plugins
- **spore-llm**: `llm.complete()`, `llm.chat()`, `llm.providers()`
- **spore-embed**: `embed.generate(provider, model?, texts)`, `embed.start_generate()` (async), `embed.providers()`
- **spore-libsql**: `libsql.open(path)`, `libsql.open_memory()`, `libsql.vector32()`, `libsql.vector64()`; Connection with `:execute()`, `:query()`, `:close()`
- **spore-moss**: `moss.capability({root, mode})` returns capability with `:view()`, `:search()`, `:complexity()`, `:security()`, `:docs()`, `:files()`, `:duplicates()`, `:hotspots()`, `:stale_docs()`, `:check_refs()`, `:ast()`, `:query()`, `:trace()`, `:callers()`, `:callees()`, `:find()`, `:replace()`, etc.
- **spore-tools**: `tools.capability({root})` returns capability with `:run()`, `:fix()`, `:test_run()`, etc.
- **spore-packages**: `packages.capability({root})` returns capability with `:query()`, `:dependencies()`, `:audit()`
- **spore-sessions**: `sessions.parse()`, `sessions.list()`, `sessions.formats()`
- **spore-fs**: `fs.capability({path, mode})` returns capability with `:open()`, `:read()`, `:write()`, etc.

## Supported LLM Providers

Anthropic, OpenAI, Azure, Gemini, Cohere, DeepSeek, Groq, Mistral, Ollama, OpenRouter, Perplexity, Together, XAI

## Supported Embedding Providers

OpenAI, Azure, Gemini, Cohere, Mistral, Ollama, Together

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
```

## Environment Variables

- `SPORE_INSECURE_SSL` - Bypass SSL verification (for local proxies)
- `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc. - Provider API keys

## Behavioral Patterns

From ecosystem-wide session analysis:

- **Question scope early:** Before implementing, ask whether it belongs in this crate/module
- **Check consistency:** Look at how similar things are done elsewhere in the codebase
- **Implement fully:** No silent arbitrary caps, incomplete pagination, or unexposed trait methods
- **Name for purpose:** Avoid names that describe one consumer
- **Verify before stating:** Don't assert API behavior or codebase facts without checking

## Conventions

- Crate names: `rhizome-spore-{name}`
- Memory stored in `.spore/` directory
- Plugins live in `crates/plugins/` (cdylib crates)
- Plugins export `luaopen_spore_{name}()` C function
