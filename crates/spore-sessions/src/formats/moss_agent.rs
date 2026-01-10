//! Moss @agent JSONL format parser.

use super::{LogFormat, SessionFile, list_jsonl_sessions, peek_lines};
use crate::{SessionAnalysis, ToolStats};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Moss agent session log format (JSONL).
pub struct MossAgentFormat;

/// Event types in moss agent logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum AgentEvent {
    #[serde(rename = "session_start")]
    SessionStart {
        session_id: String,
        timestamp: String,
        moss_root: Option<String>,
    },
    #[serde(rename = "task")]
    Task {
        user_prompt: String,
        provider: Option<String>,
        model: Option<String>,
        role: Option<String>,
        max_turns: Option<u32>,
        #[serde(flatten)]
        extra: HashMap<String, Value>,
    },
    #[serde(rename = "turn_start")]
    TurnStart {
        turn: u32,
        state: Option<String>,
        working_memory_count: Option<u32>,
        notes_count: Option<u32>,
        #[serde(flatten)]
        extra: HashMap<String, Value>,
    },
    #[serde(rename = "llm_response")]
    LlmResponse {
        turn: u32,
        response: String,
        state: Option<String>,
        retries: Option<u32>,
    },
    #[serde(rename = "command")]
    Command {
        turn: u32,
        cmd: String,
        success: bool,
        output_length: Option<usize>,
        #[serde(flatten)]
        extra: HashMap<String, Value>,
    },
    #[serde(rename = "session_end")]
    SessionEnd {
        duration_seconds: Option<u64>,
        total_turns: Option<u32>,
    },
    #[serde(rename = "max_turns_reached")]
    MaxTurnsReached { turn: u32 },
    #[serde(other)]
    Unknown,
}

/// Parsed moss agent session.
/// Used by Lua bindings and future session listing features.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct MossAgentSession {
    pub session_id: String,
    pub timestamp: Option<String>,
    pub prompt: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    pub turns: u32,
    pub commands: Vec<CommandInfo>,
    pub completed: bool,
    pub max_turns_hit: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct CommandInfo {
    pub cmd: String,
    pub success: bool,
    pub turn: u32,
}

#[allow(dead_code)]
impl MossAgentSession {
    /// Parse a session from a log file path.
    pub fn parse(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let reader = BufReader::new(file);
        let mut session = Self::default();

        for line in reader.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<AgentEvent>(&line) {
                match event {
                    AgentEvent::SessionStart {
                        session_id,
                        timestamp,
                        ..
                    } => {
                        session.session_id = session_id;
                        session.timestamp = Some(timestamp);
                    }
                    AgentEvent::Task {
                        user_prompt,
                        provider,
                        model,
                        role,
                        ..
                    } => {
                        session.prompt = Some(user_prompt);
                        session.provider = provider;
                        session.model = model;
                        session.role = role;
                    }
                    AgentEvent::TurnStart { turn, .. } => {
                        session.turns = session.turns.max(turn);
                    }
                    AgentEvent::Command {
                        cmd, success, turn, ..
                    } => {
                        session.commands.push(CommandInfo { cmd, success, turn });
                    }
                    AgentEvent::SessionEnd { .. } => {
                        session.completed = true;
                    }
                    AgentEvent::MaxTurnsReached { .. } => {
                        session.max_turns_hit = true;
                    }
                    _ => {}
                }
            }
        }

        if session.session_id.is_empty() {
            return None;
        }
        Some(session)
    }
}

impl LogFormat for MossAgentFormat {
    fn name(&self) -> &'static str {
        "moss"
    }

    fn sessions_dir(&self, project: Option<&Path>) -> PathBuf {
        let project_root = project
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        project_root.join(".moss/agent/logs")
    }

    fn list_sessions(&self, project: Option<&Path>) -> Vec<SessionFile> {
        let dir = self.sessions_dir(project);
        list_jsonl_sessions(&dir)
    }

    fn detect(&self, path: &Path) -> f64 {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "jsonl" {
            return 0.0;
        }

        // Peek at first few lines for moss agent events
        for line in peek_lines(path, 3) {
            if let Ok(entry) = serde_json::from_str::<Value>(&line) {
                // Moss agent logs have "event" field
                if let Some(event) = entry.get("event").and_then(|v| v.as_str()) {
                    if matches!(event, "session_start" | "task" | "turn_start") {
                        // Check for moss-specific fields
                        if entry.get("moss_root").is_some()
                            || entry.get("user_prompt").is_some()
                            || entry.get("working_memory_count").is_some()
                        {
                            return 1.0;
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
        let mut commands_by_name: HashMap<String, ToolStats> = HashMap::new();
        let mut total_turns = 0u32;

        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(event) = serde_json::from_str::<AgentEvent>(&line) {
                match event {
                    AgentEvent::SessionStart { .. } => {
                        *analysis
                            .message_counts
                            .entry("session_start".into())
                            .or_insert(0) += 1;
                    }
                    AgentEvent::Task { .. } => {
                        *analysis.message_counts.entry("task".into()).or_insert(0) += 1;
                    }
                    AgentEvent::TurnStart { turn, state, .. } => {
                        total_turns = total_turns.max(turn);
                        let state_key = state.unwrap_or_else(|| "unknown".into());
                        *analysis
                            .message_counts
                            .entry(format!("turn:{}", state_key))
                            .or_insert(0) += 1;
                    }
                    AgentEvent::LlmResponse { .. } => {
                        *analysis
                            .message_counts
                            .entry("llm_response".into())
                            .or_insert(0) += 1;
                    }
                    AgentEvent::Command { cmd, success, .. } => {
                        // Extract command name (first word)
                        let cmd_name = cmd.split_whitespace().next().unwrap_or("unknown");
                        let stat = commands_by_name
                            .entry(cmd_name.to_string())
                            .or_insert_with(|| ToolStats::new(cmd_name));
                        stat.calls += 1;
                        if !success {
                            stat.errors += 1;
                        }
                    }
                    AgentEvent::SessionEnd { .. } => {
                        *analysis
                            .message_counts
                            .entry("session_end".into())
                            .or_insert(0) += 1;
                    }
                    AgentEvent::MaxTurnsReached { .. } => {
                        *analysis
                            .message_counts
                            .entry("max_turns_reached".into())
                            .or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }

        analysis.total_turns = total_turns as usize;
        analysis.tool_stats = commands_by_name;

        Ok(analysis)
    }
}
