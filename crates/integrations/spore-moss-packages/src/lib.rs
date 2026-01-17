//! rhizome-spore-moss-packages: Moss packages integration for spore.
//!
//! Registers package ecosystem and index functions into the spore Lua runtime:
//!
//! ## Ecosystem Detection
//! - `packages.ecosystem.list()` - List all ecosystem names
//! - `packages.ecosystem.detect(root?)` - Detect ecosystem for project
//! - `packages.ecosystem.is_available(name)` - Check if ecosystem tool available
//!
//! ## Package Queries
//! - `packages.query(package, opts?)` - Query package info
//! - `packages.dependencies(root?)` - List declared dependencies
//! - `packages.tree(root?)` - Get dependency tree
//! - `packages.audit(root?)` - Check for vulnerabilities
//!
//! ## Package Index
//! - `packages.index.list()` - List all index names
//! - `packages.index.fetch(index, package)` - Fetch package metadata

use mlua::{Lua, Result, Table};
use rhizome_moss_packages::{
    AuditResult, Dependency, DependencyTree, Ecosystem, PackageError, PackageInfo, TreeNode,
    Vulnerability, detect_ecosystem, get_ecosystem,
    index::{self, PackageMeta},
    list_ecosystems,
};
use rhizome_spore_lua::Integration;
use std::path::{Path, PathBuf};

/// Moss packages integration for spore.
pub struct MossPackagesIntegration {
    root: PathBuf,
}

impl MossPackagesIntegration {
    /// Create a new packages integration rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Integration for MossPackagesIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let packages = lua.create_table()?;

        // Ecosystem functions (packages.ecosystem.*)
        register_ecosystem(&packages, lua, &self.root)?;

        // Package query functions
        register_query(&packages, lua, &self.root)?;
        register_dependencies(&packages, lua, &self.root)?;
        register_tree(&packages, lua, &self.root)?;
        register_audit(&packages, lua, &self.root)?;

        // Index functions (packages.index.*)
        register_index(&packages, lua)?;

        lua.globals().set("packages", packages)?;
        Ok(())
    }
}

/// Register packages.ecosystem.* functions
fn register_ecosystem(packages: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let ecosystem = lua.create_table()?;

    // packages.ecosystem.list() -> array of ecosystem names
    ecosystem.set(
        "list",
        lua.create_function(|lua, ()| {
            let ecosystems = list_ecosystems();
            let result = lua.create_table()?;
            for (i, name) in ecosystems.iter().enumerate() {
                result.set(i + 1, *name)?;
            }
            Ok(result)
        })?,
    )?;

    // packages.ecosystem.detect(root?) -> ecosystem name or nil
    let root_detect = root.to_path_buf();
    ecosystem.set(
        "detect",
        lua.create_function(move |_, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root_detect.join(p)
                    }
                })
                .unwrap_or_else(|| root_detect.clone());

            Ok(detect_ecosystem(&target).map(|e| e.name()))
        })?,
    )?;

    // packages.ecosystem.is_available(name) -> bool
    ecosystem.set(
        "is_available",
        lua.create_function(|_, name: String| {
            if let Some(eco) = get_ecosystem(&name) {
                Ok(eco.find_tool().is_some())
            } else {
                Ok(false)
            }
        })?,
    )?;

    packages.set("ecosystem", ecosystem)?;
    Ok(())
}

/// Register packages.query(package, opts?) -> package info
fn register_query(packages: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    packages.set(
        "query",
        lua.create_function(move |lua, args: (String, Option<Table>)| {
            let (package, opts) = args;

            let ecosystem_name = opts
                .as_ref()
                .and_then(|t| t.get::<String>("ecosystem").ok());

            let eco: &dyn Ecosystem = if let Some(name) = ecosystem_name {
                get_ecosystem(&name)
                    .ok_or_else(|| mlua::Error::external(format!("Unknown ecosystem: {}", name)))?
            } else {
                detect_ecosystem(&root)
                    .ok_or_else(|| mlua::Error::external("No ecosystem detected for project"))?
            };

            let info = eco
                .query(&package, &root)
                .map_err(|e| mlua::Error::external(package_error_message(e)))?;

            package_info_to_lua(lua, &info)
        })?,
    )?;
    Ok(())
}

/// Register packages.dependencies(root?) -> array of dependencies
fn register_dependencies(packages: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    packages.set(
        "dependencies",
        lua.create_function(move |lua, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root.join(p)
                    }
                })
                .unwrap_or_else(|| root.clone());

            let eco = detect_ecosystem(&target)
                .ok_or_else(|| mlua::Error::external("No ecosystem detected for project"))?;

            let deps = eco
                .list_dependencies(&target)
                .map_err(|e| mlua::Error::external(package_error_message(e)))?;

            let result = lua.create_table()?;
            for (i, dep) in deps.iter().enumerate() {
                result.set(i + 1, dependency_to_lua(lua, dep)?)?;
            }
            Ok(result)
        })?,
    )?;
    Ok(())
}

/// Register packages.tree(root?) -> dependency tree
fn register_tree(packages: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    packages.set(
        "tree",
        lua.create_function(move |lua, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root.join(p)
                    }
                })
                .unwrap_or_else(|| root.clone());

            let eco = detect_ecosystem(&target)
                .ok_or_else(|| mlua::Error::external("No ecosystem detected for project"))?;

            let tree = eco
                .dependency_tree(&target)
                .map_err(|e| mlua::Error::external(package_error_message(e)))?;

            dependency_tree_to_lua(lua, &tree)
        })?,
    )?;
    Ok(())
}

/// Register packages.audit(root?) -> audit result
fn register_audit(packages: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    packages.set(
        "audit",
        lua.create_function(move |lua, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root.join(p)
                    }
                })
                .unwrap_or_else(|| root.clone());

            let eco = detect_ecosystem(&target)
                .ok_or_else(|| mlua::Error::external("No ecosystem detected for project"))?;

            let result = eco
                .audit(&target)
                .map_err(|e| mlua::Error::external(package_error_message(e)))?;

            audit_result_to_lua(lua, &result)
        })?,
    )?;
    Ok(())
}

/// Register packages.index.* functions
fn register_index(packages: &Table, lua: &Lua) -> Result<()> {
    let idx = lua.create_table()?;

    // packages.index.list() -> array of index names
    idx.set(
        "list",
        lua.create_function(|lua, ()| {
            let indices = index::list_indices();
            let result = lua.create_table()?;
            for (i, name) in indices.iter().enumerate() {
                result.set(i + 1, *name)?;
            }
            Ok(result)
        })?,
    )?;

    // packages.index.fetch(index, package) -> package metadata
    idx.set(
        "fetch",
        lua.create_function(|lua, args: (String, String)| {
            let (index_name, package) = args;

            let idx = index::get_index(&index_name)
                .ok_or_else(|| mlua::Error::external(format!("Unknown index: {}", index_name)))?;

            let meta = idx
                .fetch(&package)
                .map_err(|e| mlua::Error::external(format!("Index fetch failed: {}", e)))?;

            package_meta_to_lua(lua, &meta)
        })?,
    )?;

    packages.set("index", idx)?;
    Ok(())
}

/// Convert PackageError to user-friendly message
fn package_error_message(e: PackageError) -> String {
    match e {
        PackageError::NoToolFound => "No package manager tool found in PATH".to_string(),
        PackageError::ToolFailed(msg) => format!("Package tool failed: {}", msg),
        PackageError::ParseError(msg) => format!("Failed to parse output: {}", msg),
        PackageError::NotFound(name) => format!("Package not found: {}", name),
        PackageError::RegistryError(msg) => format!("Registry error: {}", msg),
    }
}

/// Convert PackageInfo to Lua table
fn package_info_to_lua(lua: &Lua, info: &PackageInfo) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("name", info.name.clone())?;
    t.set("version", info.version.clone())?;

    if let Some(desc) = &info.description {
        t.set("description", desc.clone())?;
    }
    if let Some(license) = &info.license {
        t.set("license", license.clone())?;
    }
    if let Some(homepage) = &info.homepage {
        t.set("homepage", homepage.clone())?;
    }
    if let Some(repo) = &info.repository {
        t.set("repository", repo.clone())?;
    }

    // Features
    if !info.features.is_empty() {
        let features = lua.create_table()?;
        for (i, feat) in info.features.iter().enumerate() {
            let f = lua.create_table()?;
            f.set("name", feat.name.clone())?;
            if let Some(desc) = &feat.description {
                f.set("description", desc.clone())?;
            }
            let deps = lua.create_table()?;
            for (j, d) in feat.dependencies.iter().enumerate() {
                deps.set(j + 1, d.clone())?;
            }
            f.set("dependencies", deps)?;
            features.set(i + 1, f)?;
        }
        t.set("features", features)?;
    }

    // Dependencies
    if !info.dependencies.is_empty() {
        let deps = lua.create_table()?;
        for (i, dep) in info.dependencies.iter().enumerate() {
            deps.set(i + 1, dependency_to_lua(lua, dep)?)?;
        }
        t.set("dependencies", deps)?;
    }

    Ok(t)
}

/// Convert Dependency to Lua table
fn dependency_to_lua(lua: &Lua, dep: &Dependency) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("name", dep.name.clone())?;
    if let Some(version) = &dep.version_req {
        t.set("version_req", version.clone())?;
    }
    t.set("optional", dep.optional)?;
    Ok(t)
}

/// Convert DependencyTree to Lua table
fn dependency_tree_to_lua(lua: &Lua, tree: &DependencyTree) -> Result<Table> {
    let t = lua.create_table()?;
    let roots = lua.create_table()?;
    for (i, node) in tree.roots.iter().enumerate() {
        roots.set(i + 1, tree_node_to_lua(lua, node)?)?;
    }
    t.set("roots", roots)?;
    Ok(t)
}

/// Convert TreeNode to Lua table (recursive)
fn tree_node_to_lua(lua: &Lua, node: &TreeNode) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("name", node.name.clone())?;
    t.set("version", node.version.clone())?;

    if !node.dependencies.is_empty() {
        let deps = lua.create_table()?;
        for (i, child) in node.dependencies.iter().enumerate() {
            deps.set(i + 1, tree_node_to_lua(lua, child)?)?;
        }
        t.set("dependencies", deps)?;
    }

    Ok(t)
}

/// Convert AuditResult to Lua table
fn audit_result_to_lua(lua: &Lua, result: &AuditResult) -> Result<Table> {
    let t = lua.create_table()?;
    let vulns = lua.create_table()?;
    for (i, vuln) in result.vulnerabilities.iter().enumerate() {
        vulns.set(i + 1, vulnerability_to_lua(lua, vuln)?)?;
    }
    t.set("vulnerabilities", vulns)?;
    Ok(t)
}

/// Convert Vulnerability to Lua table
fn vulnerability_to_lua(lua: &Lua, vuln: &Vulnerability) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("package", vuln.package.clone())?;
    t.set("version", vuln.version.clone())?;
    t.set("severity", vuln.severity.as_str())?;
    t.set("title", vuln.title.clone())?;
    if let Some(url) = &vuln.url {
        t.set("url", url.clone())?;
    }
    if let Some(cve) = &vuln.cve {
        t.set("cve", cve.clone())?;
    }
    if let Some(fixed) = &vuln.fixed_in {
        t.set("fixed_in", fixed.clone())?;
    }
    Ok(t)
}

/// Convert PackageMeta to Lua table
fn package_meta_to_lua(lua: &Lua, meta: &PackageMeta) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("name", meta.name.clone())?;
    t.set("version", meta.version.clone())?;
    if let Some(desc) = &meta.description {
        t.set("description", desc.clone())?;
    }
    if let Some(license) = &meta.license {
        t.set("license", license.clone())?;
    }
    if let Some(homepage) = &meta.homepage {
        t.set("homepage", homepage.clone())?;
    }
    if let Some(repo) = &meta.repository {
        t.set("repository", repo.clone())?;
    }
    Ok(t)
}
