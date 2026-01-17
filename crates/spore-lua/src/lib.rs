//! rhizome-spore-lua: Lua runtime for spore agents.
//!
//! This crate provides the Lua execution environment for agent scripts,
//! with support for dynamic plugins and integration modules.

pub mod plugin;

use mlua::{Lua, Result, Table, Value};
pub use plugin::{ABI_VERSION, PluginInfo, PluginLoader};
use std::path::PathBuf;

/// Trait for registering integration modules into the Lua runtime.
pub trait Integration {
    /// Register this integration's functions into the Lua global scope.
    fn register(&self, lua: &Lua) -> Result<()>;
}

/// The spore Lua runtime.
pub struct Runtime {
    lua: Lua,
    plugins: PluginLoader,
}

impl Runtime {
    /// Create a new Lua runtime with spore-core bindings.
    pub fn new() -> Result<Self> {
        let lua = Lua::new();

        // Register spore-core bindings (LLM, memory)
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

    /// Run a Lua script from a string.
    pub fn run(&self, code: &str) -> Result<()> {
        self.lua.load(code).exec()
    }

    /// Run a Lua script from a file.
    pub fn run_file(&self, path: &std::path::Path) -> Result<()> {
        let code = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
        self.run(&code)
    }

    /// Run a Lua script with injected capabilities.
    ///
    /// The capabilities are available as `caps.{name}` in the script.
    /// This creates a sandboxed environment without access to plugin constructors.
    pub fn run_with_caps(&self, code: &str, caps: Table) -> Result<()> {
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

        // Also inject spore table if it exists
        if let Ok(spore) = globals.get::<Table>("spore") {
            env.set("spore", spore)?;
        }

        // Run in sandboxed environment
        self.lua.load(code).set_environment(env).exec()
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

/// Register spore-core bindings (LLM, memory) into Lua.
fn register_core(lua: &Lua) -> Result<()> {
    let spore = lua.create_table()?;

    // TODO: Register LlmClient bindings
    // TODO: Register MemoryStore bindings

    lua.globals().set("spore", spore)?;
    Ok(())
}
