# TODO

## For Iris (flora)

Iris needs these capabilities for agent-authored insights:

- [x] `spore-embed` - Embedding generation integration
  - Providers: OpenAI, Azure, Gemini, Cohere, Mistral, Ollama, Together
  - API: `embed.generate(provider, model?, texts)` -> array of float vectors
  - Async: `embed.start_generate(provider, model?, texts)` -> Handle
  - Uses rig-core EmbeddingModel trait

- [x] `spore-libsql` - LibSQL/SQLite with native vector support
  - API: `libsql.open(path)`, `libsql.open_memory()` -> Connection
  - Connection: `conn:execute(sql, params?)`, `conn:query(sql, params?)`
  - Vector helpers: `libsql.vector32(array)`, `libsql.vector64(array)`
  - Native vector similarity via `vector_distance_cos()`, `vector_top_k()`
  - DiskANN indexes for fast KNN queries

- [x] `spore-moss-sessions` - Session parsing integration
  - Wraps moss-sessions for Lua access
  - API: `sessions.parse(path)` -> session data, `sessions.list()`, `sessions.formats()`, `sessions.detect(path)`
  - Formats: Claude Code, Gemini CLI, Codex, Moss Agent
  - Analysis is done in Lua (not Rust) - see `moss/docs/design/sessions-refactor.md`

## CLI

- [x] `spore` crate - CLI binary that:
  - Reads `.spore/config.toml`
  - Sets up Lua runtime with requested integrations
  - Runs the entry point script
  - Commands: `spore run [path]`, `spore init [path]`

## Architecture

- [x] Move LLM client from spore-core to spore-llm integration
- [x] spore-core should be minimal (just runtime infrastructure)

## Integrations

- [x] `spore-libsql` - Direct libsql/SQLite access from Lua (with vector support)
- [ ] `spore-reed` - S-expression parsing/codegen (deferred: unclear value with single frontend/backend)

## Distribution

- [ ] Modular flake packaging
  - Expose each integration as a separate flake output (cffi-based plugins can be built independently)
  - Add `spore-full` package that depends on all integrations
  - Consider: config attrset to select which modules to include in a custom build

## spore-moss integration

### Implemented

- [x] `cap:security()` - Security analysis (runs bandit for Python, graceful fallback if not installed)
- [x] `cap:docs(limit?)` - Documentation coverage analysis (per-language breakdown, worst files)
- [x] `cap:files(limit?)` - Large files analysis (by lines, by language)
- [x] `cap:duplicates(opts?)` - Duplicate function detection
- [x] `cap:hotspots()` - Git churn hotspot analysis
- [x] `cap:stale_docs()` - Find stale documentation
- [x] `cap:check_refs()` - Check documentation references

### Future considerations

- Batch edit support (`moss.edit.batch()`) - moss has BatchEdit for atomic multi-file edits
- Call graph queries (`cap:callers()`, `cap:callees()`) - requires moss index
