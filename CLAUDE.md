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
    ├── moonlet-ecosystems/ # Package ecosystem queries
    └── moonlet-package-index/ # Package registry lookups

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
- **moonlet-ecosystems**: `ecosystems.capability({root})` returns capability with `:detect()`, `:query()`, `:dependencies()`, `:tree()`, `:audit()`
- **moonlet-package-index**: `package_index.list()`, `package_index.fetch(index, package)` for registry lookups
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

## Core Rule

**Note things down immediately:**
- Bugs/issues → fix or add to TODO.md
- Design decisions → docs/ or code comments
- Future work → TODO.md
- Key insights → this file

**Triggers:** User corrects you, 2+ failed attempts, "aha" moment, framework quirk discovered → document before proceeding.

**Conversation is not memory.** Anything said in chat evaporates at session end. If it implies future behavior change, write it to CLAUDE.md or a memory file immediately — or it will not happen.

**Warning — these phrases mean something needs to be written down right now:**
- "I won't do X again" / "I'll remember to..." / "I've learned that..."
- "Next time I'll..." / "From now on I'll..."
- Any acknowledgement of a recurring error without a corresponding CLAUDE.md or memory edit

**When the user corrects you:** Ask what rule would have prevented this, and write it before proceeding. **"The rule exists, I just didn't follow it" is never the diagnosis** — a rule that doesn't prevent the failure it describes is incomplete; fix the rule, not your behavior.

**Something unexpected is a signal, not noise.** Surprising output, anomalous numbers, files containing what they shouldn't — stop and ask why before continuing. Don't accept anomalies and move on.

**Do the work properly.** When asked to analyze X, actually read X - don't synthesize from conversation.

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

## Design Principles

**Unify, don't multiply.** One interface for multiple cases > separate interfaces. Plugin systems > hardcoded switches.

**Simplicity over cleverness.** HashMap > inventory crate. OnceLock > lazy_static. Functions > traits until you need the trait. Use ecosystem tooling over hand-rolling.

**Explicit over implicit.** Log when skipping. Show what's at stake before refusing.

**Separate niche from shared.** Don't bloat shared config with feature-specific data. Use separate files for specialized data.

## Negative Constraints

Do not:
- Announce actions ("I will now...") - just do them
- Leave work uncommitted
- Use interactive git commands (`git add -p`, `git add -i`, `git rebase -i`) — these block on stdin and hang in non-interactive shells; stage files by name instead
- Use path dependencies in Cargo.toml - causes clippy to stash changes across repos
- Use `--no-verify` - fix the issue or fix the hook
- Assume tools are missing - check if `nix develop` is available for the right environment

## Workflow

**Batch cargo commands** to minimize round-trips:
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test
```
After editing multiple files, run the full check once — not after each edit. Formatting is handled automatically by the pre-commit hook (`cargo fmt`).

**When making the same change across multiple crates**, edit all files first, then build once.

**Minimize file churn.** When editing a file, read it once, plan all changes, and apply them in one pass. Avoid read-edit-build-fail-read-fix cycles by thinking through the complete change before starting.

**Always commit completed work.** After tests pass, commit immediately — don't wait to be asked. When a plan has multiple phases, commit after each phase passes. Do not accumulate changes across phases. Uncommitted work is lost work.

**Use `normalize view` for structural exploration:**
```bash
~/git/rhizone/normalize/target/debug/normalize view <file>    # outline with line numbers
~/git/rhizone/normalize/target/debug/normalize view <dir>     # directory structure
```

## Context Management

**Use subagents to protect the main context window.** For broad exploration or mechanical multi-file work, delegate to an Explore or general-purpose subagent rather than running searches inline. The subagent returns a distilled summary; raw tool output stays out of the main context.

Rules of thumb:
- Research tasks (investigating a question, surveying patterns) → subagent; don't pollute main context with exploratory noise
- Searching >5 files or running >3 rounds of grep/read → use a subagent
- Codebase-wide analysis (architecture, patterns, cross-file survey) → always subagent
- Mechanical work across many files (applying the same change everywhere) → parallel subagents
- Single targeted lookup (one file, one symbol) → inline is fine

## Session Handoff

Use plan mode as a handoff mechanism when:
- A task is fully complete (committed, pushed, docs updated)
- The session has drifted from its original purpose
- Context has accumulated enough that a fresh start would help

**For handoffs:** enter plan mode, write a short plan pointing at TODO.md, and ExitPlanMode. **Do NOT investigate first** — the session is context-heavy and about to be discarded. The fresh session investigates after approval.

**For mid-session planning** on a different topic: investigating inside plan mode is fine — context isn't being thrown away.

Before the handoff plan, update TODO.md and memory files with anything worth preserving.

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
