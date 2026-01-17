//! Filesystem plugin for spore with capability-based security.
//!
//! This plugin provides sandboxed filesystem access through capabilities.
//! Each capability is restricted to a root path and access mode (read/write).

#![allow(non_snake_case)] // Lua C API convention: L for lua_State

use mlua::ffi::{self, lua_State};
use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::path::{Path, PathBuf};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for FsCapability userdata.
const FS_CAP_METATABLE: &[u8] = b"spore.fs.Capability\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct SporePluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

/// Capability parameters stored in userdata.
#[derive(Debug, Clone)]
struct CapabilityParams {
    /// Root path for this capability.
    path: PathBuf,
    /// Access mode: "r", "w", or "rw".
    mode: String,
}

impl CapabilityParams {
    fn can_read(&self) -> bool {
        self.mode.contains('r')
    }

    fn can_write(&self) -> bool {
        self.mode.contains('w')
    }

    /// Validate and resolve a path relative to the capability root.
    fn resolve_path(&self, rel_path: &str) -> Result<PathBuf, String> {
        let full_path = self.path.join(rel_path);

        // Canonicalize to resolve .. and symlinks
        let canonical = if full_path.exists() {
            full_path
                .canonicalize()
                .map_err(|e| format!("invalid path: {}", e))?
        } else {
            normalize_path(&full_path)
        };

        // Canonicalize the root
        let root_canonical = if self.path.exists() {
            self.path
                .canonicalize()
                .map_err(|e| format!("invalid root: {}", e))?
        } else {
            normalize_path(&self.path)
        };

        // Ensure path doesn't escape root
        if !canonical.starts_with(&root_canonical) {
            return Err("path escapes capability root".into());
        }

        Ok(canonical)
    }
}

/// Normalize a path without requiring it to exist.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            _ => result.push(component),
        }
    }
    result
}

// ============================================================================
// Plugin exports
// ============================================================================

/// Plugin info export.
#[unsafe(no_mangle)]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
        name: c"fs".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_spore_fs(L: *mut lua_State) -> c_int {
    unsafe {
        // Register capability metatable
        register_capability_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 1);

        // Add capability constructor
        ffi::lua_pushcclosure(L, fs_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        1 // Return module table
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        // Create metatable if it doesn't exist
        if ffi::luaL_newmetatable(L, FS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            // Create __index table with methods
            ffi::lua_createtable(L, 0, 5);

            // Add methods
            ffi::lua_pushcclosure(L, fs_cap_read, 0);
            ffi::lua_setfield(L, -2, c"read".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_write, 0);
            ffi::lua_setfield(L, -2, c"write".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_list, 0);
            ffi::lua_setfield(L, -2, c"list".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_exists, 0);
            ffi::lua_setfield(L, -2, c"exists".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_attenuate, 0);
            ffi::lua_setfield(L, -2, c"attenuate".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            // Add __gc for cleanup
            ffi::lua_pushcclosure(L, fs_cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            // Add __tostring
            ffi::lua_pushcclosure(L, fs_cap_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// Capability constructor
// ============================================================================

/// fs.capability({ path = "...", mode = "rw" }) -> FsCapability
unsafe extern "C-unwind" fn fs_capability(L: *mut lua_State) -> c_int {
    unsafe {
        // Expect table argument
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "capability expects a table argument");
        }

        // Get path
        ffi::lua_getfield(L, 1, c"path".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TSTRING {
            return push_error(L, "capability requires 'path' string");
        }
        let path_ptr = ffi::lua_tostring(L, -1);
        let path = CStr::from_ptr(path_ptr).to_string_lossy().into_owned();
        ffi::lua_pop(L, 1);

        // Get mode (default "r")
        ffi::lua_getfield(L, 1, c"mode".as_ptr());
        let mode = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let mode_ptr = ffi::lua_tostring(L, -1);
            CStr::from_ptr(mode_ptr).to_string_lossy().into_owned()
        } else {
            "r".to_string()
        };
        ffi::lua_pop(L, 1);

        // Validate mode
        if !mode.chars().all(|c| c == 'r' || c == 'w') {
            return push_error(L, "mode must only contain 'r' and/or 'w'");
        }

        create_capability(L, PathBuf::from(path), mode)
    }
}

/// Create a capability userdata and push it onto the stack.
unsafe fn create_capability(L: *mut lua_State, path: PathBuf, mode: String) -> c_int {
    unsafe {
        // Allocate userdata
        let params = Box::new(CapabilityParams { path, mode });
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut CapabilityParams>());
        let ud_ptr = ud as *mut *mut CapabilityParams;
        *ud_ptr = Box::into_raw(params);

        // Set metatable
        ffi::luaL_newmetatable(L, FS_CAP_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1 // Return userdata
    }
}

/// Get capability params from userdata at given index.
unsafe fn get_capability(L: *mut lua_State, idx: c_int) -> Option<&'static CapabilityParams> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, FS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let params_ptr = *(ud as *const *mut CapabilityParams);
        if params_ptr.is_null() {
            return None;
        }
        Some(&*params_ptr)
    }
}

// ============================================================================
// Capability methods
// ============================================================================

/// cap:read(path) -> string
unsafe extern "C-unwind" fn fs_cap_read(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(params) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !params.can_read() {
            return push_error(L, "capability does not permit reads");
        }

        // Get path argument
        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "read requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        // Resolve and validate path
        let full_path = match params.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        // Read file
        match fs::read_to_string(&full_path) {
            Ok(content) => {
                let c_content = CString::new(content).unwrap_or_else(|_| CString::new("").unwrap());
                ffi::lua_pushstring(L, c_content.as_ptr());
                1
            }
            Err(e) => push_error(L, &format!("read failed: {}", e)),
        }
    }
}

/// cap:write(path, content)
unsafe extern "C-unwind" fn fs_cap_write(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(params) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !params.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        // Get path argument
        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "write requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        // Get content argument
        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "write requires content argument");
        }
        let content_ptr = ffi::lua_tostring(L, 3);
        let content = CStr::from_ptr(content_ptr).to_string_lossy();

        // Resolve and validate path
        let full_path = match params.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        // Create parent directories if needed
        if let Some(parent) = full_path.parent()
            && !parent.exists()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return push_error(L, &format!("failed to create directories: {}", e));
        }

        // Write file
        match fs::write(&full_path, content.as_bytes()) {
            Ok(()) => 0, // No return value
            Err(e) => push_error(L, &format!("write failed: {}", e)),
        }
    }
}

/// cap:list(path?) -> table
unsafe extern "C-unwind" fn fs_cap_list(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(params) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !params.can_read() {
            return push_error(L, "capability does not permit reads");
        }

        // Get optional path argument (default ".")
        let rel_path = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let path_ptr = ffi::lua_tostring(L, 2);
            CStr::from_ptr(path_ptr).to_string_lossy().into_owned()
        } else {
            ".".to_string()
        };

        // Resolve and validate path
        let full_path = match params.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        // List directory
        let entries = match fs::read_dir(&full_path) {
            Ok(entries) => entries,
            Err(e) => return push_error(L, &format!("list failed: {}", e)),
        };

        // Create result table
        ffi::lua_createtable(L, 0, 0);
        let mut idx = 1;

        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                let c_name = CString::new(name).unwrap_or_else(|_| CString::new("").unwrap());
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_rawseti(L, -2, idx);
                idx += 1;
            }
        }

        1 // Return table
    }
}

/// cap:exists(path) -> boolean
unsafe extern "C-unwind" fn fs_cap_exists(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(params) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !params.can_read() {
            return push_error(L, "capability does not permit reads");
        }

        // Get path argument
        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "exists requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        // Resolve path (we use the non-canonical version to check existence)
        let full_path = params.path.join(rel_path.as_ref());

        // Check if path escapes root using normalized path
        let normalized = normalize_path(&full_path);
        let root_normalized = normalize_path(&params.path);
        if !normalized.starts_with(&root_normalized) {
            ffi::lua_pushboolean(L, 0);
            return 1;
        }

        ffi::lua_pushboolean(L, full_path.exists() as c_int);
        1
    }
}

/// cap:attenuate({ path = "subdir", mode = "r" }) -> FsCapability
unsafe extern "C-unwind" fn fs_cap_attenuate(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(params) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        // Expect table argument
        if ffi::lua_type(L, 2) != ffi::LUA_TTABLE {
            return push_error(L, "attenuate expects a table argument");
        }

        // Get new path (relative to current root)
        ffi::lua_getfield(L, 2, c"path".as_ptr());
        let new_path = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let path_ptr = ffi::lua_tostring(L, -1);
            let rel = CStr::from_ptr(path_ptr).to_string_lossy();
            // Validate it doesn't escape
            match params.resolve_path(&rel) {
                Ok(p) => p,
                Err(e) => return push_error(L, &format!("cannot attenuate: {}", e)),
            }
        } else {
            params.path.clone()
        };
        ffi::lua_pop(L, 1);

        // Get new mode
        ffi::lua_getfield(L, 2, c"mode".as_ptr());
        let new_mode = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let mode_ptr = ffi::lua_tostring(L, -1);
            let mode = CStr::from_ptr(mode_ptr).to_string_lossy();

            // Validate mode is subset of original
            for c in mode.chars() {
                match c {
                    'r' if !params.can_read() => {
                        return push_error(L, "cannot attenuate: original lacks read permission");
                    }
                    'w' if !params.can_write() => {
                        return push_error(L, "cannot attenuate: original lacks write permission");
                    }
                    'r' | 'w' => {}
                    _ => return push_error(L, "mode must only contain 'r' and/or 'w'"),
                }
            }
            mode.into_owned()
        } else {
            params.mode.clone()
        };
        ffi::lua_pop(L, 1);

        create_capability(L, new_path, new_mode)
    }
}

/// Garbage collector for capability userdata.
unsafe extern "C-unwind" fn fs_cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let params_ptr = *(ud as *mut *mut CapabilityParams);
            if !params_ptr.is_null() {
                // Free the boxed params
                drop(Box::from_raw(params_ptr));
            }
        }
        0
    }
}

/// __tostring for capability.
unsafe extern "C-unwind" fn fs_cap_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(params) = get_capability(L, 1) {
            let s = format!(
                "FsCapability(path={:?}, mode={:?})",
                params.path, params.mode
            );
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"FsCapability(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Push an error message and call lua_error.
unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(
            normalize_path(Path::new("/a/./b/c")),
            PathBuf::from("/a/b/c")
        );
    }

    #[test]
    fn test_capability_params_can_read_write() {
        let cap = CapabilityParams {
            path: PathBuf::from("/tmp"),
            mode: "rw".to_string(),
        };
        assert!(cap.can_read());
        assert!(cap.can_write());

        let readonly = CapabilityParams {
            path: PathBuf::from("/tmp"),
            mode: "r".to_string(),
        };
        assert!(readonly.can_read());
        assert!(!readonly.can_write());
    }
}
