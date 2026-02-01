//! Package index plugin for moonlet.
//!
//! Provides access to package registry metadata lookups.
//!
//! ## Module Functions
//! - `package_index.list()` - List all package index names
//! - `package_index.fetch(index, package)` - Fetch package metadata from registry

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use normalize_package_index::{PackageMeta, get_index, list_indices};
use std::ffi::{CStr, CString, c_char, c_int};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Plugin info for version checking.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
        name: c"package_index".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_moonlet_package_index(L: *mut lua_State) -> c_int {
    unsafe {
        // Create module table
        ffi::lua_createtable(L, 0, 2);

        ffi::lua_pushcclosure(L, index_list, 0);
        ffi::lua_setfield(L, -2, c"list".as_ptr());

        ffi::lua_pushcclosure(L, index_fetch, 0);
        ffi::lua_setfield(L, -2, c"fetch".as_ptr());

        1
    }
}

// ============================================================================
// Module functions
// ============================================================================

/// package_index.list() -> array of index names
unsafe extern "C-unwind" fn index_list(L: *mut lua_State) -> c_int {
    unsafe {
        let indices = list_indices();
        ffi::lua_createtable(L, indices.len() as c_int, 0);

        for (i, name) in indices.iter().enumerate() {
            let c_name = CString::new(*name).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// package_index.fetch(index, package) -> metadata table
unsafe extern "C-unwind" fn index_fetch(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "fetch requires index name");
        }
        let index_ptr = ffi::lua_tostring(L, 1);
        let index_name = CStr::from_ptr(index_ptr).to_string_lossy();

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "fetch requires package name");
        }
        let pkg_ptr = ffi::lua_tostring(L, 2);
        let package = CStr::from_ptr(pkg_ptr).to_string_lossy();

        let idx = match get_index(&index_name) {
            Some(i) => i,
            None => return push_error(L, &format!("Unknown index: {}", index_name)),
        };

        match idx.fetch(&package) {
            Ok(meta) => push_package_meta(L, &meta),
            Err(e) => push_error(L, &format!("Index fetch failed: {}", e)),
        }
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

unsafe fn push_package_meta(L: *mut lua_State, meta: &PackageMeta) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 6);

        let c_name = CString::new(meta.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        let c_version = CString::new(meta.version.as_str()).unwrap();
        ffi::lua_pushstring(L, c_version.as_ptr());
        ffi::lua_setfield(L, -2, c"version".as_ptr());

        if let Some(desc) = &meta.description {
            let c_desc = CString::new(desc.as_str()).unwrap();
            ffi::lua_pushstring(L, c_desc.as_ptr());
            ffi::lua_setfield(L, -2, c"description".as_ptr());
        }

        if let Some(license) = &meta.license {
            let c_lic = CString::new(license.as_str()).unwrap();
            ffi::lua_pushstring(L, c_lic.as_ptr());
            ffi::lua_setfield(L, -2, c"license".as_ptr());
        }

        if let Some(homepage) = &meta.homepage {
            let c_hp = CString::new(homepage.as_str()).unwrap();
            ffi::lua_pushstring(L, c_hp.as_ptr());
            ffi::lua_setfield(L, -2, c"homepage".as_ptr());
        }

        if let Some(repo) = &meta.repository {
            let c_repo = CString::new(repo.as_str()).unwrap();
            ffi::lua_pushstring(L, c_repo.as_ptr());
            ffi::lua_setfield(L, -2, c"repository".as_ptr());
        }

        1
    }
}

// ============================================================================
// Helpers
// ============================================================================

unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
