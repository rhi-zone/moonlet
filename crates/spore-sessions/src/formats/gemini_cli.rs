//! Gemini CLI JSON format parser.

use super::{LogFormat, SessionFile, read_file};
use crate::{SessionAnalysis, TokenStats, ToolStats, normalize_path};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Gemini CLI session log format (JSON with messages array).
pub struct GeminiCliFormat;

impl LogFormat for GeminiCliFormat {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn sessions_dir(&self, _project: Option<&Path>) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".gemini/tmp")
    }

    fn list_sessions(&self, project: Option<&Path>) -> Vec<SessionFile> {
        let dir = self.sessions_dir(project);
        // Gemini stores sessions in ~/.gemini/tmp/<hash>/logs.json
        let mut sessions = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let subdir = entry.path();
                if !subdir.is_dir() {
                    continue;
                }
                let logs_path = subdir.join("logs.json");
                if logs_path.exists() {
                    if let Ok(meta) = logs_path.metadata() {
                        if let Ok(mtime) = meta.modified() {
                            sessions.push(SessionFile {
                                path: logs_path,
                                mtime,
                            });
                        }
                    }
                }
            }
        }
        sessions
    }

    fn detect(&self, path: &Path) -> f64 {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "json" {
            return 0.0;
        }

        // Try to parse as JSON (not JSONL)
        let Ok(content) = read_file(path) else {
            return 0.0;
        };

        let Ok(data) = serde_json::from_str::<Value>(&content) else {
            return 0.0;
        };

        // Gemini CLI has sessionId and messages array with type="gemini"
        if data.get("sessionId").is_some() && data.get("messages").is_some() {
            if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
                for msg in messages {
                    if msg.get("type").and_then(|t| t.as_str()) == Some("gemini") {
                        return 1.0;
                    }
                }
            }
            return 0.5; // Has structure but no gemini messages yet
        }

        0.0
    }

    fn analyze(&self, path: &Path) -> Result<SessionAnalysis, String> {
        let content = read_file(path)?;
        let data: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

        let mut analysis = SessionAnalysis::new(path.to_path_buf(), self.name());

        let messages = data
            .get("messages")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();

        // Count message types
        for msg in &messages {
            if let Some(msg_type) = msg.get("type").and_then(|v| v.as_str()) {
                *analysis
                    .message_counts
                    .entry(msg_type.to_string())
                    .or_insert(0) += 1;
            }
        }

        // Analyze tools
        analysis.tool_stats = analyze_tools(&messages);

        // Analyze tokens
        analysis.token_stats = analyze_tokens(&messages);

        // Analyze file token usage
        analysis.file_tokens = analyze_file_tokens(&messages);

        // Count turns
        analysis.total_turns = messages
            .iter()
            .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("gemini"))
            .count();

        Ok(analysis)
    }
}

fn analyze_tools(messages: &[Value]) -> HashMap<String, ToolStats> {
    let mut stats: HashMap<String, ToolStats> = HashMap::new();

    for msg in messages {
        if msg.get("type").and_then(|t| t.as_str()) != Some("gemini") {
            continue;
        }

        let Some(tool_calls) = msg.get("toolCalls").and_then(|t| t.as_array()) else {
            continue;
        };

        for tc in tool_calls {
            let tool_name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");

            let stat = stats
                .entry(tool_name.to_string())
                .or_insert_with(|| ToolStats::new(tool_name));
            stat.calls += 1;

            // Check for errors
            if tc.get("status").and_then(|s| s.as_str()) == Some("error") {
                stat.errors += 1;
            }
        }
    }

    stats
}

fn analyze_tokens(messages: &[Value]) -> TokenStats {
    let mut stats = TokenStats::default();

    for msg in messages {
        if msg.get("type").and_then(|t| t.as_str()) != Some("gemini") {
            continue;
        }

        let Some(tokens) = msg.get("tokens") else {
            continue;
        };

        stats.api_calls += 1;
        stats.total_input += tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
        stats.total_output += tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
        stats.cache_read += tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(0);

        let context = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0)
            + tokens.get("cached").and_then(|v| v.as_u64()).unwrap_or(0);
        stats.update_context(context);
    }

    stats
}

fn analyze_file_tokens(messages: &[Value]) -> HashMap<String, u64> {
    let mut file_tokens: HashMap<String, u64> = HashMap::new();

    for msg in messages {
        if msg.get("type").and_then(|t| t.as_str()) != Some("gemini") {
            continue;
        }

        let output_tokens = msg
            .get("tokens")
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        if output_tokens == 0 {
            continue;
        }

        // Extract files from tool calls
        let mut files: Vec<String> = Vec::new();
        if let Some(tool_calls) = msg.get("toolCalls").and_then(|t| t.as_array()) {
            for tc in tool_calls {
                if let Some(args) = tc.get("args") {
                    // read_file, write_file have file_path
                    if let Some(fp) = args.get("file_path").and_then(|v| v.as_str()) {
                        files.push(fp.to_string());
                    }
                }
            }
        }

        // Distribute tokens to files
        if !files.is_empty() {
            let per_file = output_tokens / files.len() as u64;
            for f in files {
                let norm = normalize_path(&f);
                *file_tokens.entry(norm).or_insert(0) += per_file;
            }
        }
    }

    file_tokens
}
