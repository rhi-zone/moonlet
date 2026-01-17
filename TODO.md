# TODO

## For Iris (flora)

Iris needs these capabilities for agent-authored insights:

- [ ] `spore-embed` - Embedding generation integration
  - Providers: Gemini `text-embedding-004`, OpenAI `text-embedding-3-small`, ollama local models
  - API: `embed.generate(provider, model, text)` -> float vector
  - **Decision:** Separate crate from spore-llm. Rationale: embedding use cases (semantic search, clustering, RAG retrieval) are often distinct from chat/completion - users wanting embeddings don't necessarily want LLM inference support, and vice versa.

- [ ] Vector similarity in memory store (or separate crate)
  - Store embeddings alongside content
  - Query by cosine similarity
  - Options: extend spore-core memory, or new `spore-vec` crate with sqlite-vec

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

- [ ] `spore-libsql` - Direct libsql/SQLite access from Lua
- [ ] `spore-reed` - S-expression parsing/codegen (deferred: unclear value with single frontend/backend)

## Distribution

- [ ] Modular flake packaging
  - Expose each integration as a separate flake output (cffi-based plugins can be built independently)
  - Add `spore-full` package that depends on all integrations
  - Consider: config attrset to select which modules to include in a custom build

## spore-moss integration

### Not yet implemented

**`moss.analyze.security(path)`**
- Calls external tools (bandit for Python, etc.) which may not be installed
- Would need graceful fallback or feature detection
- Consider: should agents even run security scans, or is that a human-initiated action?

**`moss.analyze.docs(path, opts)`**
- Requires moss file index to be set up for cross-file interface resolution
- Without index, falls back to on-demand parsing which is slower
- Need to decide: should spore-moss manage its own index, or expect one to exist?

### Future considerations

- Batch edit support (`moss.edit.batch()`) - moss has BatchEdit for atomic multi-file edits
- Call graph queries (`moss.analyze.callers()`, `moss.analyze.callees()`) - requires index
- Duplicate detection (`moss.analyze.duplicates.functions()`, `moss.analyze.duplicates.types()`)
