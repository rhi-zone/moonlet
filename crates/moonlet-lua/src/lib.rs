//! moonlet-lua: Lua runtime for moonlet agents.
//!
//! This crate provides the Lua execution environment for agent scripts,
//! with support for dynamic plugins and integration modules.

pub mod handle;
pub mod plugin;

pub use handle::{Handle, HandleItem, HandleResult, Stream, push_handle, spawn_subprocess};
use mlua::{Function, Lua, Result, Table, Value};
pub use plugin::{ABI_VERSION, PluginInfo, PluginLoader};
use std::path::PathBuf;

/// Configuration for the sandboxed require function.
#[derive(Debug, Clone, Default)]
pub struct RequireConfig {
    /// Allow require() for Lua builtins (string, table, math, etc.)
    /// These always take precedence and cannot be overridden.
    pub builtins: bool,
    /// Allow require() for loaded moonlet plugins (e.g., require("moonlet.sessions"))
    pub plugins: bool,
    /// Allow require() for project Lua modules (relative to project root)
    pub project: bool,
    /// Project root directory for loading project modules
    pub project_root: Option<PathBuf>,
}

/// Trait for registering integration modules into the Lua runtime.
pub trait Integration {
    /// Register this integration's functions into the Lua global scope.
    fn register(&self, lua: &Lua) -> Result<()>;
}

/// The moonlet Lua runtime.
pub struct Runtime {
    lua: Lua,
    plugins: PluginLoader,
}

impl Runtime {
    /// Create a new Lua runtime with moonlet-core bindings.
    pub fn new() -> Result<Self> {
        let lua = Lua::new();

        // Register moonlet-core bindings (LLM, memory)
        register_core(&lua)?;

        Ok(Self {
            lua,
            plugins: PluginLoader::new(),
        })
    }

    /// Register an integration plugin (legacy static integration).
    pub fn register<I: Integration>(&self, integration: &I) -> Result<()> {
        integration.register(&self.lua)
    }

    /// Add a custom plugin search path.
    pub fn add_plugin_path(&mut self, path: PathBuf) {
        self.plugins.add_search_path(path);
    }

    /// Load a dynamic plugin by name.
    ///
    /// Searches for the plugin library in configured search paths and loads it.
    /// The plugin's metatables are registered but not exposed to scripts.
    pub fn load_plugin(&mut self, name: &str) -> Result<()> {
        self.plugins.load(&self.lua, name)
    }

    /// Check if a plugin is loaded.
    pub fn is_plugin_loaded(&self, name: &str) -> bool {
        self.plugins.is_loaded(name)
    }

    /// Create a capability from a loaded plugin.
    ///
    /// The params table contains plugin-specific configuration.
    /// For example, fs plugin expects `{ path = "...", mode = "r|w|rw" }`.
    pub fn create_capability(&self, plugin_name: &str, params: Table) -> Result<Value> {
        self.plugins
            .create_capability(&self.lua, plugin_name, params)
    }

    /// Get a loaded plugin's module table.
    pub fn get_plugin_module(&self, name: &str) -> Result<Table> {
        self.plugins.get_module(&self.lua, name)
    }

    /// Get list of loaded plugin names.
    pub fn loaded_plugins(&self) -> Vec<&str> {
        self.plugins.loaded_plugins()
    }

    /// Run a Lua script from a string.
    pub fn run(&self, code: &str) -> Result<()> {
        self.lua.load(code).exec()
    }

    /// Run a Lua script from a file.
    pub fn run_file(&self, path: &std::path::Path) -> Result<()> {
        let code = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
        self.run(&code)
    }

    /// Run a Lua script with injected capabilities and restricted require.
    ///
    /// The capabilities are available as `caps.{name}` in the script.
    /// This creates a sandboxed environment with a restricted require function.
    pub fn run_with_caps(
        &self,
        code: &str,
        caps: Table,
        require_config: &RequireConfig,
    ) -> Result<()> {
        // Create sandboxed environment
        let env = self.lua.create_table()?;

        // Copy safe globals
        let globals = self.lua.globals();
        for name in &[
            "print",
            "pairs",
            "ipairs",
            "next",
            "type",
            "tostring",
            "tonumber",
            "error",
            "assert",
            "pcall",
            "xpcall",
            "select",
            "unpack",
            "setmetatable",
            "getmetatable",
            "rawget",
            "rawset",
            "rawequal",
            "string",
            "table",
            "math",
            "os",
            "coroutine",
        ] {
            if let Ok(val) = globals.get::<Value>(*name) {
                env.set(*name, val)?;
            }
        }

        // Inject capabilities
        env.set("caps", caps)?;

        // Also inject moonlet table if it exists
        if let Ok(moonlet) = globals.get::<Table>("moonlet") {
            env.set("moonlet", moonlet)?;
        }

        // Create and inject restricted require function
        let require_fn = self.create_restricted_require(require_config)?;
        env.set("require", require_fn)?;

        // Run in sandboxed environment
        self.lua.load(code).set_environment(env).exec()
    }

    /// Create a restricted require function based on configuration.
    fn create_restricted_require(&self, config: &RequireConfig) -> Result<Function> {
        let lua = &self.lua;

        // Create a table to cache loaded modules
        let loaded: Table = lua.create_table()?;

        // Pre-populate with builtins if enabled (these always take precedence)
        if config.builtins {
            let globals = lua.globals();
            for name in &[
                "string",
                "table",
                "math",
                "os",
                "coroutine",
                "utf8",
                "debug",
            ] {
                if let Ok(val) = globals.get::<Value>(*name) {
                    loaded.set(*name, val)?;
                }
            }
        }

        // Pre-populate with loaded plugins if enabled
        if config.plugins {
            for plugin_name in self.plugins.loaded_plugins() {
                if let Ok(module) = self.plugins.get_module(lua, plugin_name) {
                    let full_name = format!("moonlet.{}", plugin_name);
                    loaded.set(full_name.as_str(), module)?;
                }
            }
        }

        // Store the loaded table in the registry so the closure can access it
        lua.set_named_registry_value("_SPORE_LOADED", loaded)?;

        // Store config values for the closure
        let allow_builtins = config.builtins;
        let allow_plugins = config.plugins;
        let allow_project = config.project;
        let project_root = config.project_root.clone();

        // Create the require function
        lua.create_function(move |lua, name: String| {
            // Get the loaded modules cache from registry
            let loaded: Table = lua.named_registry_value("_SPORE_LOADED")?;

            // First check if already loaded/cached
            if let Ok(cached) = loaded.get::<Value>(name.as_str())
                && cached != Value::Nil
            {
                return Ok(cached);
            }

            // Check builtins (always take precedence, even if allow_builtins is false
            // we still check to give a better error message)
            let builtins = [
                "string",
                "table",
                "math",
                "os",
                "coroutine",
                "utf8",
                "debug",
            ];
            if builtins.contains(&name.as_str()) {
                if !allow_builtins {
                    return Err(mlua::Error::external(format!(
                        "require('{}') is disabled: builtins not allowed",
                        name
                    )));
                }
                let globals = lua.globals();
                if let Ok(val) = globals.get::<Value>(name.as_str()) {
                    loaded.set(name.as_str(), val.clone())?;
                    return Ok(val);
                }
            }

            // Check for moonlet.* plugins
            if name.starts_with("moonlet.") {
                if !allow_plugins {
                    return Err(mlua::Error::external(format!(
                        "require('{}') is disabled: plugins not allowed",
                        name
                    )));
                }
                // Plugin should already be in loaded table if it exists
                return Err(mlua::Error::external(format!(
                    "module '{}' not found (plugin not loaded)",
                    name
                )));
            }

            // Check project modules
            if !allow_project {
                return Err(mlua::Error::external(format!(
                    "require('{}') is disabled: project modules not allowed",
                    name
                )));
            }

            let Some(ref root) = project_root else {
                return Err(mlua::Error::external(format!(
                    "require('{}') failed: no project root configured",
                    name
                )));
            };

            // Convert module name to path (foo.bar -> foo/bar.lua)
            let rel_path = name.replace('.', "/") + ".lua";
            let full_path = root.join(&rel_path);

            // Security: ensure the resolved path is within project root
            let canonical = full_path.canonicalize().map_err(|e| {
                mlua::Error::external(format!("module '{}' not found: {}", name, e))
            })?;
            let canonical_root = root
                .canonicalize()
                .map_err(|e| mlua::Error::external(format!("invalid project root: {}", e)))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(mlua::Error::external(format!(
                    "require('{}') denied: path escapes project root",
                    name
                )));
            }

            // Load and execute the module
            let code = std::fs::read_to_string(&canonical).map_err(|e| {
                mlua::Error::external(format!("module '{}' not found: {}", name, e))
            })?;

            let module: Value = lua.load(&code).set_name(&name).eval()?;

            // Cache the result (use true if module returned nil)
            let cache_val = if module == Value::Nil {
                Value::Boolean(true)
            } else {
                module.clone()
            };
            loaded.set(name.as_str(), cache_val)?;

            Ok(module)
        })
    }

    /// Get a reference to the underlying Lua state.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new().expect("failed to create Lua runtime")
    }
}

/// Register moonlet-core bindings (LLM, memory) into Lua.
fn register_core(lua: &Lua) -> Result<()> {
    let moonlet = lua.create_table()?;

    // TODO: Register LlmClient bindings
    // TODO: Register MemoryStore bindings

    lua.globals().set("moonlet", moonlet)?;

    // Register Handle metatable and poll functions using raw Lua C API
    // Safety: exec_raw provides safe access to the raw lua_State
    unsafe {
        lua.exec_raw::<()>((), |state| {
            handle::register_handle_metatable(state);
            handle::register_poll_functions(state);
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_new() {
        let runtime = Runtime::new().expect("should create runtime");
        // Verify moonlet global exists
        let moonlet: Table = runtime
            .lua()
            .globals()
            .get("moonlet")
            .expect("moonlet should exist");
        assert!(moonlet.contains_key("poll").unwrap());
    }

    #[test]
    fn test_require_builtins() {
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: true,
            plugins: false,
            project: false,
            project_root: None,
        };

        let code = r#"
            local str = require("string")
            assert(str.upper("hello") == "HELLO", "string.upper should work")
            return true
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should run with require");
    }

    #[test]
    fn test_require_builtins_disabled() {
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: false,
            plugins: false,
            project: false,
            project_root: None,
        };

        let code = r#"
            local ok, err = pcall(function() require("string") end)
            assert(not ok, "require should fail when builtins disabled")
            local err_str = tostring(err)
            assert(string.find(err_str, "builtins not allowed"), "error should mention builtins: " .. err_str)
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should run");
    }

    #[test]
    fn test_require_plugins_disabled() {
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: true,
            plugins: false,
            project: false,
            project_root: None,
        };

        let code = r#"
            local ok, err = pcall(function() require("moonlet.sessions") end)
            assert(not ok, "require should fail when plugins disabled")
            local err_str = tostring(err)
            assert(string.find(err_str, "plugins not allowed"), "error should mention plugins: " .. err_str)
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should run");
    }

    #[test]
    fn test_require_project_disabled() {
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: true,
            plugins: true,
            project: false,
            project_root: None,
        };

        let code = r#"
            local ok, err = pcall(function() require("mymodule") end)
            assert(not ok, "require should fail when project disabled")
            local err_str = tostring(err)
            assert(string.find(err_str, "project modules not allowed"), "error should mention project: " .. err_str)
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should run");
    }

    #[test]
    fn test_require_caches_modules() {
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: true,
            plugins: false,
            project: false,
            project_root: None,
        };

        let code = r#"
            local str1 = require("string")
            local str2 = require("string")
            assert(str1 == str2, "require should return cached module")
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should run with caching");
    }

    #[test]
    fn test_require_builtins_always_precedence() {
        // Even with project modules enabled, builtins should win
        let runtime = Runtime::new().expect("should create runtime");
        let caps = runtime.lua().create_table().expect("should create caps");
        let config = RequireConfig {
            builtins: true,
            plugins: true,
            project: true,
            project_root: Some(PathBuf::from("/tmp")),
        };

        let code = r#"
            -- Even if there was a string.lua in project, builtin should win
            local str = require("string")
            assert(type(str.upper) == "function", "builtin string should be used")
        "#;

        runtime
            .run_with_caps(code, caps, &config)
            .expect("should use builtin");
    }
}
