# CLAUDE.md

Behavioral rules for Claude Code in the moonlet repository.

## Project Overview

moonlet is an agentic AI framework spun out from moss. It provides:
- Multi-provider LLM client (via rig-core)
- SQLite-backed memory store
- Lua runtime with plugin support
- Agent scripts for autonomous task execution

**Key distinction:**
- **moonlet** = agency/execution (LLM calls, memory, running agents)
- **moss** = intelligence (code analysis, session parsing, understanding)

The projects are intentionally not hard-linked. Moss extends moonlet via dynamically loaded C ABI plugins.

## Architecture

```
crates/
├── moonlet-core/         # Memory store
├── moonlet-lua/          # Lua runtime, plugin loader
└── plugins/              # Dynamic C ABI plugins (cdylib)
    ├── moonlet-fs/       # Filesystem with capability-based security
    ├── moonlet-llm/      # Multi-provider LLM client
    ├── moonlet-embed/    # Multi-provider embedding generation
    ├── moonlet-libsql/   # LibSQL/SQLite with vector support
    ├── moonlet-moss/     # Code intelligence (view, search, analyze, edit)
    ├── moonlet-sessions/ # AI session parsing
    ├── moonlet-tools/    # Dev tools (linters, formatters, test runners)
    └── moonlet-packages/ # Package ecosystem queries

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
- `moonlet_plugin_info()` - Version and ABI info
- `luaopen_moonlet_{name}()` - Lua module entry point

Plugins use capability-based security. Scripts receive capabilities via `caps` table:
```lua
-- In main.lua, with caps.fs.project injected
local file = caps.fs.project:open("src/main.rs", "r")
local content = file:read("*a")
```

## Key Types

### moonlet-core
- `MemoryStore` - SQLite-backed key-value store with metadata
- `MemoryItem` - Stored memory with content, context, weight, metadata

### moonlet-lua
- `Runtime` - Lua execution environment
- `PluginLoader` - Dynamic plugin discovery and loading

### Plugins
- **moonlet-llm**: `llm.capability({providers, models?})` returns capability with `:providers()`, `:provider_info(name)`, `:complete(provider, model?, system?, prompt)`, `:chat(provider, model?, system?, message, history?)`, `:start_chat(...)` (async)
- **moonlet-embed**: `embed.capability({providers, models?})` returns capability with `:providers()`, `:provider_info(name)`, `:generate(provider, model?, texts)`, `:start_generate(...)` (async)
- **moonlet-libsql**: `libsql.capability({path?, allow_memory?})` returns capability with `:open(path)`, `:open_memory()`, `:vector32(array)`, `:vector64(array)`; Connection with `:execute()`, `:query()`, `:close()`
- **moonlet-moss**: `moss.capability({root, mode})` returns capability with `:view()`, `:search()`, `:complexity()`, `:security()`, `:docs()`, `:files()`, `:duplicates()`, `:hotspots()`, `:stale_docs()`, `:check_refs()`, `:ast()`, `:query()`, `:trace()`, `:callers()`, `:callees()`, `:find()`, `:replace()`, etc.
- **moonlet-tools**: `tools.capability({root})` returns capability with `:run()`, `:fix()`, `:test_run()`, etc.
- **moonlet-packages**: `packages.capability({root})` returns capability with `:query()`, `:dependencies()`, `:audit()`
- **moonlet-sessions**: `sessions.capability({root})` returns capability with `:parse()`, `:parse_with_format()`, `:list()`, `:detect()`, `:formats()`
- **moonlet-fs**: `fs.capability({path, mode})` returns capability with `:open()`, `:read()`, `:write()`, etc.

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

- `MOONLET_INSECURE_SSL` - Bypass SSL verification (for local proxies)
- `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc. - Provider API keys

## Behavioral Patterns

From ecosystem-wide session analysis:

- **Question scope early:** Before implementing, ask whether it belongs in this crate/module
- **Check consistency:** Look at how similar things are done elsewhere in the codebase
- **Implement fully:** No silent arbitrary caps, incomplete pagination, or unexposed trait methods
- **Name for purpose:** Avoid names that describe one consumer
- **Verify before stating:** Don't assert API behavior or codebase facts without checking

## Conventions

- Crate names: `moonlet-{name}`
- Memory stored in `.moonlet/` directory
- Plugins live in `crates/plugins/` (cdylib crates)
- Plugins export `luaopen_moonlet_{name}()` C function

## Negative Constraints

Do not:
- Use path dependencies in Cargo.toml - causes clippy to stash changes across repos
- Use `--no-verify` - fix the issue or fix the hook
- Assume tools are missing - check if `nix develop` is available for the right environment

## Commit Convention

Use conventional commits: `type(scope): message`

Types:
- `feat` - New feature
- `fix` - Bug fix
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `docs` - Documentation only
- `chore` - Maintenance (deps, CI, etc.)
- `test` - Adding or updating tests

Scope is optional but recommended for multi-crate repos.
