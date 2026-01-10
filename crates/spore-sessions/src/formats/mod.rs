//! Log format plugins.
//!
//! Each format implements the `LogFormat` trait for parsing session logs.
//!
//! # Extensibility
//!
//! Users can register custom formats via [`register()`]:
//!
//! ```ignore
//! use spore_sessions::{LogFormat, SessionAnalysis, SessionFile, register};
//! use std::path::{Path, PathBuf};
//!
//! struct MyAgentFormat;
//!
//! impl LogFormat for MyAgentFormat {
//!     fn name(&self) -> &'static str { "myagent" }
//!     fn sessions_dir(&self, project: Option<&Path>) -> PathBuf { /* ... */ }
//!     fn list_sessions(&self, project: Option<&Path>) -> Vec<SessionFile> { /* ... */ }
//!     fn detect(&self, path: &Path) -> f64 { /* ... */ }
//!     fn analyze(&self, path: &Path) -> Result<SessionAnalysis, String> { /* ... */ }
//! }
//!
//! // Register before first use
//! register(&MyAgentFormat);
//! ```

mod claude_code;
mod codex;
mod gemini_cli;
mod moss_agent;

pub use claude_code::ClaudeCodeFormat;
pub use codex::CodexFormat;
pub use gemini_cli::GeminiCliFormat;
pub use moss_agent::MossAgentFormat;

use crate::SessionAnalysis;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// Global registry of log format plugins.
static FORMATS: RwLock<Vec<&'static dyn LogFormat>> = RwLock::new(Vec::new());
static INITIALIZED: OnceLock<()> = OnceLock::new();

/// Register a custom log format plugin.
///
/// Call this before any parsing operations to add custom formats.
/// Built-in formats are registered automatically on first use.
pub fn register(format: &'static dyn LogFormat) {
    FORMATS.write().unwrap().push(format);
}

/// Initialize built-in formats (called automatically on first use).
fn init_builtin() {
    INITIALIZED.get_or_init(|| {
        let mut formats = FORMATS.write().unwrap();
        formats.push(&ClaudeCodeFormat);
        formats.push(&CodexFormat);
        formats.push(&GeminiCliFormat);
        formats.push(&MossAgentFormat);
    });
}

/// Session file with metadata.
pub struct SessionFile {
    pub path: PathBuf,
    pub mtime: std::time::SystemTime,
}

/// Trait for session log format plugins.
pub trait LogFormat: Send + Sync {
    /// Format identifier (e.g., "claude", "codex", "gemini", "moss").
    fn name(&self) -> &'static str;

    /// Get the sessions directory for this format.
    /// Does NOT check if the directory exists - that's handled by list_sessions.
    fn sessions_dir(&self, project: Option<&Path>) -> PathBuf;

    /// List all session files for this format.
    fn list_sessions(&self, project: Option<&Path>) -> Vec<SessionFile>;

    /// Check if this format can parse the given file.
    /// Returns a confidence score 0.0-1.0.
    fn detect(&self, path: &Path) -> f64;

    /// Parse the log file and produce analysis.
    fn analyze(&self, path: &Path) -> Result<SessionAnalysis, String>;
}

/// Get a format by name from the global registry.
pub fn get_format(name: &str) -> Option<&'static dyn LogFormat> {
    init_builtin();
    FORMATS
        .read()
        .unwrap()
        .iter()
        .find(|f| f.name() == name)
        .copied()
}

/// Auto-detect format for a file using the global registry.
pub fn detect_format(path: &Path) -> Option<&'static dyn LogFormat> {
    init_builtin();
    let formats = FORMATS.read().unwrap();
    let mut best: Option<(&'static dyn LogFormat, f64)> = None;
    for fmt in formats.iter() {
        let score = fmt.detect(path);
        if score > 0.0 && (best.is_none() || score > best.unwrap().1) {
            best = Some((*fmt, score));
        }
    }
    best.map(|(fmt, _)| fmt)
}

/// List all available format names from the global registry.
pub fn list_formats() -> Vec<&'static str> {
    init_builtin();
    FORMATS.read().unwrap().iter().map(|f| f.name()).collect()
}

/// Default implementation: list .jsonl files in a directory.
pub fn list_jsonl_sessions(dir: &Path) -> Vec<SessionFile> {
    let mut sessions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(meta) = path.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        sessions.push(SessionFile { path, mtime });
                    }
                }
            }
        }
    }
    sessions
}

/// Registry of available log formats.
///
/// For most use cases, prefer the global registry via [`register()`],
/// [`get_format()`], [`detect_format()`], and [`list_formats()`].
///
/// Use `FormatRegistry` when you need an isolated registry (e.g., testing).
pub struct FormatRegistry {
    formats: Vec<Box<dyn LogFormat>>,
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatRegistry {
    /// Create a new registry with all built-in formats.
    pub fn new() -> Self {
        Self {
            formats: vec![
                Box::new(ClaudeCodeFormat),
                Box::new(CodexFormat),
                Box::new(GeminiCliFormat),
                Box::new(MossAgentFormat),
            ],
        }
    }

    /// Create an empty registry (no built-in formats).
    pub fn empty() -> Self {
        Self { formats: vec![] }
    }

    /// Register a custom format.
    pub fn register(&mut self, format: Box<dyn LogFormat>) {
        self.formats.push(format);
    }

    /// Detect the best format for a file.
    pub fn detect(&self, path: &Path) -> Option<&dyn LogFormat> {
        let mut best: Option<(&dyn LogFormat, f64)> = None;
        for fmt in &self.formats {
            let score = fmt.detect(path);
            if score > 0.0 {
                if best.is_none() || score > best.unwrap().1 {
                    best = Some((fmt.as_ref(), score));
                }
            }
        }
        best.map(|(fmt, _)| fmt)
    }

    /// Get a format by name.
    pub fn get(&self, name: &str) -> Option<&dyn LogFormat> {
        self.formats
            .iter()
            .find(|f| f.name() == name)
            .map(|f| f.as_ref())
    }

    /// List all available format names.
    pub fn list(&self) -> Vec<&'static str> {
        self.formats.iter().map(|f| f.name()).collect()
    }
}

/// Analyze a session log with auto-format detection.
pub fn analyze_session(path: &Path) -> Result<SessionAnalysis, String> {
    let registry = FormatRegistry::new();
    let format = registry
        .detect(path)
        .ok_or_else(|| format!("Unknown log format: {}", path.display()))?;
    format.analyze(path)
}

/// Analyze a session log with explicit format.
pub fn analyze_session_with_format(
    path: &Path,
    format_name: &str,
) -> Result<SessionAnalysis, String> {
    let registry = FormatRegistry::new();
    let format = registry
        .get(format_name)
        .ok_or_else(|| format!("Unknown format: {}", format_name))?;
    format.analyze(path)
}

/// Helper: read first N lines of a file.
pub(crate) fn peek_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .take(n)
        .filter_map(|l| l.ok())
        .collect()
}

/// Helper: read entire file as string.
pub(crate) fn read_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| e.to_string())?;
    Ok(content)
}
