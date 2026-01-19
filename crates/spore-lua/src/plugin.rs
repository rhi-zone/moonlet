//! Dynamic plugin loading for spore.
//!
//! Plugins are dynamic libraries that export Lua C functions and can create
//! capability userdata. The host loads plugins and creates capabilities based
//! on policy configuration.

use libloading::{Library, Symbol};
use mlua::{Lua, RegistryKey, Result as LuaResult, Table, Value};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int};
use std::path::PathBuf;

/// Plugin metadata returned by spore_plugin_info().
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

/// Current ABI version. Plugins must match this to load.
pub const ABI_VERSION: u32 = 1;

/// A loaded plugin.
struct LoadedPlugin {
    /// The dynamic library handle. Kept alive to prevent unloading.
    _library: Library,
    /// Plugin name.
    #[allow(dead_code)]
    name: String,
    /// Plugin version string.
    #[allow(dead_code)]
    version: String,
    /// Registry key for the module table returned by luaopen.
    module_key: RegistryKey,
}

/// Plugin loader and registry.
pub struct PluginLoader {
    /// Loaded plugins by name.
    plugins: HashMap<String, LoadedPlugin>,
    /// Search paths for plugin libraries.
    search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    /// Create a new plugin loader with default search paths.
    pub fn new() -> Self {
        let mut search_paths = Vec::new();

        // Project-local plugins
        search_paths.push(PathBuf::from(".spore/plugins"));

        // User-local plugins
        if let Some(home) = dirs::home_dir() {
            search_paths.push(home.join(".spore/plugins"));
        }

        // System plugins (Linux)
        search_paths.push(PathBuf::from("/usr/lib/spore/plugins"));
        search_paths.push(PathBuf::from("/usr/local/lib/spore/plugins"));

        Self {
            plugins: HashMap::new(),
            search_paths,
        }
    }

    /// Add a custom search path.
    pub fn add_search_path(&mut self, path: PathBuf) {
        self.search_paths.insert(0, path); // Prepend for priority
    }

    /// Find a plugin library by name.
    fn find_plugin_path(&self, name: &str) -> Option<PathBuf> {
        let lib_name = library_filename(name);

        for dir in &self.search_paths {
            let path = dir.join(&lib_name);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    /// Load a plugin by name.
    ///
    /// This finds the plugin library, loads it, verifies ABI compatibility,
    /// and calls luaopen_spore_{name} to register it with Lua.
    pub fn load(&mut self, lua: &Lua, name: &str) -> LuaResult<()> {
        if self.plugins.contains_key(name) {
            return Ok(()); // Already loaded
        }

        let path = self
            .find_plugin_path(name)
            .ok_or_else(|| mlua::Error::external(format!("plugin not found: {}", name)))?;

        // Load the dynamic library
        let library = unsafe { Library::new(&path) }
            .map_err(|e| mlua::Error::external(format!("failed to load plugin {}: {}", name, e)))?;

        // Check for plugin info (optional but recommended)
        let (plugin_name, version) = self.get_plugin_info(&library, name)?;

        // Get the luaopen function
        let open_fn_name = format!("luaopen_spore_{}", name);
        let open_fn: Symbol<unsafe extern "C" fn(*mut mlua::ffi::lua_State) -> c_int> =
            unsafe { library.get(open_fn_name.as_bytes()) }.map_err(|e| {
                mlua::Error::external(format!("plugin {} missing {}: {}", name, open_fn_name, e))
            })?;

        // Call the luaopen function to register the plugin
        // We use exec_raw to safely access the raw lua_State
        let open_fn = *open_fn; // Copy the function pointer
        let module: Table = unsafe {
            lua.exec_raw((), |state| {
                let result = open_fn(state);
                // If the function returned nothing, push nil
                if result == 0 {
                    mlua::ffi::lua_pushnil(state);
                }
                // exec_raw will retrieve the top stack value as our return
            })?
        };

        // Store the module table in the registry for later capability creation
        let module_key = lua.create_registry_value(module)?;

        self.plugins.insert(
            name.to_string(),
            LoadedPlugin {
                _library: library,
                name: plugin_name,
                version,
                module_key,
            },
        );

        Ok(())
    }

    /// Get plugin info if available.
    fn get_plugin_info(&self, library: &Library, name: &str) -> LuaResult<(String, String)> {
        let info_fn: Result<Symbol<unsafe extern "C" fn() -> PluginInfo>, _> =
            unsafe { library.get(b"spore_plugin_info") };

        match info_fn {
            Ok(info_fn) => {
                let info = unsafe { info_fn() };

                // Check ABI version
                if info.abi_version != ABI_VERSION {
                    return Err(mlua::Error::external(format!(
                        "plugin {} ABI mismatch: plugin has {}, host has {}",
                        name, info.abi_version, ABI_VERSION
                    )));
                }

                let plugin_name = unsafe { CStr::from_ptr(info.name) }
                    .to_string_lossy()
                    .into_owned();
                let version = unsafe { CStr::from_ptr(info.version) }
                    .to_string_lossy()
                    .into_owned();

                Ok((plugin_name, version))
            }
            Err(_) => {
                // No plugin info - use defaults
                Ok((name.to_string(), "unknown".to_string()))
            }
        }
    }

    /// Check if a plugin is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Get loaded plugin names.
    pub fn loaded_plugins(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Unload a plugin.
    ///
    /// Note: This only removes the plugin from the registry. The library
    /// may still be in use if Lua has references to its functions.
    pub fn unload(&mut self, name: &str) -> LuaResult<()> {
        self.plugins
            .remove(name)
            .ok_or_else(|| mlua::Error::external(format!("plugin not loaded: {}", name)))?;
        Ok(())
    }

    /// Get the module table for a loaded plugin.
    ///
    /// Returns the module table that was created when the plugin was loaded.
    /// This is used to expose plugins to the require system.
    pub fn get_module(&self, lua: &Lua, plugin_name: &str) -> LuaResult<Table> {
        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| mlua::Error::external(format!("plugin not loaded: {}", plugin_name)))?;

        lua.registry_value(&plugin.module_key)
    }

    /// Create a capability from a loaded plugin.
    ///
    /// Calls the plugin's `capability(params)` function and returns the
    /// resulting userdata. The params table should contain plugin-specific
    /// configuration (e.g., `{ path = "/tmp", mode = "rw" }` for fs plugin).
    pub fn create_capability(
        &self,
        lua: &Lua,
        plugin_name: &str,
        params: Table,
    ) -> LuaResult<Value> {
        let plugin = self
            .plugins
            .get(plugin_name)
            .ok_or_else(|| mlua::Error::external(format!("plugin not loaded: {}", plugin_name)))?;

        // Get the module table from registry
        let module: Table = lua.registry_value(&plugin.module_key)?;

        // Call the capability function
        let capability_fn = module.get::<mlua::Function>("capability").map_err(|_| {
            mlua::Error::external(format!(
                "plugin {} does not export a capability function",
                plugin_name
            ))
        })?;

        capability_fn.call(params)
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the platform-specific library filename.
fn library_filename(name: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        format!("librhizome_spore_{}.so", name)
    }
    #[cfg(target_os = "macos")]
    {
        format!("librhizome_spore_{}.dylib", name)
    }
    #[cfg(target_os = "windows")]
    {
        format!("rhizome_spore_{}.dll", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_filename() {
        let name = library_filename("fs");
        #[cfg(target_os = "linux")]
        assert_eq!(name, "librhizome_spore_fs.so");
        #[cfg(target_os = "macos")]
        assert_eq!(name, "librhizome_spore_fs.dylib");
        #[cfg(target_os = "windows")]
        assert_eq!(name, "rhizome_spore_fs.dll");
    }

    #[test]
    fn test_plugin_loader_new() {
        let loader = PluginLoader::new();
        assert!(loader.search_paths.len() >= 2);
        assert!(loader.plugins.is_empty());
    }

    #[test]
    fn test_add_search_path() {
        let mut loader = PluginLoader::new();
        let custom = PathBuf::from("/custom/plugins");
        loader.add_search_path(custom.clone());
        assert_eq!(loader.search_paths[0], custom);
    }

    #[test]
    fn test_get_module_not_loaded() {
        let lua = Lua::new();
        let loader = PluginLoader::new();
        let result = loader.get_module(&lua, "nonexistent");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("plugin not loaded")
        );
    }

    #[test]
    fn test_create_capability_not_loaded() {
        let lua = Lua::new();
        let loader = PluginLoader::new();
        let params = lua.create_table().unwrap();
        let result = loader.create_capability(&lua, "nonexistent", params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("plugin not loaded")
        );
    }
}
