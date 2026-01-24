# Moss Integrations Design

Design document for adding `moonlet-moss-tools` and `moonlet-moss-packages` integrations.

## Context

Spore already has `spore-moss` (code analysis, editing) and `spore-moss-sessions` (session parsing). This document covers adding bindings for the remaining moss modules:

- **moss-tools**: External tool execution (linters, formatters, type checkers, test runners)
- **moss-packages**: Package ecosystem queries and registry index lookups

## Module Overview

### moss-tools

Provides unified interface for external development tools:

**Tool Registry** (linters, formatters, type checkers):
- Adapters: oxlint, eslint, ruff, biome, prettier, black, rustfmt, tsc, mypy, pyright, cargo check
- Custom tools via `.moss/tools.toml`
- SARIF output format support
- Detection: which tools are relevant for a project

**Test Runners**:
- Adapters: cargo test, pytest, go test, npm test, bun test
- Detection: find best runner for project
- Execution: run tests, capture output

Key types:
```rust
// Tool registry
pub trait Tool: Send + Sync {
    fn info(&self) -> &ToolInfo;
    fn is_available(&self) -> bool;
    fn detect(&self, root: &Path) -> f32;          // 0.0-1.0 confidence
    fn run(&self, paths: &[&Path], root: &Path) -> Result<ToolResult, ToolError>;
    fn fix(&self, paths: &[&Path], root: &Path) -> Result<ToolResult, ToolError>;
}

pub struct ToolResult {
    pub tool: String,
    pub diagnostics: Vec<Diagnostic>,
    pub success: bool,
    pub error: Option<String>,
}

pub struct Diagnostic {
    pub severity: DiagnosticSeverity,  // Error, Warning, Info, Hint
    pub message: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub rule: Option<String>,
    pub fix: Option<Fix>,
}

// Test runners
pub trait TestRunner: Send + Sync {
    fn info(&self) -> TestRunnerInfo;
    fn is_available(&self) -> bool;
    fn detect(&self, root: &Path) -> f32;
    fn run(&self, root: &Path, args: &[&str]) -> std::io::Result<TestResult>;
}

pub struct TestResult {
    pub runner: String,
    pub status: ExitStatus,
}
```

### moss-packages

Two feature-gated capabilities:

**Ecosystem feature** (project-level dependency management):
- Detect ecosystem from manifest files (Cargo.toml, package.json, pyproject.toml, etc.)
- Query package info via available tools (cargo, npm, pip, etc.)
- List declared dependencies
- Get dependency tree from lockfile
- Security audit for known vulnerabilities

**Index feature** (registry-level package lookup):
- 60+ package indices: npm, pip, cargo, apt, brew, nix, docker, maven, etc.
- Fetch package metadata from registries
- Cross-platform package mapping

Key types:
```rust
// Ecosystem
pub trait Ecosystem: Send + Sync {
    fn name(&self) -> &'static str;
    fn manifest_files(&self) -> &'static [&'static str];
    fn query(&self, package: &str, project_root: &Path) -> Result<PackageInfo, PackageError>;
    fn list_dependencies(&self, project_root: &Path) -> Result<Vec<Dependency>, PackageError>;
    fn dependency_tree(&self, project_root: &Path) -> Result<DependencyTree, PackageError>;
    fn audit(&self, project_root: &Path) -> Result<AuditResult, PackageError>;
}

pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub features: Vec<Feature>,
    pub dependencies: Vec<Dependency>,
}

pub struct Vulnerability {
    pub package: String,
    pub version: String,
    pub severity: VulnerabilitySeverity,  // Critical, High, Medium, Low, Unknown
    pub title: String,
    pub cve: Option<String>,
    pub fixed_in: Option<String>,
}

// Index
pub trait PackageIndex: Send + Sync {
    fn ecosystem(&self) -> &'static str;
    fn fetch(&self, name: &str) -> Result<PackageMeta, IndexError>;
}
```

## Design Decisions

### Lua Namespace

**Decision**: Separate top-level globals, consistent with existing integrations.

```lua
-- Existing
llm.complete(...)
sessions.parse(...)
moss.view(...)

-- New
tools.run(...)
tools.test.run(...)
packages.ecosystem.query(...)
packages.index.fetch(...)
```

Rationale: Each integration is independent. Users enable what they need. Nesting under `moss.*` would create false coupling.

### Crate Structure

**Decision**: Two separate crates.

```
crates/integrations/
├── spore-moss/           # existing
├── spore-moss-sessions/  # existing
├── moonlet-moss-tools/     # new
└── moonlet-moss-packages/  # new
```

Config:
```toml
# .spore/config.toml
[integrations]
moss = true
moss_sessions = true
moss_tools = true
moss_packages = true
```

### Missing Tool Handling

**Decision**: Error when tool not available (idiomatic Lua).

```lua
-- Tool not installed -> error
local ok, result = pcall(tools.run, "ruff", {"src/"})
if not ok then
    print("ruff not available: " .. result)
end

-- Check availability explicitly if needed
if tools.is_available("ruff") then
    local result = tools.run("ruff", {"src/"})
end
```

Rationale: Errors are idiomatic. Returning `{available = false}` would require checking a field on every call.

### Streaming vs Captured Output

**Decision**: Separate functions, not overloaded.

```lua
-- Captured (synchronous, returns when done)
local result = tools.test.run("cargo")
-- result = {runner = "cargo", success = true, output = "..."}

-- Streaming (returns handle immediately)
local handle = tools.test.start("cargo")
while handle:is_running() do
    local line = handle:read_line()
    if line then print(line) end
end
local result = handle:wait()
```

Rationale: Overloading makes types ambiguous. Separate functions have clear signatures.

### Parallel Execution

**Decision**: Explicit handles + poll, no hidden global state.

```lua
local h1 = tools.test.start("cargo")
local h2 = tools.test.start("pytest")

-- Poll returns handles that have data ready
while spore.any_running({h1, h2}) do
    local ready = spore.poll({h1, h2}, {timeout_ms = 100})
    for _, h in ipairs(ready) do
        local line = h:read_line()
        if line then print(h.name .. ": " .. line) end
    end
end

local r1 = h1:wait()
local r2 = h2:wait()
```

Rationale: No global scheduler state. Handles are explicit values. `poll` is a pure function over handles.

### Index Scope

**Decision**: Expose all 60+ indices.

```lua
local indices = packages.index.list()  -- returns all available index names
local info = packages.index.fetch("npm", "lodash")
local info = packages.index.fetch("brew", "ripgrep")
```

Rationale: Let users pick what they need. No artificial curation.

### Capabilities (Deferred)

**Decision**: Design properly, implement later.

Current state: Integrations have implicit capabilities (network, fs). This is expedient but not principled.

Desired state: Fine-grained object-capability model.

```lua
-- Future: explicit capabilities
local npm_net = caps.network_host("registry.npmjs.org")
local output_fs = caps.fs_path("./output", "rw")

packages.index.fetch("npm", "lodash", {network = npm_net})
moss.edit.write("./output/result.txt", content, {fs = output_fs})
```

Why defer:
- Significant design work (affects all integrations, not just these two)
- Need to decide: per-call vs per-integration vs per-runtime
- Current behavior is acceptable for initial release

Follow-up: Dedicated capability system design doc.

## Proposed Lua API

### moonlet-moss-tools

```lua
-- Tool registry
tools.list()                           -- list all tool names
tools.list({category = "linter"})      -- filter by category
tools.is_available(name)               -- check if tool binary exists
tools.detect(root?)                    -- detect relevant tools for project
tools.info(name)                       -- get tool info (category, extensions, etc.)

-- Run tools (synchronous, captured output)
tools.run(name, paths?, opts?)         -- run tool, return diagnostics
tools.fix(name, paths?, opts?)         -- run tool in fix mode

-- Result structure:
-- {
--   tool = "ruff",
--   success = true,
--   diagnostics = {
--     {severity = "error", message = "...", file = "...", line = 1, column = 1, rule = "E501"},
--     ...
--   },
--   error = nil
-- }

-- Test runners
tools.test.list()                      -- list all runner names
tools.test.is_available(name)          -- check if runner available
tools.test.detect(root?)               -- detect best runner for project

-- Run tests (synchronous, captured output)
tools.test.run(name?, args?, opts?)    -- run tests, return result
-- If name is nil, auto-detects runner

-- Result structure:
-- {
--   runner = "cargo",
--   success = true,
--   output = "...",
--   duration_ms = 1234
-- }

-- Run tests (streaming, for interactive use)
tools.test.start(name?, args?, opts?)  -- start tests, return handle
-- Returns Handle (see async design below)
```

### moonlet-moss-packages

```lua
-- Ecosystem detection and queries
packages.ecosystem.list()              -- list all ecosystem names
packages.ecosystem.detect(root?)       -- detect ecosystem for project
packages.ecosystem.is_available(name)  -- check if ecosystem tool available

-- Package queries (uses detected/specified ecosystem)
packages.query(package, opts?)         -- query package info
packages.query("serde")                -- auto-detect ecosystem
packages.query("lodash", {ecosystem = "npm"})

-- Result structure:
-- {
--   name = "serde",
--   version = "1.0.195",
--   description = "A generic serialization/deserialization framework",
--   license = "MIT OR Apache-2.0",
--   homepage = "https://serde.rs",
--   repository = "https://github.com/serde-rs/serde",
--   dependencies = {{name = "serde_derive", version_req = "1.0", optional = true}, ...}
-- }

-- Project dependencies
packages.dependencies(root?)           -- list declared dependencies
packages.tree(root?)                   -- get dependency tree

-- Security audit
packages.audit(root?)                  -- check for vulnerabilities
-- Result structure:
-- {
--   vulnerabilities = {
--     {package = "foo", version = "1.0", severity = "high", title = "...", cve = "CVE-..."},
--     ...
--   }
-- }

-- Package index (registry queries, requires network)
packages.index.list()                  -- list all index names (60+)
packages.index.fetch(index, package)   -- fetch package metadata
packages.index.fetch("npm", "lodash")
packages.index.fetch("brew", "ripgrep")
packages.index.fetch("apt", "nginx")

-- Result structure:
-- {
--   name = "lodash",
--   version = "4.17.21",
--   description = "Lodash modular utilities",
--   license = "MIT",
--   repository = "https://github.com/lodash/lodash",
-- }
```

## Async Handle Design

For streaming output and parallel execution, we need a `Handle` type.

### Requirements

1. No hidden global state
2. Support streaming output line-by-line
3. Support waiting for completion
4. Support polling multiple handles
5. Proper cleanup on drop

### Handle API

```lua
-- Handle is a Lua userdata
local h = tools.test.start("cargo")

-- Properties
h.name          -- "cargo"
h:is_running()  -- true/false

-- Reading output (non-blocking, nil if no data ready)
h:read_stdout()  -- read line from stdout
h:read_stderr()  -- read line from stderr
h:read_any()     -- read from either, returns {stream = "stdout"|"stderr", line = "..."}

-- Drain all available lines
h:drain_stdout() -- returns array of lines
h:drain_stderr() -- returns array of lines
h:drain_any()    -- returns array of {stream, line}

-- Blocking operations
h:wait()         -- block until complete, return final result

-- Cancellation
h:kill()         -- terminate the subprocess
```

### Poll API

```lua
-- Check if any handles are still running
spore.any_running({h1, h2, h3})  -- returns true/false

-- Wait for any handle to have data (with timeout)
local ready = spore.poll({h1, h2, h3}, {timeout_ms = 100})
-- Returns array of handles that have data ready

-- Wait for all handles to complete
local results = spore.wait_all({h1, h2, h3})
-- Returns array of results in same order
```

### Implementation Sketch

Rust side:
```rust
// Handle wraps separate channel receivers for stdout/stderr
pub struct Handle {
    name: String,
    stdout: Receiver<String>,
    stderr: Receiver<String>,
    join_handle: Option<JoinHandle<ProcessResult>>,
    result: Option<ProcessResult>,
    config: HandleConfig,
}

pub struct HandleConfig {
    buffer_size: Option<usize>,        // None = unbounded
    overflow: OverflowStrategy,
}

pub enum OverflowStrategy {
    DropOldest,
    DropNewest,
    Block,
}

struct ProcessResult {
    success: bool,
    exit_code: Option<i32>,
}

impl Handle {
    fn is_running(&self) -> bool {
        self.result.is_none()
    }

    fn read_stdout(&self) -> Option<String> {
        self.stdout.try_recv().ok()
    }

    fn read_stderr(&self) -> Option<String> {
        self.stderr.try_recv().ok()
    }

    fn read_any(&self) -> Option<(Stream, String)> {
        // Use select! to read from whichever has data
        crossbeam_channel::select! {
            recv(self.stdout) -> msg => msg.ok().map(|s| (Stream::Stdout, s)),
            recv(self.stderr) -> msg => msg.ok().map(|s| (Stream::Stderr, s)),
            default => None,
        }
    }

    fn wait(&mut self) -> &ProcessResult {
        if self.result.is_none() {
            // Drain remaining output (caller should have read it)
            while self.stdout.try_recv().is_ok() {}
            while self.stderr.try_recv().is_ok() {}
            // Wait for thread
            if let Some(jh) = self.join_handle.take() {
                self.result = Some(jh.join().unwrap());
            }
        }
        self.result.as_ref().unwrap()
    }

    fn kill(&mut self) {
        // Send kill signal to subprocess
        // Implementation depends on how we're tracking the child process
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if self.is_running() {
            self.kill();
        }
    }
}

pub enum Stream {
    Stdout,
    Stderr,
}

// poll implementation uses crossbeam-channel's select! macro
fn poll(handles: &[&Handle], timeout: Duration) -> Vec<usize> {
    // Build a select over all receivers (stdout + stderr for each handle)
    // Return indices of handles with data ready on either stream
}
```

### Design Decisions

**Stderr handling**: Separate stream from stdout (semantically different).

```lua
h:read_stdout()    -- read from stdout
h:read_stderr()    -- read from stderr
h:read_any()       -- read from either, returns {stream = "stdout"|"stderr", line = "..."}
```

**Handle lifecycle**: Kill subprocess when handle dropped. Rationale: explicit cleanup, no orphan processes.

```lua
-- Default: kill on drop
local h = tools.test.start("cargo")
h = nil  -- process killed

-- Future (low priority): explicit detach if needed
h:detach()  -- process continues, handle becomes inert
```

**Backpressure**: Configurable via predefined strategies.

```lua
local h = tools.test.start("cargo", {
    buffer_size = 1000,           -- nil = unbounded (default)
    overflow = "drop_oldest"      -- or "drop_newest", "block"
})
```

Strategies:
- `nil` / unbounded: No limit, grows as needed (default, fine for typical use)
- `"drop_oldest"`: Ring buffer, oldest lines discarded
- `"drop_newest"`: New lines ignored when full
- `"block"`: Producer blocks until space available (risk: deadlock if Lua never reads)

User-implementable callback not viable (would be called from Rust thread, can't safely call into Lua).

### Open Questions (Low Priority)

1. Should `detach()` be exposed? Use case unclear.
2. Should overflow strategy be per-stream (stdout vs stderr)?

### Remaining Design Work

Before implementing Phase 2 (async Handle), flesh out:

1. **Poll implementation**: How to build a crossbeam `select!` dynamically across N handles, each with 2 channels (stdout + stderr). May need `crossbeam-channel`'s `Select` struct for runtime-constructed selects.

2. **Subprocess spawning in `start()`**:
   - Use `std::process::Command` with `Stdio::piped()` for stdout/stderr
   - Spawn reader threads for each pipe, send lines over channels
   - Track `Child` handle for kill support
   - Wire up bounded/unbounded channels based on config

3. **mlua UserData for Handle**:
   - Implement `UserData` trait
   - Expose methods: `is_running`, `read_stdout`, `read_stderr`, `read_any`, `drain_*`, `wait`, `kill`
   - Handle mutability (UserData methods take `&self`, but we need `&mut self` for some ops - use interior mutability with `RefCell` or `Mutex`)

4. **`spore.poll` / `spore.any_running` / `spore.wait_all`**:
   - These are global functions, not methods on Handle
   - Need to extract receivers from multiple Handle userdata values
   - Consider: should these live in `spore-lua` (shared infra) or a new `spore-async` crate?

## Implementation Plan

### Phase 1: Sync API

1. Create `moonlet-moss-tools` crate
   - Tool registry bindings (list, detect, run, fix)
   - Test runner bindings (list, detect, run)
   - Sync only, captured output

2. Create `moonlet-moss-packages` crate
   - Ecosystem bindings (detect, query, dependencies, tree, audit)
   - Index bindings (list, fetch)

3. Add config flags and integration registration

### Phase 2: Async Handle

1. Add `Handle` type to `spore-lua` (shared infrastructure)
2. Add `spore.poll`, `spore.any_running`, `spore.wait_all`
3. Add `tools.test.start()` returning Handle

### Phase 3: Capabilities (Future)

Separate design doc for capability system affecting all integrations.

### Phase 4: CFFI Plugin Architecture (Future)

Current integrations are compiled directly into the spore binary. A CFFI-based plugin system would allow:

- Dynamically loaded integrations (`.so`/`.dylib` files)
- Adding integrations without recompiling spore
- Distributing integrations as separate packages
- Language-agnostic plugins (any language that can produce C-compatible shared libraries)

Design considerations:
- Plugin discovery (search paths, manifest files)
- ABI stability for the Integration trait
- Error handling across FFI boundary
- Memory safety (who owns what)
- Versioning and compatibility

This is a significant architectural change. See separate design doc (to be created).

## Dependencies

```toml
# moonlet-moss-tools
[dependencies]
rhizome-spore-lua.workspace = true
rhizome-moss-tools = { git = "https://github.com/rhizome-lab/moss" }
mlua.workspace = true
serde_json.workspace = true

# moonlet-moss-packages
[dependencies]
rhizome-spore-lua.workspace = true
rhizome-moss-packages = { git = "https://github.com/rhizome-lab/moss", features = ["ecosystem", "index"] }
mlua.workspace = true
serde_json.workspace = true
```
