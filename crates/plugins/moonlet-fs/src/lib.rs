//! Filesystem plugin for moonlet with capability-based security.
//!
//! This plugin provides sandboxed filesystem access through capabilities.
//! Each capability is restricted to a root path and access mode (read/write).
//!
//! Implements pith filesystem interfaces (Directory, InputStream, OutputStream).

#![allow(non_snake_case)] // Lua C API convention: L for lua_State

use mlua::ffi::{self, lua_State};
use portals_filesystem::{DirEntry, Directory, Error as FsError, FileType, Metadata};
use portals_io::{InputStream, OutputStream, Seek, SeekFrom, StreamError};
use std::ffi::{CStr, CString, c_char, c_int};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek as StdSeek, Write};
use std::path::{Path, PathBuf};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for FsCapability userdata.
const FS_CAP_METATABLE: &[u8] = b"moonlet.fs.Capability\0";

/// Metatable name for InputStream userdata.
const INPUT_STREAM_METATABLE: &[u8] = b"moonlet.fs.InputStream\0";

/// Metatable name for OutputStream userdata.
const OUTPUT_STREAM_METATABLE: &[u8] = b"moonlet.fs.OutputStream\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Pith InputStream implementation
// ============================================================================

/// File-backed input stream implementing pith's InputStream trait.
pub struct FileInputStream {
    file: File,
}

impl FileInputStream {
    pub fn new(file: File) -> Self {
        Self { file }
    }
}

impl InputStream for FileInputStream {
    fn read_into(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        match self.file.read(buf) {
            Ok(0) if !buf.is_empty() => Err(StreamError::Closed),
            Ok(n) => Ok(n),
            Err(e) => Err(StreamError::Other(e.to_string())),
        }
    }

    fn blocking_read_into(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        // For regular files, read is already blocking
        self.read_into(buf)
    }

    fn subscribe(&self) -> impl Future<Output = ()> {
        // Files are always ready
        std::future::ready(())
    }
}

impl Seek for FileInputStream {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, StreamError> {
        self.file
            .seek(pos.into())
            .map_err(|e| StreamError::Other(e.to_string()))
    }
}

// ============================================================================
// Pith OutputStream implementation
// ============================================================================

/// File-backed output stream implementing pith's OutputStream trait.
pub struct FileOutputStream {
    file: File,
}

impl FileOutputStream {
    pub fn new(file: File) -> Self {
        Self { file }
    }
}

impl OutputStream for FileOutputStream {
    fn check_write(&self) -> Result<usize, StreamError> {
        // Files can always accept writes
        Ok(usize::MAX)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        self.file
            .write_all(bytes)
            .map_err(|e| StreamError::Other(e.to_string()))
    }

    fn blocking_write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        // For regular files, write is already blocking
        self.write(bytes)
    }

    fn flush(&mut self) -> Result<(), StreamError> {
        self.file
            .flush()
            .map_err(|e| StreamError::Other(e.to_string()))
    }

    fn blocking_flush(&mut self) -> Result<(), StreamError> {
        self.flush()
    }

    fn subscribe(&self) -> impl Future<Output = ()> {
        // Files are always ready
        std::future::ready(())
    }
}

impl Seek for FileOutputStream {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, StreamError> {
        self.file
            .seek(pos.into())
            .map_err(|e| StreamError::Other(e.to_string()))
    }
}

// ============================================================================
// Capability (Directory implementation)
// ============================================================================

/// Capability parameters stored in userdata.
#[derive(Debug, Clone)]
pub struct FsCapability {
    /// Root path for this capability.
    root: PathBuf,
    /// Access mode: "r", "w", or "rw".
    mode: String,
}

impl FsCapability {
    /// Create a new capability with the given root and mode.
    pub fn new(root: PathBuf, mode: String) -> Self {
        Self { root, mode }
    }

    fn can_read(&self) -> bool {
        self.mode.contains('r')
    }

    fn can_write(&self) -> bool {
        self.mode.contains('w')
    }

    /// Validate and resolve a path relative to the capability root.
    fn resolve_path(&self, rel_path: &Path) -> Result<PathBuf, FsError> {
        let full_path = self.root.join(rel_path);

        // Canonicalize to resolve .. and symlinks
        let canonical = if full_path.exists() {
            full_path.canonicalize().map_err(FsError::Io)?
        } else {
            normalize_path(&full_path)
        };

        // Canonicalize the root
        let root_canonical = if self.root.exists() {
            self.root.canonicalize().map_err(FsError::Io)?
        } else {
            normalize_path(&self.root)
        };

        // Ensure path doesn't escape root
        if !canonical.starts_with(&root_canonical) {
            return Err(FsError::Access);
        }

        Ok(canonical)
    }

    // Internal methods that return concrete types (for Lua bindings)

    fn open_read_concrete(&self, path: &Path) -> Result<FileInputStream, FsError> {
        if !self.can_read() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        let file = File::open(&full_path)?;
        Ok(FileInputStream::new(file))
    }

    fn open_write_concrete(&self, path: &Path) -> Result<FileOutputStream, FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;

        // Create parent directories if needed
        if let Some(parent) = full_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&full_path)?;
        Ok(FileOutputStream::new(file))
    }

    fn open_append_concrete(&self, path: &Path) -> Result<FileOutputStream, FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;

        // Create parent directories if needed
        if let Some(parent) = full_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)?;
        Ok(FileOutputStream::new(file))
    }

    /// Create an attenuated (restricted) capability.
    pub fn attenuate(&self, path: Option<&Path>, mode: Option<&str>) -> Result<Self, FsError> {
        // Resolve new root path
        let new_root = if let Some(p) = path {
            self.resolve_path(p)?
        } else {
            self.root.clone()
        };

        // Validate new mode is subset of original
        let new_mode = if let Some(m) = mode {
            for c in m.chars() {
                match c {
                    'r' if !self.can_read() => return Err(FsError::Access),
                    'w' if !self.can_write() => return Err(FsError::Access),
                    'r' | 'w' => {}
                    _ => return Err(FsError::Invalid),
                }
            }
            m.to_string()
        } else {
            self.mode.clone()
        };

        Ok(Self {
            root: new_root,
            mode: new_mode,
        })
    }
}

impl Directory for FsCapability {
    fn open_read(&self, path: &Path) -> Result<impl InputStream + Seek, FsError> {
        if !self.can_read() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        let file = File::open(&full_path)?;
        Ok(FileInputStream::new(file))
    }

    fn open_write(&self, path: &Path) -> Result<impl OutputStream + Seek, FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;

        // Create parent directories if needed
        if let Some(parent) = full_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&full_path)?;
        Ok(FileOutputStream::new(file))
    }

    fn open_append(&self, path: &Path) -> Result<impl OutputStream, FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;

        // Create parent directories if needed
        if let Some(parent) = full_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)?;
        Ok(FileOutputStream::new(file))
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, FsError> {
        if !self.can_read() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        let meta = fs::metadata(&full_path)?;

        let file_type = if meta.is_dir() {
            FileType::Directory
        } else if meta.is_symlink() {
            FileType::Symlink
        } else if meta.is_file() {
            FileType::Regular
        } else {
            FileType::Unknown
        };

        Ok(Metadata {
            file_type,
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            accessed: meta
                .accessed()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            created: meta
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        })
    }

    fn read_dir(
        &self,
        path: &Path,
    ) -> Result<impl Iterator<Item = Result<DirEntry, FsError>>, FsError> {
        if !self.can_read() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        let entries = fs::read_dir(&full_path)?;

        Ok(entries.map(|entry| {
            let entry = entry?;
            let meta = entry.metadata()?;
            let file_type = if meta.is_dir() {
                FileType::Directory
            } else if meta.is_symlink() {
                FileType::Symlink
            } else if meta.is_file() {
                FileType::Regular
            } else {
                FileType::Unknown
            };

            Ok(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                file_type,
            })
        }))
    }

    fn create_dir(&self, path: &Path) -> Result<(), FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        fs::create_dir_all(&full_path)?;
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<(), FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        fs::remove_file(&full_path)?;
        Ok(())
    }

    fn remove_dir(&self, path: &Path) -> Result<(), FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let full_path = self.resolve_path(path)?;
        fs::remove_dir(&full_path)?;
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), FsError> {
        if !self.can_write() {
            return Err(FsError::Access);
        }
        let from_path = self.resolve_path(from)?;
        let to_path = self.resolve_path(to)?;
        fs::rename(&from_path, &to_path)?;
        Ok(())
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
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
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
pub unsafe extern "C-unwind" fn luaopen_moonlet_fs(L: *mut lua_State) -> c_int {
    unsafe {
        // Register metatables
        register_capability_metatable(L);
        register_input_stream_metatable(L);
        register_output_stream_metatable(L);

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
        if ffi::luaL_newmetatable(L, FS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 12);

            // Pith Directory methods
            ffi::lua_pushcclosure(L, fs_cap_open_read, 0);
            ffi::lua_setfield(L, -2, c"open_read".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_open_write, 0);
            ffi::lua_setfield(L, -2, c"open_write".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_open_append, 0);
            ffi::lua_setfield(L, -2, c"open_append".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_metadata, 0);
            ffi::lua_setfield(L, -2, c"metadata".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_read_dir, 0);
            ffi::lua_setfield(L, -2, c"read_dir".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_create_dir, 0);
            ffi::lua_setfield(L, -2, c"create_dir".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_remove_file, 0);
            ffi::lua_setfield(L, -2, c"remove_file".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_remove_dir, 0);
            ffi::lua_setfield(L, -2, c"remove_dir".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_rename, 0);
            ffi::lua_setfield(L, -2, c"rename".as_ptr());

            // Attenuate for capability restriction
            ffi::lua_pushcclosure(L, fs_cap_attenuate, 0);
            ffi::lua_setfield(L, -2, c"attenuate".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, fs_cap_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// InputStream metatable
// ============================================================================

unsafe fn register_input_stream_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, INPUT_STREAM_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 3);

            ffi::lua_pushcclosure(L, input_stream_read, 0);
            ffi::lua_setfield(L, -2, c"read".as_ptr());

            ffi::lua_pushcclosure(L, input_stream_read_all, 0);
            ffi::lua_setfield(L, -2, c"read_all".as_ptr());

            ffi::lua_pushcclosure(L, input_stream_close, 0);
            ffi::lua_setfield(L, -2, c"close".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, input_stream_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, input_stream_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// OutputStream metatable
// ============================================================================

unsafe fn register_output_stream_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, OUTPUT_STREAM_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 3);

            ffi::lua_pushcclosure(L, output_stream_write, 0);
            ffi::lua_setfield(L, -2, c"write".as_ptr());

            ffi::lua_pushcclosure(L, output_stream_flush, 0);
            ffi::lua_setfield(L, -2, c"flush".as_ptr());

            ffi::lua_pushcclosure(L, output_stream_close, 0);
            ffi::lua_setfield(L, -2, c"close".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, output_stream_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, output_stream_tostring, 0);
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

        create_capability_userdata(L, FsCapability::new(PathBuf::from(path), mode))
    }
}

/// Create a capability userdata and push it onto the stack.
unsafe fn create_capability_userdata(L: *mut lua_State, cap: FsCapability) -> c_int {
    unsafe {
        let boxed = Box::new(cap);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut FsCapability>());
        let ud_ptr = ud as *mut *mut FsCapability;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, FS_CAP_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

/// Get capability from userdata at given index.
unsafe fn get_capability(L: *mut lua_State, idx: c_int) -> Option<&'static FsCapability> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, FS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let cap_ptr = *(ud as *const *mut FsCapability);
        if cap_ptr.is_null() {
            return None;
        }
        Some(&*cap_ptr)
    }
}

// ============================================================================
// Pith Directory method bindings
// ============================================================================

/// cap:open_read(path) -> InputStream
unsafe extern "C-unwind" fn fs_cap_open_read(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "open_read requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.open_read_concrete(Path::new(rel_path.as_ref())) {
            Ok(stream) => create_input_stream_userdata(L, stream),
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:open_write(path) -> OutputStream
unsafe extern "C-unwind" fn fs_cap_open_write(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "open_write requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.open_write_concrete(Path::new(rel_path.as_ref())) {
            Ok(stream) => create_output_stream_userdata(L, stream),
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:open_append(path) -> OutputStream
unsafe extern "C-unwind" fn fs_cap_open_append(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "open_append requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.open_append_concrete(Path::new(rel_path.as_ref())) {
            Ok(stream) => create_output_stream_userdata(L, stream),
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:metadata(path) -> table { file_type, size, modified, accessed, created }
unsafe extern "C-unwind" fn fs_cap_metadata(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "metadata requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.metadata(Path::new(rel_path.as_ref())) {
            Ok(meta) => {
                ffi::lua_createtable(L, 0, 5);

                // file_type
                let ft = match meta.file_type {
                    FileType::Regular => "file",
                    FileType::Directory => "directory",
                    FileType::Symlink => "symlink",
                    FileType::Unknown => "unknown",
                };
                let c_ft = CString::new(ft).unwrap();
                ffi::lua_pushstring(L, c_ft.as_ptr());
                ffi::lua_setfield(L, -2, c"file_type".as_ptr());

                // size
                ffi::lua_pushinteger(L, meta.size as ffi::lua_Integer);
                ffi::lua_setfield(L, -2, c"size".as_ptr());

                // timestamps (as numbers, nil if unavailable)
                if let Some(t) = meta.modified {
                    ffi::lua_pushinteger(L, t as ffi::lua_Integer);
                } else {
                    ffi::lua_pushnil(L);
                }
                ffi::lua_setfield(L, -2, c"modified".as_ptr());

                if let Some(t) = meta.accessed {
                    ffi::lua_pushinteger(L, t as ffi::lua_Integer);
                } else {
                    ffi::lua_pushnil(L);
                }
                ffi::lua_setfield(L, -2, c"accessed".as_ptr());

                if let Some(t) = meta.created {
                    ffi::lua_pushinteger(L, t as ffi::lua_Integer);
                } else {
                    ffi::lua_pushnil(L);
                }
                ffi::lua_setfield(L, -2, c"created".as_ptr());

                1
            }
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:read_dir(path?) -> iterator function
unsafe extern "C-unwind" fn fs_cap_read_dir(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let rel_path = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let path_ptr = ffi::lua_tostring(L, 2);
            CStr::from_ptr(path_ptr).to_string_lossy().into_owned()
        } else {
            ".".to_string()
        };

        // Collect entries into a table and return an iterator
        match cap.read_dir(Path::new(&rel_path)) {
            Ok(iter) => {
                ffi::lua_createtable(L, 0, 0);
                let mut idx = 1;

                for entry_result in iter {
                    match entry_result {
                        Ok(entry) => {
                            ffi::lua_createtable(L, 0, 2);

                            let c_name = CString::new(entry.name).unwrap();
                            ffi::lua_pushstring(L, c_name.as_ptr());
                            ffi::lua_setfield(L, -2, c"name".as_ptr());

                            let ft = match entry.file_type {
                                FileType::Regular => "file",
                                FileType::Directory => "directory",
                                FileType::Symlink => "symlink",
                                FileType::Unknown => "unknown",
                            };
                            let c_ft = CString::new(ft).unwrap();
                            ffi::lua_pushstring(L, c_ft.as_ptr());
                            ffi::lua_setfield(L, -2, c"file_type".as_ptr());

                            ffi::lua_rawseti(L, -2, idx);
                            idx += 1;
                        }
                        Err(_) => continue, // Skip errored entries
                    }
                }

                1
            }
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:create_dir(path)
unsafe extern "C-unwind" fn fs_cap_create_dir(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "create_dir requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.create_dir(Path::new(rel_path.as_ref())) {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:remove_file(path)
unsafe extern "C-unwind" fn fs_cap_remove_file(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "remove_file requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.remove_file(Path::new(rel_path.as_ref())) {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:remove_dir(path)
unsafe extern "C-unwind" fn fs_cap_remove_dir(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "remove_dir requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let rel_path = CStr::from_ptr(path_ptr).to_string_lossy();

        match cap.remove_dir(Path::new(rel_path.as_ref())) {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:rename(from, to)
unsafe extern "C-unwind" fn fs_cap_rename(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "rename requires 'from' path argument");
        }
        let from_ptr = ffi::lua_tostring(L, 2);
        let from_path = CStr::from_ptr(from_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "rename requires 'to' path argument");
        }
        let to_ptr = ffi::lua_tostring(L, 3);
        let to_path = CStr::from_ptr(to_ptr).to_string_lossy();

        match cap.rename(Path::new(from_path.as_ref()), Path::new(to_path.as_ref())) {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// cap:attenuate({ path = "subdir", mode = "r" }) -> FsCapability
unsafe extern "C-unwind" fn fs_cap_attenuate(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TTABLE {
            return push_error(L, "attenuate expects a table argument");
        }

        // Get new path (relative to current root)
        ffi::lua_getfield(L, 2, c"path".as_ptr());
        let new_path = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let path_ptr = ffi::lua_tostring(L, -1);
            let rel = CStr::from_ptr(path_ptr).to_string_lossy();
            Some(PathBuf::from(rel.as_ref()))
        } else {
            None
        };
        ffi::lua_pop(L, 1);

        // Get new mode
        ffi::lua_getfield(L, 2, c"mode".as_ptr());
        let new_mode = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let mode_ptr = ffi::lua_tostring(L, -1);
            let mode = CStr::from_ptr(mode_ptr).to_string_lossy();
            Some(mode.into_owned())
        } else {
            None
        };
        ffi::lua_pop(L, 1);

        match cap.attenuate(new_path.as_deref(), new_mode.as_deref()) {
            Ok(new_cap) => create_capability_userdata(L, new_cap),
            Err(e) => push_error(L, &format!("cannot attenuate: {}", e)),
        }
    }
}

// ============================================================================
// Capability cleanup
// ============================================================================

unsafe extern "C-unwind" fn fs_cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let cap_ptr = *(ud as *mut *mut FsCapability);
            if !cap_ptr.is_null() {
                drop(Box::from_raw(cap_ptr));
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn fs_cap_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(cap) = get_capability(L, 1) {
            let s = format!("FsCapability(root={:?}, mode={:?})", cap.root, cap.mode);
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"FsCapability(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// InputStream userdata
// ============================================================================

unsafe fn create_input_stream_userdata(L: *mut lua_State, stream: FileInputStream) -> c_int {
    unsafe {
        let boxed = Box::new(stream);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut FileInputStream>());
        let ud_ptr = ud as *mut *mut FileInputStream;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, INPUT_STREAM_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_input_stream(L: *mut lua_State, idx: c_int) -> Option<&'static mut FileInputStream> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, INPUT_STREAM_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let stream_ptr = *(ud as *mut *mut FileInputStream);
        if stream_ptr.is_null() {
            return None;
        }
        Some(&mut *stream_ptr)
    }
}

/// stream:read(len?) -> string
unsafe extern "C-unwind" fn input_stream_read(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(stream) = get_input_stream(L, 1) else {
            return push_error(L, "invalid stream");
        };

        let len = if ffi::lua_type(L, 2) == ffi::LUA_TNUMBER {
            ffi::lua_tointeger(L, 2) as usize
        } else {
            8192 // default chunk size
        };

        match stream.read(len) {
            Ok(data) => {
                ffi::lua_pushlstring(L, data.as_ptr() as *const c_char, data.len());
                1
            }
            Err(StreamError::Closed) => {
                ffi::lua_pushnil(L);
                1
            }
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// stream:read_all() -> string
unsafe extern "C-unwind" fn input_stream_read_all(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(stream) = get_input_stream(L, 1) else {
            return push_error(L, "invalid stream");
        };

        let mut content = Vec::new();
        loop {
            match stream.read(8192) {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => content.extend_from_slice(&chunk),
                Err(StreamError::Closed) => break,
                Err(e) => return push_error(L, &e.to_string()),
            }
        }

        ffi::lua_pushlstring(L, content.as_ptr() as *const c_char, content.len());
        1
    }
}

/// stream:close()
unsafe extern "C-unwind" fn input_stream_close(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let stream_ptr_ptr = ud as *mut *mut FileInputStream;
            let stream_ptr = *stream_ptr_ptr;
            if !stream_ptr.is_null() {
                drop(Box::from_raw(stream_ptr));
                *stream_ptr_ptr = std::ptr::null_mut();
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn input_stream_gc(L: *mut lua_State) -> c_int {
    unsafe { input_stream_close(L) }
}

unsafe extern "C-unwind" fn input_stream_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        ffi::lua_pushstring(L, c"InputStream".as_ptr());
        1
    }
}

// ============================================================================
// OutputStream userdata
// ============================================================================

unsafe fn create_output_stream_userdata(L: *mut lua_State, stream: FileOutputStream) -> c_int {
    unsafe {
        let boxed = Box::new(stream);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut FileOutputStream>());
        let ud_ptr = ud as *mut *mut FileOutputStream;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, OUTPUT_STREAM_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_output_stream(
    L: *mut lua_State,
    idx: c_int,
) -> Option<&'static mut FileOutputStream> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, OUTPUT_STREAM_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let stream_ptr = *(ud as *mut *mut FileOutputStream);
        if stream_ptr.is_null() {
            return None;
        }
        Some(&mut *stream_ptr)
    }
}

/// stream:write(data)
unsafe extern "C-unwind" fn output_stream_write(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(stream) = get_output_stream(L, 1) else {
            return push_error(L, "invalid stream");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "write requires string argument");
        }

        let mut len: usize = 0;
        let data_ptr = ffi::lua_tolstring(L, 2, &mut len);
        let data = std::slice::from_raw_parts(data_ptr as *const u8, len);

        match stream.write(data) {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// stream:flush()
unsafe extern "C-unwind" fn output_stream_flush(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(stream) = get_output_stream(L, 1) else {
            return push_error(L, "invalid stream");
        };

        match stream.flush() {
            Ok(()) => 0,
            Err(e) => push_error(L, &e.to_string()),
        }
    }
}

/// stream:close()
unsafe extern "C-unwind" fn output_stream_close(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let stream_ptr_ptr = ud as *mut *mut FileOutputStream;
            let stream_ptr = *stream_ptr_ptr;
            if !stream_ptr.is_null() {
                // Flush before closing
                let stream = &mut *stream_ptr;
                let _ = stream.flush();
                drop(Box::from_raw(stream_ptr));
                *stream_ptr_ptr = std::ptr::null_mut();
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn output_stream_gc(L: *mut lua_State) -> c_int {
    unsafe { output_stream_close(L) }
}

unsafe extern "C-unwind" fn output_stream_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        ffi::lua_pushstring(L, c"OutputStream".as_ptr());
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
    fn test_capability_can_read_write() {
        let cap = FsCapability::new(PathBuf::from("/tmp"), "rw".to_string());
        assert!(cap.can_read());
        assert!(cap.can_write());

        let readonly = FsCapability::new(PathBuf::from("/tmp"), "r".to_string());
        assert!(readonly.can_read());
        assert!(!readonly.can_write());
    }

    #[test]
    fn test_capability_attenuate() {
        let cap = FsCapability::new(PathBuf::from("/tmp"), "rw".to_string());

        // Can narrow permissions
        let readonly = cap.attenuate(None, Some("r")).unwrap();
        assert!(readonly.can_read());
        assert!(!readonly.can_write());

        // Cannot widen permissions
        let readonly2 = FsCapability::new(PathBuf::from("/tmp"), "r".to_string());
        assert!(readonly2.attenuate(None, Some("rw")).is_err());
    }
}
