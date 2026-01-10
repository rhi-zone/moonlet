//! OpenAI Codex CLI JSONL format parser.

use super::{LogFormat, SessionFile, peek_lines};
use crate::{
    ErrorPattern, SessionAnalysis, TokenStats, ToolStats, categorize_error, normalize_path,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// OpenAI Codex CLI session log format (JSONL).
pub struct CodexFormat;

impl LogFormat for CodexFormat {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn sessions_dir(&self, _project: Option<&Path>) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".codex/sessions")
    }

    fn list_sessions(&self, project: Option<&Path>) -> Vec<SessionFile> {
        let dir = self.sessions_dir(project);
        // Codex stores sessions in ~/.codex/sessions/YYYY/MM/DD/*.jsonl
        let mut sessions = Vec::new();
        // Walk year directories
        if let Ok(years) = std::fs::read_dir(&dir) {
            for year in years.filter_map(|e| e.ok()) {
                if !year.path().is_dir() {
                    continue;
                }
                // Walk month directories
                if let Ok(months) = std::fs::read_dir(year.path()) {
                    for month in months.filter_map(|e| e.ok()) {
                        if !month.path().is_dir() {
                            continue;
                        }
                        // Walk day directories
                        if let Ok(days) = std::fs::read_dir(month.path()) {
                            for day in days.filter_map(|e| e.ok()) {
                                if !day.path().is_dir() {
                                    continue;
                                }
                                // Find .jsonl files
                                if let Ok(files) = std::fs::read_dir(day.path()) {
                                    for file in files.filter_map(|e| e.ok()) {
                                        let path = file.path();
                                        if path.extension().and_then(|e| e.to_str())
                                            == Some("jsonl")
                                        {
                                            if let Ok(meta) = path.metadata() {
                                                if let Ok(mtime) = meta.modified() {
                                                    sessions.push(SessionFile { path, mtime });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        sessions
    }

    fn detect(&self, path: &Path) -> f64 {
        // Check extension
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "jsonl" {
            return 0.0;
        }

        // Peek at first few lines
        for line in peek_lines(path, 5) {
            if let Ok(entry) = serde_json::from_str::<Value>(&line) {
                // Codex has type field with session_meta, response_item, event_msg
                if let Some(t) = entry.get("type").and_then(|v| v.as_str()) {
                    if t == "session_meta" {
                        // Check for codex-specific originator
                        if let Some(originator) = entry
                            .get("payload")
                            .and_then(|p| p.get("originator"))
                            .and_then(|v| v.as_str())
                        {
                            if originator.contains("codex") {
                                return 1.0;
                            }
                        }
                    }
                }
            }
        }
        0.0
    }

    fn analyze(&self, path: &Path) -> Result<SessionAnalysis, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let reader = BufReader::new(file);

        let mut analysis = SessionAnalysis::new(path.to_path_buf(), self.name());
        let mut entries: Vec<Value> = Vec::new();

        // Parse all JSONL entries
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<Value>(&line) {
                entries.push(entry);
            }
        }

        // Count payload types
        for entry in &entries {
            if let Some(payload_type) = entry
                .get("payload")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
            {
                *analysis
                    .message_counts
                    .entry(payload_type.to_string())
                    .or_insert(0) += 1;
            }
        }

        // Analyze tools
        analysis.tool_stats = analyze_tools(&entries);

        // Analyze tokens
        analysis.token_stats = analyze_tokens(&entries);

        // Find error patterns
        analysis.error_patterns = find_error_patterns(&entries);

        // Analyze file token usage
        analysis.file_tokens = analyze_file_tokens(&entries);

        // Count turns
        analysis.total_turns = count_turns(&entries);

        Ok(analysis)
    }
}

fn analyze_tools(entries: &[Value]) -> HashMap<String, ToolStats> {
    let mut stats: HashMap<String, ToolStats> = HashMap::new();

    for entry in entries {
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        if payload.get("type").and_then(|v| v.as_str()) != Some("function_call") {
            continue;
        }

        if let Some(tool_name) = payload.get("name").and_then(|v| v.as_str()) {
            let stat = stats
                .entry(tool_name.to_string())
                .or_insert_with(|| ToolStats::new(tool_name));
            stat.calls += 1;
        }
    }

    // Count errors from function_call_output
    let mut call_results: HashMap<String, bool> = HashMap::new();
    for entry in entries {
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        if payload.get("type").and_then(|v| v.as_str()) == Some("function_call_output") {
            if let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) {
                let output = payload.get("output").and_then(|v| v.as_str()).unwrap_or("");
                // Check for error indicators
                let is_error = output.contains("Exit code: 1")
                    || output.contains("Error:")
                    || output.contains("error:");
                call_results.insert(call_id.to_string(), is_error);
            }
        }
    }

    stats
}

fn analyze_tokens(entries: &[Value]) -> TokenStats {
    let mut stats = TokenStats::default();
    let mut last_total: Option<TokenUsage> = None;

    for entry in entries {
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
            continue;
        }

        let Some(info) = payload.get("info") else {
            continue;
        };

        let Some(total) = info.get("total_token_usage") else {
            continue;
        };

        let input = total
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cached = total
            .get("cached_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = total
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let reasoning = total
            .get("reasoning_output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        last_total = Some(TokenUsage {
            input,
            cached,
            output,
            reasoning,
        });

        let context_window = info
            .get("model_context_window")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if context_window > 0 {
            stats.update_context(input + cached);
        }
    }

    // Use final totals
    if let Some(total) = last_total {
        stats.total_input = total.input;
        stats.cache_read = total.cached;
        stats.total_output = total.output + total.reasoning;
        // Estimate API calls from turn count
        stats.api_calls = count_turns(entries);
    }

    stats
}

struct TokenUsage {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
}

fn find_error_patterns(entries: &[Value]) -> Vec<ErrorPattern> {
    let mut categories: HashMap<&str, Vec<String>> = HashMap::new();

    for entry in entries {
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        if payload.get("type").and_then(|v| v.as_str()) != Some("function_call_output") {
            continue;
        }

        let output = payload.get("output").and_then(|v| v.as_str()).unwrap_or("");

        // Check for error indicators (non-zero exit code or explicit error messages)
        let is_error = output.contains("Exit code: 1")
            || output.starts_with("Error:")
            || output.contains("\nError:")
            || output.contains("error[E"); // Rust compiler errors
        if is_error {
            let error_text: String = output.chars().take(100).collect();
            let category = categorize_error(&error_text);
            categories.entry(category).or_default().push(error_text);
        }
    }

    let mut patterns: Vec<ErrorPattern> = categories
        .into_iter()
        .map(|(category, examples)| {
            let mut pattern = ErrorPattern::new(category);
            pattern.count = examples.len();
            pattern.examples = examples.into_iter().take(3).collect();
            pattern
        })
        .collect();

    patterns.sort_by(|a, b| b.count.cmp(&a.count));
    patterns
}

fn analyze_file_tokens(entries: &[Value]) -> HashMap<String, u64> {
    let mut file_tokens: HashMap<String, u64> = HashMap::new();

    for entry in entries {
        let Some(payload) = entry.get("payload") else {
            continue;
        };

        if payload.get("type").and_then(|v| v.as_str()) != Some("function_call") {
            continue;
        }

        let tool_name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args_str = payload
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");

        let Ok(args) = serde_json::from_str::<Value>(args_str) else {
            continue;
        };

        // Extract workdir from shell_command
        if tool_name == "shell_command" {
            if let Some(workdir) = args.get("workdir").and_then(|v| v.as_str()) {
                let norm = normalize_path(workdir);
                *file_tokens.entry(norm).or_insert(0) += 1;
            }
        }
    }

    file_tokens
}

fn count_turns(entries: &[Value]) -> usize {
    // Count user_message events as turns
    entries
        .iter()
        .filter(|e| {
            e.get("payload")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                == Some("user_message")
        })
        .count()
}
