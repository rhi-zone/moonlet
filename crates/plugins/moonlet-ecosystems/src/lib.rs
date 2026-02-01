//! Ecosystem plugin for moonlet.
//!
//! Provides capability-based access to package ecosystem queries.
//!
//! ## Module Functions (no capability needed)
//! - `ecosystems.list()` - List all ecosystem names
//! - `ecosystems.is_available(name)` - Check if ecosystem tool available
//!
//! ## Capability Constructor
//! - `ecosystems.capability({ root = "..." })` - Create ecosystems capability for project
//!
//! ## Capability Methods
//! - `cap:detect()` - Detect ecosystem for project
//! - `cap:query(package, opts?)` - Query package info
//! - `cap:dependencies()` - List declared dependencies
//! - `cap:tree()` - Get dependency tree
//! - `cap:audit()` - Check for vulnerabilities

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use normalize_ecosystems::{
    AuditResult, Dependency, DependencyTree, Ecosystem, PackageError, PackageInfo, TreeNode,
    Vulnerability, detect_ecosystem, get_ecosystem, list_ecosystems,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::PathBuf;

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for EcosystemsCapability userdata.
const ECOSYSTEMS_CAP_METATABLE: &[u8] = b"moonlet.ecosystems.Capability\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Capability
// ============================================================================

/// Packages capability - provides access to query packages for a project root.
#[derive(Debug, Clone)]
pub struct EcosystemsCapability {
    root: PathBuf,
}

impl EcosystemsCapability {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
        name: c"ecosystems".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_moonlet_ecosystems(L: *mut lua_State) -> c_int {
    unsafe {
        // Register capability metatable
        register_capability_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 3);

        // Module functions (no capability needed)
        ffi::lua_pushcclosure(L, ecosystems_list, 0);
        ffi::lua_setfield(L, -2, c"list".as_ptr());

        ffi::lua_pushcclosure(L, ecosystems_is_available, 0);
        ffi::lua_setfield(L, -2, c"is_available".as_ptr());

        // Capability constructor
        ffi::lua_pushcclosure(L, ecosystems_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        1
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, ECOSYSTEMS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 6);

            ffi::lua_pushcclosure(L, cap_detect, 0);
            ffi::lua_setfield(L, -2, c"detect".as_ptr());

            ffi::lua_pushcclosure(L, cap_query, 0);
            ffi::lua_setfield(L, -2, c"query".as_ptr());

            ffi::lua_pushcclosure(L, cap_dependencies, 0);
            ffi::lua_setfield(L, -2, c"dependencies".as_ptr());

            ffi::lua_pushcclosure(L, cap_tree, 0);
            ffi::lua_setfield(L, -2, c"tree".as_ptr());

            ffi::lua_pushcclosure(L, cap_audit, 0);
            ffi::lua_setfield(L, -2, c"audit".as_ptr());

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
// Module functions (no capability needed)
// ============================================================================

/// ecosystems.list() -> array of ecosystem names
unsafe extern "C-unwind" fn ecosystems_list(L: *mut lua_State) -> c_int {
    unsafe {
        let ecosystems = list_ecosystems();
        ffi::lua_createtable(L, ecosystems.len() as c_int, 0);

        for (i, name) in ecosystems.iter().enumerate() {
            let c_name = CString::new(*name).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// ecosystems.is_available(name) -> bool
unsafe extern "C-unwind" fn ecosystems_is_available(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "ecosystem_is_available requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 1);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let available = get_ecosystem(&name)
            .map(|e| e.find_tool().is_some())
            .unwrap_or(false);
        ffi::lua_pushboolean(L, available as c_int);
        1
    }
}

// ============================================================================
// Capability constructor
// ============================================================================

/// ecosystems.capability({ root = "..." }) -> EcosystemsCapability
unsafe extern "C-unwind" fn ecosystems_capability(L: *mut lua_State) -> c_int {
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

        create_capability_userdata(L, EcosystemsCapability::new(PathBuf::from(root)))
    }
}

unsafe fn create_capability_userdata(L: *mut lua_State, cap: EcosystemsCapability) -> c_int {
    unsafe {
        let boxed = Box::new(cap);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut EcosystemsCapability>());
        let ud_ptr = ud as *mut *mut EcosystemsCapability;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, ECOSYSTEMS_CAP_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_capability(L: *mut lua_State, idx: c_int) -> Option<&'static EcosystemsCapability> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, ECOSYSTEMS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let cap_ptr = *(ud as *const *mut EcosystemsCapability);
        if cap_ptr.is_null() {
            return None;
        }
        Some(&*cap_ptr)
    }
}

// ============================================================================
// Capability methods
// ============================================================================

/// cap:detect() -> ecosystem name or nil
unsafe extern "C-unwind" fn cap_detect(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        match detect_ecosystem(&cap.root) {
            Some(eco) => {
                let c_name = CString::new(eco.name()).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                1
            }
            None => {
                ffi::lua_pushnil(L);
                1
            }
        }
    }
}

/// cap:query(package, opts?) -> package info table
unsafe extern "C-unwind" fn cap_query(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "query requires package name");
        }
        let pkg_ptr = ffi::lua_tostring(L, 2);
        let package = CStr::from_ptr(pkg_ptr).to_string_lossy();

        // Check for explicit ecosystem in opts
        let eco: &dyn Ecosystem = if ffi::lua_type(L, 3) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 3, c"ecosystem".as_ptr());
            if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                let eco_ptr = ffi::lua_tostring(L, -1);
                let eco_name = CStr::from_ptr(eco_ptr).to_string_lossy();
                ffi::lua_pop(L, 1);
                match get_ecosystem(&eco_name) {
                    Some(e) => e,
                    None => return push_error(L, &format!("Unknown ecosystem: {}", eco_name)),
                }
            } else {
                ffi::lua_pop(L, 1);
                match detect_ecosystem(&cap.root) {
                    Some(e) => e,
                    None => return push_error(L, "No ecosystem detected for project"),
                }
            }
        } else {
            match detect_ecosystem(&cap.root) {
                Some(e) => e,
                None => return push_error(L, "No ecosystem detected for project"),
            }
        };

        match eco.query(&package, &cap.root) {
            Ok(info) => push_package_info(L, &info),
            Err(e) => push_error(L, &package_error_message(e)),
        }
    }
}

/// cap:dependencies() -> array of dependencies
unsafe extern "C-unwind" fn cap_dependencies(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let eco = match detect_ecosystem(&cap.root) {
            Some(e) => e,
            None => return push_error(L, "No ecosystem detected for project"),
        };

        match eco.list_dependencies(&cap.root) {
            Ok(deps) => {
                ffi::lua_createtable(L, deps.len() as c_int, 0);
                for (i, dep) in deps.iter().enumerate() {
                    push_dependency(L, dep);
                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
                1
            }
            Err(e) => push_error(L, &package_error_message(e)),
        }
    }
}

/// cap:tree() -> dependency tree
unsafe extern "C-unwind" fn cap_tree(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let eco = match detect_ecosystem(&cap.root) {
            Some(e) => e,
            None => return push_error(L, "No ecosystem detected for project"),
        };

        match eco.dependency_tree(&cap.root) {
            Ok(tree) => push_dependency_tree(L, &tree),
            Err(e) => push_error(L, &package_error_message(e)),
        }
    }
}

/// cap:audit() -> audit result
unsafe extern "C-unwind" fn cap_audit(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let eco = match detect_ecosystem(&cap.root) {
            Some(e) => e,
            None => return push_error(L, "No ecosystem detected for project"),
        };

        match eco.audit(&cap.root) {
            Ok(result) => push_audit_result(L, &result),
            Err(e) => push_error(L, &package_error_message(e)),
        }
    }
}

/// cap:attenuate({ root = "subdir" }) -> new capability
unsafe extern "C-unwind" fn cap_attenuate(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TTABLE {
            return push_error(L, "attenuate expects a table argument");
        }

        ffi::lua_getfield(L, 2, c"root".as_ptr());
        let new_root = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, -1);
            let rel = CStr::from_ptr(ptr).to_string_lossy();
            let full = cap.root.join(rel.as_ref());

            let canonical = if full.exists() {
                full.canonicalize().unwrap_or(full)
            } else {
                full
            };
            let root_canonical = if cap.root.exists() {
                cap.root.canonicalize().unwrap_or(cap.root.clone())
            } else {
                cap.root.clone()
            };

            if !canonical.starts_with(&root_canonical) {
                return push_error(L, "path escapes capability root");
            }
            canonical
        } else {
            cap.root.clone()
        };
        ffi::lua_pop(L, 1);

        create_capability_userdata(L, EcosystemsCapability::new(new_root))
    }
}

unsafe extern "C-unwind" fn cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let cap_ptr = *(ud as *mut *mut EcosystemsCapability);
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
            let s = format!("EcosystemsCapability(root={:?})", cap.root);
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"EcosystemsCapability(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

fn package_error_message(e: PackageError) -> String {
    match e {
        PackageError::NoToolFound => "No package manager tool found in PATH".to_string(),
        PackageError::ToolFailed(msg) => format!("Package tool failed: {}", msg),
        PackageError::ParseError(msg) => format!("Failed to parse output: {}", msg),
        PackageError::NotFound(name) => format!("Package not found: {}", name),
        PackageError::RegistryError(msg) => format!("Registry error: {}", msg),
    }
}

unsafe fn push_package_info(L: *mut lua_State, info: &PackageInfo) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 7);

        let c_name = CString::new(info.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        let c_version = CString::new(info.version.as_str()).unwrap();
        ffi::lua_pushstring(L, c_version.as_ptr());
        ffi::lua_setfield(L, -2, c"version".as_ptr());

        if let Some(desc) = &info.description {
            let c_desc = CString::new(desc.as_str()).unwrap();
            ffi::lua_pushstring(L, c_desc.as_ptr());
            ffi::lua_setfield(L, -2, c"description".as_ptr());
        }

        if let Some(license) = &info.license {
            let c_lic = CString::new(license.as_str()).unwrap();
            ffi::lua_pushstring(L, c_lic.as_ptr());
            ffi::lua_setfield(L, -2, c"license".as_ptr());
        }

        if let Some(homepage) = &info.homepage {
            let c_hp = CString::new(homepage.as_str()).unwrap();
            ffi::lua_pushstring(L, c_hp.as_ptr());
            ffi::lua_setfield(L, -2, c"homepage".as_ptr());
        }

        if let Some(repo) = &info.repository {
            let c_repo = CString::new(repo.as_str()).unwrap();
            ffi::lua_pushstring(L, c_repo.as_ptr());
            ffi::lua_setfield(L, -2, c"repository".as_ptr());
        }

        // Dependencies
        if !info.dependencies.is_empty() {
            ffi::lua_createtable(L, info.dependencies.len() as c_int, 0);
            for (i, dep) in info.dependencies.iter().enumerate() {
                push_dependency(L, dep);
                ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
            }
            ffi::lua_setfield(L, -2, c"dependencies".as_ptr());
        }

        1
    }
}

unsafe fn push_dependency(L: *mut lua_State, dep: &Dependency) {
    unsafe {
        ffi::lua_createtable(L, 0, 3);

        let c_name = CString::new(dep.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        if let Some(version) = &dep.version_req {
            let c_ver = CString::new(version.as_str()).unwrap();
            ffi::lua_pushstring(L, c_ver.as_ptr());
            ffi::lua_setfield(L, -2, c"version_req".as_ptr());
        }

        ffi::lua_pushboolean(L, dep.optional as c_int);
        ffi::lua_setfield(L, -2, c"optional".as_ptr());
    }
}

unsafe fn push_dependency_tree(L: *mut lua_State, tree: &DependencyTree) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 1);

        ffi::lua_createtable(L, tree.roots.len() as c_int, 0);
        for (i, node) in tree.roots.iter().enumerate() {
            push_tree_node(L, node);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"roots".as_ptr());

        1
    }
}

unsafe fn push_tree_node(L: *mut lua_State, node: &TreeNode) {
    unsafe {
        ffi::lua_createtable(L, 0, 3);

        let c_name = CString::new(node.name.as_str()).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        ffi::lua_setfield(L, -2, c"name".as_ptr());

        let c_version = CString::new(node.version.as_str()).unwrap();
        ffi::lua_pushstring(L, c_version.as_ptr());
        ffi::lua_setfield(L, -2, c"version".as_ptr());

        if !node.dependencies.is_empty() {
            ffi::lua_createtable(L, node.dependencies.len() as c_int, 0);
            for (i, child) in node.dependencies.iter().enumerate() {
                push_tree_node(L, child);
                ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
            }
            ffi::lua_setfield(L, -2, c"dependencies".as_ptr());
        }
    }
}

unsafe fn push_audit_result(L: *mut lua_State, result: &AuditResult) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 1);

        ffi::lua_createtable(L, result.vulnerabilities.len() as c_int, 0);
        for (i, vuln) in result.vulnerabilities.iter().enumerate() {
            push_vulnerability(L, vuln);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"vulnerabilities".as_ptr());

        1
    }
}

unsafe fn push_vulnerability(L: *mut lua_State, vuln: &Vulnerability) {
    unsafe {
        ffi::lua_createtable(L, 0, 7);

        let c_pkg = CString::new(vuln.package.as_str()).unwrap();
        ffi::lua_pushstring(L, c_pkg.as_ptr());
        ffi::lua_setfield(L, -2, c"package".as_ptr());

        let c_version = CString::new(vuln.version.as_str()).unwrap();
        ffi::lua_pushstring(L, c_version.as_ptr());
        ffi::lua_setfield(L, -2, c"version".as_ptr());

        let c_sev = CString::new(vuln.severity.as_str()).unwrap();
        ffi::lua_pushstring(L, c_sev.as_ptr());
        ffi::lua_setfield(L, -2, c"severity".as_ptr());

        let c_title = CString::new(vuln.title.as_str()).unwrap();
        ffi::lua_pushstring(L, c_title.as_ptr());
        ffi::lua_setfield(L, -2, c"title".as_ptr());

        if let Some(url) = &vuln.url {
            let c_url = CString::new(url.as_str()).unwrap();
            ffi::lua_pushstring(L, c_url.as_ptr());
            ffi::lua_setfield(L, -2, c"url".as_ptr());
        }

        if let Some(cve) = &vuln.cve {
            let c_cve = CString::new(cve.as_str()).unwrap();
            ffi::lua_pushstring(L, c_cve.as_ptr());
            ffi::lua_setfield(L, -2, c"cve".as_ptr());
        }

        if let Some(fixed) = &vuln.fixed_in {
            let c_fixed = CString::new(fixed.as_str()).unwrap();
            ffi::lua_pushstring(L, c_fixed.as_ptr());
            ffi::lua_setfield(L, -2, c"fixed_in".as_ptr());
        }
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
