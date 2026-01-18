//! Code intelligence plugin for spore.
//!
//! Provides capability-based access to code analysis, search, and editing.
//!
//! ## Capability Constructor
//! - `moss.capability({ root = "..." })` - Create moss capability for codebase
//!
//! ## Capability Methods - View & Search
//! - `cap:view(path)` - View file/symbol structure
//! - `cap:search(pattern, opts?)` - Text search across codebase
//!
//! ## Capability Methods - Analysis
//! - `cap:complexity(path)` - Cyclomatic complexity analysis
//! - `cap:length(path)` - Function length analysis
//! - `cap:health(path?)` - Codebase health check
//!
//! ## Capability Methods - Editing
//! - `cap:find(path, name, opts?)` - Find a symbol by name
//! - `cap:find_all(path, pattern)` - Find all symbols matching pattern
//! - `cap:delete(path, name)` - Delete a symbol
//! - `cap:replace(path, name, content)` - Replace a symbol
//! - `cap:insert_before(path, name, content)` - Insert before a symbol
//! - `cap:insert_after(path, name, content)` - Insert after a symbol
//! - `cap:prepend_to(path, container, content)` - Prepend to class/impl body
//! - `cap:append_to(path, container, content)` - Append to class/impl body
//! - `cap:write(path, content)` - Write content to file

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use rhizome_moss_languages::Symbol;
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::{Path, PathBuf};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for MossCapability userdata.
const MOSS_CAP_METATABLE: &[u8] = b"spore.moss.Capability\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct SporePluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Capability
// ============================================================================

/// Moss capability - provides access to code intelligence for a codebase root.
#[derive(Debug, Clone)]
pub struct MossCapability {
    root: PathBuf,
    /// Access mode: "r" (read-only) or "rw" (read-write for editing)
    mode: String,
}

impl MossCapability {
    pub fn new(root: PathBuf, mode: String) -> Self {
        Self { root, mode }
    }

    fn can_write(&self) -> bool {
        self.mode.contains('w')
    }

    fn resolve_path(&self, rel_path: &str) -> Result<PathBuf, String> {
        let path = Path::new(rel_path);
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Canonicalize to resolve .. and symlinks
        let canonical = if full_path.exists() {
            full_path.canonicalize().map_err(|e| e.to_string())?
        } else {
            normalize_path(&full_path)
        };

        let root_canonical = if self.root.exists() {
            self.root.canonicalize().map_err(|e| e.to_string())?
        } else {
            normalize_path(&self.root)
        };

        // Ensure path doesn't escape root
        if !canonical.starts_with(&root_canonical) {
            return Err("path escapes capability root".to_string());
        }

        Ok(canonical)
    }
}

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

#[unsafe(no_mangle)]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
        name: c"moss".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_spore_moss(L: *mut lua_State) -> c_int {
    unsafe {
        // Register capability metatable
        register_capability_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 1);

        // Capability constructor
        ffi::lua_pushcclosure(L, moss_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        1
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, MOSS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 16);

            // View & Search
            ffi::lua_pushcclosure(L, cap_view, 0);
            ffi::lua_setfield(L, -2, c"view".as_ptr());

            ffi::lua_pushcclosure(L, cap_search, 0);
            ffi::lua_setfield(L, -2, c"search".as_ptr());

            // Analysis
            ffi::lua_pushcclosure(L, cap_complexity, 0);
            ffi::lua_setfield(L, -2, c"complexity".as_ptr());

            ffi::lua_pushcclosure(L, cap_length, 0);
            ffi::lua_setfield(L, -2, c"length".as_ptr());

            ffi::lua_pushcclosure(L, cap_health, 0);
            ffi::lua_setfield(L, -2, c"health".as_ptr());

            // Editing
            ffi::lua_pushcclosure(L, cap_find, 0);
            ffi::lua_setfield(L, -2, c"find".as_ptr());

            ffi::lua_pushcclosure(L, cap_find_all, 0);
            ffi::lua_setfield(L, -2, c"find_all".as_ptr());

            ffi::lua_pushcclosure(L, cap_delete, 0);
            ffi::lua_setfield(L, -2, c"delete".as_ptr());

            ffi::lua_pushcclosure(L, cap_replace, 0);
            ffi::lua_setfield(L, -2, c"replace".as_ptr());

            ffi::lua_pushcclosure(L, cap_insert_before, 0);
            ffi::lua_setfield(L, -2, c"insert_before".as_ptr());

            ffi::lua_pushcclosure(L, cap_insert_after, 0);
            ffi::lua_setfield(L, -2, c"insert_after".as_ptr());

            ffi::lua_pushcclosure(L, cap_prepend_to, 0);
            ffi::lua_setfield(L, -2, c"prepend_to".as_ptr());

            ffi::lua_pushcclosure(L, cap_append_to, 0);
            ffi::lua_setfield(L, -2, c"append_to".as_ptr());

            ffi::lua_pushcclosure(L, cap_write, 0);
            ffi::lua_setfield(L, -2, c"write".as_ptr());

            ffi::lua_pushcclosure(L, cap_attenuate, 0);
            ffi::lua_setfield(L, -2, c"attenuate".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, cap_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// Capability constructor
// ============================================================================

/// moss.capability({ root = "...", mode = "rw" }) -> MossCapability
unsafe extern "C-unwind" fn moss_capability(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "capability expects a table argument");
        }

        // Get root path
        ffi::lua_getfield(L, 1, c"root".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TSTRING {
            return push_error(L, "capability requires 'root' string");
        }
        let root_ptr = ffi::lua_tostring(L, -1);
        let root = CStr::from_ptr(root_ptr).to_string_lossy().into_owned();
        ffi::lua_pop(L, 1);

        // Get mode (default "rw")
        ffi::lua_getfield(L, 1, c"mode".as_ptr());
        let mode = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let mode_ptr = ffi::lua_tostring(L, -1);
            CStr::from_ptr(mode_ptr).to_string_lossy().into_owned()
        } else {
            "rw".to_string()
        };
        ffi::lua_pop(L, 1);

        create_capability_userdata(L, MossCapability::new(PathBuf::from(root), mode))
    }
}

unsafe fn create_capability_userdata(L: *mut lua_State, cap: MossCapability) -> c_int {
    unsafe {
        let boxed = Box::new(cap);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut MossCapability>());
        let ud_ptr = ud as *mut *mut MossCapability;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, MOSS_CAP_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_capability(L: *mut lua_State, idx: c_int) -> Option<&'static MossCapability> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, MOSS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let cap_ptr = *(ud as *const *mut MossCapability);
        if cap_ptr.is_null() {
            return None;
        }
        Some(&*cap_ptr)
    }
}

// ============================================================================
// View & Search
// ============================================================================

/// cap:view(path) -> table of symbols
unsafe extern "C-unwind" fn cap_view(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "view requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let extractor = rhizome_moss::extract::Extractor::new();
        let result = extractor.extract(&full_path, &content);

        // Create output table
        ffi::lua_createtable(L, 0, 2);

        let c_file = CString::new(result.file_path.as_str()).unwrap();
        ffi::lua_pushstring(L, c_file.as_ptr());
        ffi::lua_setfield(L, -2, c"file".as_ptr());

        // Symbols array
        ffi::lua_createtable(L, result.symbols.len() as c_int, 0);
        for (i, sym) in result.symbols.iter().enumerate() {
            push_symbol(L, sym);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"symbols".as_ptr());

        1
    }
}

/// cap:search(pattern, opts?) -> search results
unsafe extern "C-unwind" fn cap_search(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "search requires pattern argument");
        }
        let pattern_ptr = ffi::lua_tostring(L, 2);
        let pattern = CStr::from_ptr(pattern_ptr).to_string_lossy();

        // Parse options
        let (limit, ignore_case) = if ffi::lua_type(L, 3) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 3, c"limit".as_ptr());
            let limit = if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                ffi::lua_tointeger(L, -1) as usize
            } else {
                100
            };
            ffi::lua_pop(L, 1);

            ffi::lua_getfield(L, 3, c"ignore_case".as_ptr());
            let ignore_case = ffi::lua_toboolean(L, -1) != 0;
            ffi::lua_pop(L, 1);

            (limit, ignore_case)
        } else {
            (100, false)
        };

        match rhizome_moss::text_search::grep(&pattern, &cap.root, None, limit, ignore_case) {
            Ok(result) => {
                ffi::lua_createtable(L, 0, 3);

                // Matches array
                ffi::lua_createtable(L, result.matches.len() as c_int, 0);
                for (i, m) in result.matches.iter().enumerate() {
                    ffi::lua_createtable(L, 0, 4);

                    let c_file = CString::new(m.file.as_str()).unwrap();
                    ffi::lua_pushstring(L, c_file.as_ptr());
                    ffi::lua_setfield(L, -2, c"file".as_ptr());

                    ffi::lua_pushinteger(L, m.line as ffi::lua_Integer);
                    ffi::lua_setfield(L, -2, c"line".as_ptr());

                    let c_content = CString::new(m.content.as_str()).unwrap();
                    ffi::lua_pushstring(L, c_content.as_ptr());
                    ffi::lua_setfield(L, -2, c"content".as_ptr());

                    if let Some(sym) = &m.symbol {
                        let c_sym = CString::new(sym.as_str()).unwrap();
                        ffi::lua_pushstring(L, c_sym.as_ptr());
                        ffi::lua_setfield(L, -2, c"symbol".as_ptr());
                    }

                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
                ffi::lua_setfield(L, -2, c"matches".as_ptr());

                ffi::lua_pushinteger(L, result.total_matches as ffi::lua_Integer);
                ffi::lua_setfield(L, -2, c"total".as_ptr());

                ffi::lua_pushinteger(L, result.files_searched as ffi::lua_Integer);
                ffi::lua_setfield(L, -2, c"files_searched".as_ptr());

                1
            }
            Err(e) => push_error(L, &format!("Search failed: {}", e)),
        }
    }
}

// ============================================================================
// Analysis
// ============================================================================

/// cap:complexity(path) -> complexity report
unsafe extern "C-unwind" fn cap_complexity(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "complexity requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let analyzer = rhizome_moss::analyze::complexity::ComplexityAnalyzer::new();
        let report = analyzer.analyze(&full_path, &content);

        ffi::lua_createtable(L, 0, 7);

        let c_file = CString::new(report.file_path.as_str()).unwrap();
        ffi::lua_pushstring(L, c_file.as_ptr());
        ffi::lua_setfield(L, -2, c"file".as_ptr());

        // Functions array
        ffi::lua_createtable(L, report.functions.len() as c_int, 0);
        for (i, f) in report.functions.iter().enumerate() {
            ffi::lua_createtable(L, 0, 5);

            let c_name = CString::new(f.name.as_str()).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_setfield(L, -2, c"name".as_ptr());

            ffi::lua_pushinteger(L, f.complexity as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"complexity".as_ptr());

            ffi::lua_pushinteger(L, f.start_line as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"start_line".as_ptr());

            let c_risk = CString::new(f.risk_level().as_str()).unwrap();
            ffi::lua_pushstring(L, c_risk.as_ptr());
            ffi::lua_setfield(L, -2, c"risk".as_ptr());

            if let Some(parent) = &f.parent {
                let c_parent = CString::new(parent.as_str()).unwrap();
                ffi::lua_pushstring(L, c_parent.as_ptr());
                ffi::lua_setfield(L, -2, c"parent".as_ptr());
            }

            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"functions".as_ptr());

        ffi::lua_pushnumber(L, report.avg_complexity());
        ffi::lua_setfield(L, -2, c"avg_complexity".as_ptr());

        ffi::lua_pushinteger(L, report.max_complexity() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"max_complexity".as_ptr());

        ffi::lua_pushinteger(L, report.high_risk_count() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"high_risk_count".as_ptr());

        ffi::lua_pushinteger(L, report.critical_risk_count() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"critical_risk_count".as_ptr());

        ffi::lua_pushnumber(L, report.score());
        ffi::lua_setfield(L, -2, c"score".as_ptr());

        1
    }
}

/// cap:length(path) -> function length report
unsafe extern "C-unwind" fn cap_length(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "length requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let analyzer = rhizome_moss::analyze::function_length::LengthAnalyzer::new();
        let report = analyzer.analyze(&full_path, &content);

        ffi::lua_createtable(L, 0, 6);

        let c_file = CString::new(report.file_path.as_str()).unwrap();
        ffi::lua_pushstring(L, c_file.as_ptr());
        ffi::lua_setfield(L, -2, c"file".as_ptr());

        // Functions array
        ffi::lua_createtable(L, report.functions.len() as c_int, 0);
        for (i, f) in report.functions.iter().enumerate() {
            ffi::lua_createtable(L, 0, 6);

            let c_name = CString::new(f.name.as_str()).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_setfield(L, -2, c"name".as_ptr());

            ffi::lua_pushinteger(L, f.lines as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"lines".as_ptr());

            ffi::lua_pushinteger(L, f.start_line as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"start_line".as_ptr());

            ffi::lua_pushinteger(L, f.end_line as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"end_line".as_ptr());

            let c_cat = CString::new(f.category().as_str()).unwrap();
            ffi::lua_pushstring(L, c_cat.as_ptr());
            ffi::lua_setfield(L, -2, c"category".as_ptr());

            if let Some(parent) = &f.parent {
                let c_parent = CString::new(parent.as_str()).unwrap();
                ffi::lua_pushstring(L, c_parent.as_ptr());
                ffi::lua_setfield(L, -2, c"parent".as_ptr());
            }

            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"functions".as_ptr());

        ffi::lua_pushnumber(L, report.avg_length());
        ffi::lua_setfield(L, -2, c"avg_length".as_ptr());

        ffi::lua_pushinteger(L, report.max_length() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"max_length".as_ptr());

        ffi::lua_pushinteger(L, report.long_count() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"long_count".as_ptr());

        ffi::lua_pushinteger(L, report.too_long_count() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"too_long_count".as_ptr());

        1
    }
}

/// cap:health(path?) -> health report
unsafe extern "C-unwind" fn cap_health(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let target = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let path_ptr = ffi::lua_tostring(L, 2);
            let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();
            match cap.resolve_path(&rel_path) {
                Ok(p) => p,
                Err(e) => return push_error(L, &e),
            }
        } else {
            cap.root.clone()
        };

        let report = rhizome_moss::health::analyze_health(&target);

        ffi::lua_createtable(L, 0, 8);

        ffi::lua_pushinteger(L, report.total_files as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"total_files".as_ptr());

        ffi::lua_pushinteger(L, report.total_lines as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"total_lines".as_ptr());

        ffi::lua_pushinteger(L, report.total_functions as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"total_functions".as_ptr());

        ffi::lua_pushnumber(L, report.avg_complexity);
        ffi::lua_setfield(L, -2, c"avg_complexity".as_ptr());

        ffi::lua_pushinteger(L, report.max_complexity as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"max_complexity".as_ptr());

        ffi::lua_pushinteger(L, report.high_risk_functions as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"high_risk_functions".as_ptr());

        // Files by language
        ffi::lua_createtable(L, 0, report.files_by_language.len() as c_int);
        for (lang, count) in &report.files_by_language {
            let c_lang = CString::new(lang.as_str()).unwrap();
            ffi::lua_pushinteger(L, *count as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c_lang.as_ptr());
        }
        ffi::lua_setfield(L, -2, c"files_by_language".as_ptr());

        // Large files
        ffi::lua_createtable(L, report.large_files.len() as c_int, 0);
        for (i, lf) in report.large_files.iter().enumerate() {
            ffi::lua_createtable(L, 0, 2);

            let c_path = CString::new(lf.path.as_str()).unwrap();
            ffi::lua_pushstring(L, c_path.as_ptr());
            ffi::lua_setfield(L, -2, c"path".as_ptr());

            ffi::lua_pushinteger(L, lf.lines as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"lines".as_ptr());

            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"large_files".as_ptr());

        1
    }
}

// ============================================================================
// Editing
// ============================================================================

/// cap:find(path, name, opts?) -> symbol location or nil
unsafe extern "C-unwind" fn cap_find(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "find requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "find requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 3);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let case_insensitive = if ffi::lua_type(L, 4) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 4, c"ignore_case".as_ptr());
            let result = ffi::lua_toboolean(L, -1) != 0;
            ffi::lua_pop(L, 1);
            result
        } else {
            false
        };

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        match editor.find_symbol(&full_path, &content, &name, case_insensitive) {
            Some(loc) => {
                push_symbol_location(L, &loc);
                1
            }
            None => {
                ffi::lua_pushnil(L);
                1
            }
        }
    }
}

/// cap:find_all(path, pattern) -> array of symbol locations
unsafe extern "C-unwind" fn cap_find_all(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "find_all requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "find_all requires pattern argument");
        }
        let pattern_ptr = ffi::lua_tostring(L, 3);
        let pattern = CStr::from_ptr(pattern_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let locations = editor.find_symbols_matching(&full_path, &content, &pattern);

        ffi::lua_createtable(L, locations.len() as c_int, 0);
        for (i, loc) in locations.iter().enumerate() {
            push_symbol_location(L, loc);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// cap:delete(path, name) -> modified content
unsafe extern "C-unwind" fn cap_delete(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "delete requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "delete requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 3);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let loc = match editor.find_symbol(&full_path, &content, &name, false) {
            Some(l) => l,
            None => return push_error(L, &format!("Symbol not found: {}", name)),
        };

        let result = editor.delete_symbol(&content, &loc);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:replace(path, name, new_content) -> modified content
unsafe extern "C-unwind" fn cap_replace(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "replace requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "replace requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 3);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "replace requires new_content argument");
        }
        let new_ptr = ffi::lua_tostring(L, 4);
        let new_content = CStr::from_ptr(new_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let loc = match editor.find_symbol(&full_path, &content, &name, false) {
            Some(l) => l,
            None => return push_error(L, &format!("Symbol not found: {}", name)),
        };

        let result = editor.replace_symbol(&content, &loc, &new_content);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:insert_before(path, name, new_content) -> modified content
unsafe extern "C-unwind" fn cap_insert_before(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "insert_before requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "insert_before requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 3);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "insert_before requires new_content argument");
        }
        let new_ptr = ffi::lua_tostring(L, 4);
        let new_content = CStr::from_ptr(new_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let loc = match editor.find_symbol(&full_path, &content, &name, false) {
            Some(l) => l,
            None => return push_error(L, &format!("Symbol not found: {}", name)),
        };

        let result = editor.insert_before(&content, &loc, &new_content);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:insert_after(path, name, new_content) -> modified content
unsafe extern "C-unwind" fn cap_insert_after(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "insert_after requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "insert_after requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 3);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "insert_after requires new_content argument");
        }
        let new_ptr = ffi::lua_tostring(L, 4);
        let new_content = CStr::from_ptr(new_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let loc = match editor.find_symbol(&full_path, &content, &name, false) {
            Some(l) => l,
            None => return push_error(L, &format!("Symbol not found: {}", name)),
        };

        let result = editor.insert_after(&content, &loc, &new_content);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:prepend_to(path, container, content) -> modified content
unsafe extern "C-unwind" fn cap_prepend_to(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "prepend_to requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "prepend_to requires container argument");
        }
        let container_ptr = ffi::lua_tostring(L, 3);
        let container = CStr::from_ptr(container_ptr).to_string_lossy();

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "prepend_to requires content argument");
        }
        let content_ptr = ffi::lua_tostring(L, 4);
        let new_content = CStr::from_ptr(content_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let body = match editor.find_container_body(&full_path, &content, &container) {
            Some(b) => b,
            None => return push_error(L, &format!("Container not found: {}", container)),
        };

        let result = editor.prepend_to_container(&content, &body, &new_content);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:append_to(path, container, content) -> modified content
unsafe extern "C-unwind" fn cap_append_to(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "append_to requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "append_to requires container argument");
        }
        let container_ptr = ffi::lua_tostring(L, 3);
        let container = CStr::from_ptr(container_ptr).to_string_lossy();

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "append_to requires content argument");
        }
        let content_ptr = ffi::lua_tostring(L, 4);
        let new_content = CStr::from_ptr(content_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to read {}: {}", rel_path, e)),
        };

        let editor = rhizome_moss::edit::Editor::new();
        let body = match editor.find_container_body(&full_path, &content, &container) {
            Some(b) => b,
            None => return push_error(L, &format!("Container not found: {}", container)),
        };

        let result = editor.append_to_container(&content, &body, &new_content);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// cap:write(path, content) -> true
unsafe extern "C-unwind" fn cap_write(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if !cap.can_write() {
            return push_error(L, "capability does not permit writes");
        }

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "write requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "write requires content argument");
        }
        let content_ptr = ffi::lua_tostring(L, 3);
        let content = CStr::from_ptr(content_ptr).to_string_lossy();

        let full_path = match cap.resolve_path(&rel_path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &e),
        };

        match std::fs::write(&full_path, content.as_bytes()) {
            Ok(()) => {
                ffi::lua_pushboolean(L, 1);
                1
            }
            Err(e) => push_error(L, &format!("Failed to write {}: {}", rel_path, e)),
        }
    }
}

/// cap:attenuate({ root = "subdir", mode = "r" }) -> new capability
unsafe extern "C-unwind" fn cap_attenuate(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TTABLE {
            return push_error(L, "attenuate expects a table argument");
        }

        // Get new root
        ffi::lua_getfield(L, 2, c"root".as_ptr());
        let new_root = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, -1);
            let rel = CStr::from_ptr(ptr).to_string_lossy();
            match cap.resolve_path(&rel) {
                Ok(p) => p,
                Err(e) => return push_error(L, &e),
            }
        } else {
            cap.root.clone()
        };
        ffi::lua_pop(L, 1);

        // Get new mode
        ffi::lua_getfield(L, 2, c"mode".as_ptr());
        let new_mode = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, -1);
            let mode = CStr::from_ptr(ptr).to_string_lossy();
            // Can only narrow, not expand
            if mode.contains('w') && !cap.can_write() {
                return push_error(L, "cannot expand mode");
            }
            mode.into_owned()
        } else {
            cap.mode.clone()
        };
        ffi::lua_pop(L, 1);

        create_capability_userdata(L, MossCapability::new(new_root, new_mode))
    }
}

unsafe extern "C-unwind" fn cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let cap_ptr = *(ud as *mut *mut MossCapability);
            if !cap_ptr.is_null() {
                drop(Box::from_raw(cap_ptr));
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn cap_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(cap) = get_capability(L, 1) {
            let s = format!("MossCapability(root={:?}, mode={:?})", cap.root, cap.mode);
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"MossCapability(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// Helpers
// ============================================================================

unsafe fn push_symbol(L: *mut lua_State, sym: &Symbol) {
    unsafe {
        ffi::lua_createtable(L, 0, 6);

        let c_name = CString::new(sym.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        let c_kind = CString::new(sym.kind.as_str()).unwrap();
        ffi::lua_pushstring(L, c_kind.as_ptr());
        ffi::lua_setfield(L, -2, c"kind".as_ptr());

        if !sym.signature.is_empty() {
            let c_sig = CString::new(sym.signature.as_str()).unwrap();
            ffi::lua_pushstring(L, c_sig.as_ptr());
            ffi::lua_setfield(L, -2, c"signature".as_ptr());
        }

        ffi::lua_pushinteger(L, sym.start_line as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"start_line".as_ptr());

        ffi::lua_pushinteger(L, sym.end_line as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"end_line".as_ptr());

        if let Some(doc) = &sym.docstring {
            let c_doc = CString::new(doc.as_str()).unwrap();
            ffi::lua_pushstring(L, c_doc.as_ptr());
            ffi::lua_setfield(L, -2, c"docstring".as_ptr());
        }

        // Children
        if !sym.children.is_empty() {
            ffi::lua_createtable(L, sym.children.len() as c_int, 0);
            for (i, child) in sym.children.iter().enumerate() {
                push_symbol(L, child);
                ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
            }
            ffi::lua_setfield(L, -2, c"children".as_ptr());
        }
    }
}

unsafe fn push_symbol_location(L: *mut lua_State, loc: &rhizome_moss::edit::SymbolLocation) {
    unsafe {
        ffi::lua_createtable(L, 0, 6);

        let c_name = CString::new(loc.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        let c_kind = CString::new(loc.kind.as_str()).unwrap();
        ffi::lua_pushstring(L, c_kind.as_ptr());
        ffi::lua_setfield(L, -2, c"kind".as_ptr());

        ffi::lua_pushinteger(L, loc.start_line as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"start_line".as_ptr());

        ffi::lua_pushinteger(L, loc.end_line as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"end_line".as_ptr());

        ffi::lua_pushinteger(L, loc.start_byte as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"start_byte".as_ptr());

        ffi::lua_pushinteger(L, loc.end_byte as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"end_byte".as_ptr());
    }
}

unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
