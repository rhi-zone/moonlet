# Plugin Architecture Design

Spore integrations as dynamically loaded plugins instead of compiled-in modules.

## Goals

1. **Minimal core**: spore binary is just runtime + plugin loader
2. **Dynamic loading**: integrations loaded at runtime from shared libraries
3. **Independent distribution**: plugins can be built/distributed separately
4. **Capability-based API**: capabilities are userdata with methods, not global functions
5. **No mlua coupling**: plugins use raw Lua C API, not mlua

## Prior Art

See `~/git/lotus/crates/plugins/*/src/lib.rs` for reference implementation:
- Raw Lua C API (`lua_State*`), not mlua wrappers
- Capability validation on every operation
- Standard Lua C function signature: `extern "C" fn(L: *mut lua_State) -> c_int`

## Current State (Compiled-In)

```
spore (binary)
├── spore-lua (runtime)
├── spore-llm (compiled in)
├── spore-moss (compiled in)
├── spore-moss-tools (compiled in)
└── ...
```

Config toggles registration, but everything is linked:
```toml
[integrations]
llm = true      # just controls whether register() is called
moss = false    # code is still in binary
```

## Target State (Dynamic Plugins)

```
spore (binary)
├── spore-lua (runtime)
└── plugin loader

~/.spore/plugins/
├── libspore_fs.so
├── libspore_ai.so
├── libspore_moss.so
└── ...
```

## Capability-Based API

### Why Not String Registry

A central registry like `spore.capability("fs", ...)` has problems:
- Who owns the "fs" name? Conflicts between plugins?
- Discovery is opaque
- Not Lua-idiomatic

### Plugins as Lua Modules

Plugins are Lua modules loaded via `require`, following standard Lua C module conventions:

```lua
-- Load plugin modules
local fs = require("spore.fs")
local ai = require("spore.ai")
local moss = require("spore.moss")

-- Each module exports a capability constructor
local home = fs.capability({ path = os.getenv("HOME"), mode = "r" })
local tmp = fs.capability({ path = "/tmp", mode = "rw" })

-- Capabilities are userdata with methods
local content = home:read("file.txt")
tmp:write("output.txt", content)

-- AI example
local claude = ai.capability({
    provider = "anthropic",
    model = "claude-3-5-sonnet",
    api_key = os.getenv("ANTHROPIC_API_KEY")
})
local response = claude:chat(messages)
local embedding = claude:embed(text)

-- Modules can also export utility functions that don't need capabilities
local exists = fs.exists("/some/path")  -- maybe some ops are safe without cap?
```

Benefits:
- **Namespacing**: `spore.fs`, `rhizome.moss`, `myorg.custom`
- **Familiar pattern**: standard Lua `require`
- **No central registry**: module path is the identity
- **Capability separation**: constructor vs methods
- **Fine-grained**: multiple capabilities with different permissions

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ Host (spore-lua)                                            │
│                                                             │
│  require("spore.fs")                                        │
│    │                                                        │
│    ├─► Lua require searcher finds spore.fs                  │
│    ├─► Loads libspore_fs.so via package.cpath               │
│    ├─► Calls luaopen_spore_fs(L)                            │
│    └─► Plugin returns module table                          │
│                                                             │
│  fs.capability({ path = "/tmp", mode = "rw" })              │
│    │                                                        │
│    ├─► Calls capability constructor in module               │
│    ├─► Creates userdata with params as user value           │
│    └─► Attaches metatable with methods                      │
│                                                             │
│  cap:read(path)                                             │
│    │                                                        │
│    └─► Calls fs_read(L) via metatable                       │
│          - self (capability userdata) at stack index 1      │
│          - path at stack index 2                            │
│          - Extracts params, validates, performs operation   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                           │
                           │ package.cpath / dlopen
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ Plugin (spore/fs.so or libspore_fs.so)                      │
│                                                             │
│  Standard Lua C module pattern                              │
│  Uses raw lua_State*, NOT mlua                              │
│                                                             │
│  Exports:                                                   │
│    luaopen_spore_fs(L) → module table                       │
│                                                             │
│  Module table contains:                                     │
│    - capability(params) → constructor function              │
│    - utility functions (optional)                           │
│                                                             │
│  Capability methods are Lua C functions:                    │
│    extern "C" fn fs_read(L: *mut lua_State) -> c_int        │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Plugin C ABI

Plugins are standard Lua C modules. The only spore-specific addition is optional metadata for version checking.

```c
#include <lua.h>

// Optional: plugin metadata for version/compat checking before luaopen_*
typedef struct {
    const char* name;           // "fs", "ai", "moss"
    const char* version;        // "0.1.0"
    uint32_t abi_version;       // for compatibility check
} SporePluginInfo;

// Optional export - host can check before loading
SporePluginInfo spore_plugin_info(void);

// Required: standard Lua C module entry point
// Returns module table with capability() constructor and utilities
int luaopen_spore_fs(lua_State* L);
```

Plugins define their own userdata types internally with their own metatables:

```
luaopen_spore_fs(L)
  └─► returns module table
        ├─► capability(params) → creates FsCapability userdata
        │     └─► metatable: read, write, list, open, attenuate, ...
        │           └─► open() → creates FsFile userdata
        │                 └─► metatable: read, write, seek, close, ...
        └─► utility functions (exists, join, etc.)
```

## Rust Plugin Implementation

```rust
// crates/plugins/spore-fs/src/lib.rs

use std::ffi::c_int;
use mlua::ffi::{self as lua, lua_State, luaL_Reg};

const ABI_VERSION: u32 = 1;

// Optional: metadata export
#[no_mangle]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
        name: c"fs".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

// Required: standard Lua C module entry point
#[no_mangle]
pub unsafe extern "C" fn luaopen_spore_fs(L: *mut lua_State) -> c_int {
    // Register capability metatable
    register_capability_metatable(L);
    register_file_metatable(L);

    // Create module table
    lua::lua_newtable(L);

    // Add capability constructor
    lua::lua_pushcfunction(L, Some(fs_capability));
    lua::lua_setfield(L, -2, c"capability".as_ptr());

    // Add utility functions
    lua::lua_pushcfunction(L, Some(fs_exists));
    lua::lua_setfield(L, -2, c"exists".as_ptr());

    1  // return module table
}

// Capability metatable methods
static CAP_METHODS: &[luaL_Reg] = &[
    luaL_Reg { name: c"read".as_ptr(), func: Some(fs_cap_read) },
    luaL_Reg { name: c"write".as_ptr(), func: Some(fs_cap_write) },
    luaL_Reg { name: c"open".as_ptr(), func: Some(fs_cap_open) },
    luaL_Reg { name: c"list".as_ptr(), func: Some(fs_cap_list) },
    luaL_Reg { name: c"attenuate".as_ptr(), func: Some(fs_cap_attenuate) },
    luaL_Reg { name: std::ptr::null(), func: None },
];

// File handle metatable methods
static FILE_METHODS: &[luaL_Reg] = &[
    luaL_Reg { name: c"read".as_ptr(), func: Some(fs_file_read) },
    luaL_Reg { name: c"write".as_ptr(), func: Some(fs_file_write) },
    luaL_Reg { name: c"seek".as_ptr(), func: Some(fs_file_seek) },
    luaL_Reg { name: c"close".as_ptr(), func: Some(fs_file_close) },
    luaL_Reg { name: std::ptr::null(), func: None },
];

unsafe fn register_capability_metatable(L: *mut lua_State) {
    luaL_newmetatable(L, c"spore.fs.Capability".as_ptr());
    lua::lua_newtable(L);  // __index table
    luaL_setfuncs(L, CAP_METHODS.as_ptr(), 0);
    lua::lua_setfield(L, -2, c"__index".as_ptr());
    lua::lua_pushcfunction(L, Some(fs_cap_gc));
    lua::lua_setfield(L, -2, c"__gc".as_ptr());
    lua::lua_pop(L, 1);
}

// fs.capability({ path = "/tmp", mode = "rw" }) -> FsCapability userdata
unsafe extern "C" fn fs_capability(L: *mut lua_State) -> c_int {
    // Validate params table at index 1
    // Create userdata, store params as user value, set metatable
    // ...
    1
}

// Helper: get params from capability userdata at index 1
unsafe fn get_capability_params(L: *mut lua_State) -> Result<serde_json::Value, String> {
    // Get user value from userdata at index 1
    lua::lua_getuservalue(L, 1);

    // Convert Lua table to JSON (helper function)
    let json = lua_table_to_json(L, -1)?;
    lua::lua_pop(L, 1);

    Ok(json)
}

// Helper: validate path stays within capability root
fn validate_path(params: &serde_json::Value, rel_path: &str) -> Result<std::path::PathBuf, String> {
    let root = params["path"].as_str().ok_or("capability missing 'path' param")?;
    let mode = params["mode"].as_str().unwrap_or("r");

    let root_path = std::path::Path::new(root);
    let full_path = root_path.join(rel_path);

    // Canonicalize to resolve .. and symlinks
    let canonical = full_path.canonicalize()
        .map_err(|e| format!("invalid path: {}", e))?;

    // Ensure path doesn't escape root
    let canonical_root = root_path.canonicalize()
        .map_err(|e| format!("invalid root: {}", e))?;

    if !canonical.starts_with(&canonical_root) {
        return Err("path escapes capability root".into());
    }

    Ok(canonical)
}

// Helper: push error and return lua_error result
unsafe fn lua_push_error(L: *mut lua_State, msg: &str) -> c_int {
    let c_msg = std::ffi::CString::new(msg).unwrap_or_else(|_| c"error".into());
    lua::lua_pushstring(L, c_msg.as_ptr());
    lua::lua_error(L)
}

// fs:read(path) -> string
#[no_mangle]
pub unsafe extern "C" fn fs_read(L: *mut lua_State) -> c_int {
    // Arg 1: self (capability userdata)
    // Arg 2: path (string)

    let params = match get_capability_params(L) {
        Ok(p) => p,
        Err(e) => return lua_push_error(L, &e),
    };

    // Get path argument
    let mut len = 0;
    let path_ptr = lua::lua_tolstring(L, 2, &mut len);
    if path_ptr.is_null() {
        return lua_push_error(L, "fs:read requires path argument");
    }
    let path = std::str::from_utf8(std::slice::from_raw_parts(path_ptr as *const u8, len))
        .map_err(|_| "invalid UTF-8")
        .unwrap();

    // Validate path is within capability
    let full_path = match validate_path(&params, path) {
        Ok(p) => p,
        Err(e) => return lua_push_error(L, &e),
    };

    // Read file
    match std::fs::read_to_string(&full_path) {
        Ok(content) => {
            let c_content = std::ffi::CString::new(content).unwrap();
            lua::lua_pushstring(L, c_content.as_ptr());
            1  // return 1 value
        }
        Err(e) => lua_push_error(L, &format!("read failed: {}", e)),
    }
}

// fs:write(path, content)
#[no_mangle]
pub unsafe extern "C" fn fs_write(L: *mut lua_State) -> c_int {
    let params = match get_capability_params(L) {
        Ok(p) => p,
        Err(e) => return lua_push_error(L, &e),
    };

    // Check write permission
    let mode = params["mode"].as_str().unwrap_or("r");
    if !mode.contains('w') {
        return lua_push_error(L, "capability does not permit writes");
    }

    // Get path and content arguments
    // ... similar to fs_read

    0  // return nothing on success
}

// ... other methods
```

## Host Implementation

```rust
// In spore-lua crate

use libloading::{Library, Symbol};
use mlua::{Lua, UserData, UserDataMethods, Table, Result};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct PluginLoader {
    plugins: HashMap<String, LoadedPlugin>,
    search_paths: Vec<PathBuf>,
}

struct LoadedPlugin {
    _library: Library,  // kept alive
    info: PluginInfo,
    methods: Vec<MethodDef>,
}

struct PluginInfo {
    name: String,
    version: String,
    abi_version: u32,
}

struct MethodDef {
    name: String,
    func: mlua::ffi::lua_CFunction,
}

const ABI_VERSION: u32 = 1;

impl PluginLoader {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            search_paths: vec![
                dirs::home_dir().unwrap().join(".spore/plugins"),
                PathBuf::from("/usr/lib/spore/plugins"),
            ],
        }
    }

    fn find_plugin_path(&self, name: &str) -> Option<PathBuf> {
        let lib_name = format!("libspore_{}.so", name);  // platform-specific
        for dir in &self.search_paths {
            let path = dir.join(&lib_name);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    pub fn load(&mut self, name: &str) -> Result<&LoadedPlugin, String> {
        if self.plugins.contains_key(name) {
            return Ok(&self.plugins[name]);
        }

        let path = self.find_plugin_path(name)
            .ok_or_else(|| format!("plugin not found: {}", name))?;

        let lib = unsafe { Library::new(&path) }
            .map_err(|e| format!("failed to load plugin: {}", e))?;

        // Get plugin info
        let info_fn: Symbol<extern "C" fn() -> SporePluginInfo> =
            unsafe { lib.get(b"spore_plugin_info") }
            .map_err(|_| "plugin missing spore_plugin_info")?;

        let raw_info = info_fn();

        // Check ABI version
        if raw_info.abi_version != ABI_VERSION {
            return Err(format!(
                "ABI mismatch: plugin has {}, host has {}",
                raw_info.abi_version, ABI_VERSION
            ));
        }

        let info = PluginInfo {
            name: unsafe { std::ffi::CStr::from_ptr(raw_info.name) }
                .to_string_lossy().into(),
            version: unsafe { std::ffi::CStr::from_ptr(raw_info.version) }
                .to_string_lossy().into(),
            abi_version: raw_info.abi_version,
        };

        // Get methods
        let methods_fn: Symbol<extern "C" fn() -> *const SporeMethod> =
            unsafe { lib.get(b"spore_plugin_methods") }
            .map_err(|_| "plugin missing spore_plugin_methods")?;

        let methods_ptr = methods_fn();
        let methods = parse_methods(methods_ptr);

        self.plugins.insert(name.to_string(), LoadedPlugin {
            _library: lib,
            info,
            methods,
        });

        Ok(&self.plugins[name])
    }

    /// Create a capability userdata for the given plugin
    pub fn create_capability(
        &mut self,
        lua: &Lua,
        plugin_name: &str,
        params: Table,
    ) -> mlua::Result<mlua::AnyUserData> {
        let plugin = self.load(plugin_name)
            .map_err(mlua::Error::external)?;

        // Create userdata
        struct CapabilityData {
            plugin_name: String,
        }

        impl UserData for CapabilityData {}

        let cap = lua.create_userdata(CapabilityData {
            plugin_name: plugin_name.to_string(),
        })?;

        // Store params as user value
        cap.set_user_value(params)?;

        // Create metatable with methods
        let mt = lua.create_table()?;
        let index = lua.create_table()?;

        for method in &plugin.methods {
            // Convert raw C function to mlua function
            let func = unsafe { lua.create_c_function(method.func)? };
            index.set(method.name.as_str(), func)?;
        }

        mt.set("__index", index)?;
        cap.set_metatable(Some(mt));

        Ok(cap)
    }
}

/// Register spore.capability() function
pub fn register_capability_api(lua: &Lua, loader: std::sync::Arc<std::sync::Mutex<PluginLoader>>) -> mlua::Result<()> {
    let spore = lua.globals().get::<Table>("spore")?;

    let cap_fn = lua.create_function(move |lua, args: (String, Table)| {
        let (plugin_name, params) = args;
        let mut loader = loader.lock().unwrap();
        loader.create_capability(lua, &plugin_name, params)
    })?;

    spore.set("capability", cap_fn)?;

    Ok(())
}
```

## Plugin Discovery

Search order:
1. Explicit path in config
2. Project-local: `.spore/plugins/`
3. User-local: `~/.spore/plugins/`
4. System: `/usr/lib/spore/plugins/`

Naming convention:
- Linux: `libspore_{name}.so`
- macOS: `libspore_{name}.dylib`
- Windows: `spore_{name}.dll`

## Config

```toml
# .spore/config.toml

[plugins]
# Enable plugin (loaded from search paths)
fs = true
ai = true

# Explicit path
moss = { path = "/opt/spore/plugins/libspore_moss.so" }

# Disabled
experimental = false
```

## Safety Considerations

### Memory
- Plugin uses Lua C API directly - follows Lua memory model
- Capability userdata owned by Lua GC
- Plugin must not store lua_State pointers beyond function call

### Panics
- Panics across FFI are UB
- Plugin functions should use catch_unwind at boundary
- Convert panics to Lua errors

### Thread Safety
- lua_State is not thread-safe
- Plugin functions called from Lua thread only
- Async operations need careful handling

## Implementation Plan

### Phase 1: Plugin Loader
- Create `spore-plugin-loader` crate (or add to `spore-lua`)
- Implement PluginLoader struct
- Implement spore.capability() function
- Define C ABI types

### Phase 2: First Plugin
- Create `spore-fs` plugin as cdylib
- Implement basic fs operations (read, write, list, exists)
- Test loading and calling

### Phase 3: Convert Existing Integrations
- Convert spore-llm to plugin
- Convert spore-moss to plugin
- Convert spore-moss-tools to plugin
- Convert spore-moss-packages to plugin

### Phase 4: Remove Compiled-In
- Remove integration dependencies from spore binary
- Update config format
- Update documentation

## Capability Enforcement

The design above shows scripts calling `fs.capability()` directly - but that's not security, just namespacing. Any script could request root access. True capability-based security requires **capabilities to be injected, not created by untrusted code**.

### The Problem

```lua
-- Untrusted script - should NOT be allowed to do this:
local root_fs = fs.capability({ path = "/", mode = "rw" })
root_fs:write("/etc/passwd", "hacked")
```

### Solution: Capability Injection

Scripts don't create capabilities - they receive them. The trusted host (spore runtime) creates capabilities based on policy, then passes them to scripts:

```lua
-- Host (trusted) creates capabilities before running script
local caps = {
    fs = {
        project = fs.capability({ path = "/project", mode = "rw" }),
        data = fs.capability({ path = "/data/datasets", mode = "r" }),
        tmp = fs.capability({ path = "/tmp", mode = "rw" }),
    },
    ai = {
        claude = ai.capability({ provider = "anthropic", model = "claude-3-5-sonnet" }),
    },
}

-- Script is invoked with capabilities as arguments
runtime:call("agent.lua", caps)
```

```lua
-- agent.lua (untrusted)
local function main(caps)
    -- Can only use what was given
    local content = caps.fs.project:read("src/main.rs")  -- OK
    caps.fs.project:write("build/out.txt", content)      -- OK
    local data = caps.fs.data:read("training.csv")       -- OK (read-only)

    -- Cannot escalate
    local root = fs.capability({ path = "/" })   -- ERROR: fs not in scope
    caps.fs.data:write("x.txt", "y")             -- ERROR: data cap is read-only
end
return { main = main }
```

### Implementation

**1. Remove constructors from script environment**

The `require("spore.fs")` module is NOT available to scripts. Only the host has access.

```rust
impl Runtime {
    /// Load a plugin module (host-only, not exposed to scripts)
    pub fn load_plugin(&mut self, name: &str) -> Result<PluginModule> {
        // ...
    }

    /// Create capability (host-only)
    pub fn create_capability(&self, plugin: &PluginModule, params: Table) -> Result<Capability> {
        plugin.create_capability(params)
    }

    /// Run script with injected capabilities
    pub fn run(&self, script: &str, caps: HashMap<String, Capability>) -> Result<Value> {
        let lua = &self.lua;

        // Create sandbox environment WITHOUT plugin access
        let env = lua.create_table()?;
        env.set("print", lua.globals().get::<Function>("print")?)?;
        env.set("pairs", lua.globals().get::<Function>("pairs")?)?;
        // ... other safe builtins

        // Inject capabilities
        let caps_table = lua.create_table()?;
        for (name, cap) in caps {
            caps_table.set(name, cap)?;
        }
        env.set("caps", caps_table)?;

        // Load and run script in sandbox
        let chunk = lua.load(script).set_environment(env);
        chunk.exec()?;
        // ...
    }
}
```

**2. Policy-based capability creation**

Host reads policy from config:

```toml
# .spore/policy.toml

[agent.caps.fs]
project = { path = "${PROJECT_ROOT}", mode = "rw" }
data = { path = "/data/datasets", mode = "r" }
tmp = { path = "/tmp/spore-${AGENT_ID}", mode = "rw" }

[agent.caps.ai]
claude = { provider = "anthropic", model = "claude-3-5-sonnet" }
gpt = { provider = "openai", model = "gpt-4o" }

# Network disabled by default
# [agent.caps.net]
# api = { allow = ["api.anthropic.com", "api.openai.com"] }
```

```rust
fn create_caps_from_policy(runtime: &Runtime, policy: &Policy) -> HashMap<String, Capability> {
    let mut caps = HashMap::new();

    for (name, spec) in &policy.caps {
        let plugin = runtime.load_plugin(&spec.plugin)?;
        let params = expand_params(&spec.params);  // ${PROJECT_ROOT} -> actual path
        caps.insert(name, runtime.create_capability(&plugin, params)?);
    }

    caps
}
```

**3. Capability attenuation (plugin-implemented)**

Plugins implement attenuation as a method on the capability userdata. No host involvement - pure plugin logic:

```lua
local function main(caps)
    -- Attenuate: create restricted sub-capability
    local readonly = caps.fs.project:attenuate({ mode = "r" })
    local subdir = caps.fs.project:attenuate({ path = "src/", mode = "r" })

    -- Pass reduced capability to untrusted submodule
    untrusted_analyzer.run(subdir)
end
```

Plugin implementation:

```rust
// "attenuate" is a method on the capability userdata's metatable
// (plugins can define multiple userdata types, each with own metatable)
unsafe extern "C" fn fs_cap_attenuate(L: *mut lua_State) -> c_int {
    let orig = get_capability_params(L, 1)?;  // self
    let restrictions = get_table_arg(L, 2)?;

    // Validate: can only narrow, never expand
    if let Some(new_path) = restrictions["path"].as_str() {
        let orig_path = orig["path"].as_str().unwrap();
        let new_abs = Path::new(orig_path).join(new_path).canonicalize()?;
        let orig_abs = Path::new(orig_path).canonicalize()?;
        if !new_abs.starts_with(&orig_abs) {
            return lua_push_error(L, "path escapes capability root");
        }
    }

    if let Some(new_mode) = restrictions["mode"].as_str() {
        let orig_mode = orig["mode"].as_str().unwrap_or("r");
        if !is_mode_subset(new_mode, orig_mode) {
            return lua_push_error(L, "cannot expand mode");
        }
    }

    // Create NEW capability with merged params (original unchanged)
    let merged = merge_params(&orig, &restrictions);
    push_capability_userdata(L, merged);
    1
}
```

### Summary

| Component | Access |
|-----------|--------|
| Host (spore binary) | Full - loads plugins, creates capabilities |
| Policy file | Defines what capabilities to create |
| Script entry point | Receives capabilities as arguments |
| Script code | Uses capabilities, can attenuate |
| Submodules | Receive attenuated capabilities |

This is real capability-based security:
- **Unforgeable**: Scripts can't create capabilities from nothing
- **Transferable**: Capabilities can be passed to submodules
- **Attenuable**: Can create restricted sub-capabilities
- **Revocable**: Host can revoke (see open questions)

## Open Questions

### 1. Async Operations
AI operations are async (network calls). Options:
- Block in plugin (simple, but blocks Lua)
- Return future/promise userdata
- Callback-based API
- Lua coroutine integration

### 2. Capability Inheritance
Should capabilities be able to derive restricted sub-capabilities?
```lua
local fs = spore.capability("fs", {path = "/project", mode = "rw"})
local readonly_fs = fs:restrict({mode = "r"})  -- derived capability
```

### 3. Capability Revocation
Can capabilities be revoked after creation?
```lua
local fs = spore.capability("fs", {...})
fs:revoke()  -- subsequent calls fail
```

### 4. LuaJIT FFI vs C ABI
Current design uses C ABI (libloading + Lua C API). LuaJIT FFI would reduce boilerplate and potentially improve performance, but:
- Locks us to LuaJIT (development uncertain, though OpenResty fork exists)
- FFI exposed to scripts would break capability security (direct C access bypasses sandbox)
- Could use FFI internally (host-side only) while keeping C ABI for plugin boundary

Worth revisiting if there are performance-critical paths where host-side FFI would help.

## Build System

Plugin Cargo.toml:
```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
# Only need raw Lua FFI, not full mlua
mlua = { version = "0.10", features = ["luajit"], default-features = false }
serde_json = "1"
```

Nix packaging:
- Each plugin is separate derivation
- `spore-full` bundles common plugins
- Plugins declare compatible ABI version
