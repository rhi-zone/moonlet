//! Memory system for workflow context.
//!
//! Simple key-value store with metadata, queryable from Lua.
//! No special slot support - that's a user-space concern.

use std::path::Path;

use rusqlite::{Connection, params};

/// Convert a dot-notation key to a safe JSON path.
/// SQLite uses $."key" for quoted keys, $.key1.key2 for nested.
/// "author.name" -> $.author.name (safe chars) or $."author"."name" (quoted)
/// "slot" -> $.slot or $."slot"
fn key_to_json_path(key: &str) -> String {
    let segments: Vec<&str> = key.split('.').collect();
    let escaped: Vec<String> = segments
        .iter()
        .map(|s| {
            // If key contains only safe chars, use unquoted
            if s.chars().all(|c| c.is_alphanumeric() || c == '_') {
                s.to_string()
            } else {
                // Quote and escape
                format!("\"{}\"", escape_json_key(s))
            }
        })
        .collect();
    format!("$.{}", escaped.join("."))
}

/// Escape a string for use in a JSON path key.
fn escape_json_key(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Memory store backed by SQLite.
pub struct MemoryStore {
    conn: Connection,
}

/// A memory item with content and metadata.
#[derive(Debug, Clone)]
pub struct MemoryItem {
    pub id: i64,
    pub content: String,
    pub context: Option<String>,
    pub weight: f64,
    pub created_at: i64,
    pub accessed_at: i64,
    /// Arbitrary metadata as JSON
    pub metadata: String,
}

impl MemoryStore {
    /// Open or create memory store at the given path.
    pub fn open(root: &Path) -> Result<Self, rusqlite::Error> {
        let db_path = root.join(".spore").join("memory.db");

        // Ensure .spore directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory (
                id INTEGER PRIMARY KEY,
                content TEXT NOT NULL,
                context TEXT,
                weight REAL DEFAULT 1.0,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                accessed_at INTEGER DEFAULT (strftime('%s', 'now')),
                metadata TEXT DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_memory_context ON memory(context);
            CREATE INDEX IF NOT EXISTS idx_memory_weight ON memory(weight DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_accessed ON memory(accessed_at DESC);
            ",
        )?;

        Ok(Self { conn })
    }

    /// Store content with optional metadata.
    pub fn store(
        &self,
        content: &str,
        context: Option<&str>,
        weight: Option<f64>,
        metadata: Option<&str>,
    ) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO memory (content, context, weight, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![
                content,
                context,
                weight.unwrap_or(1.0),
                metadata.unwrap_or("{}")
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Recall items matching a query.
    ///
    /// Query can match:
    /// - content (substring match)
    /// - context (exact or substring)
    /// - metadata keys (via JSON)
    ///
    /// Results ordered by weight DESC, accessed_at DESC.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryItem>, rusqlite::Error> {
        // Simple query: match content or context
        let mut stmt = self.conn.prepare(
            "SELECT id, content, context, weight, created_at, accessed_at, metadata
             FROM memory
             WHERE content LIKE ?1 OR context LIKE ?1 OR context = ?2
             ORDER BY weight DESC, accessed_at DESC
             LIMIT ?3",
        )?;

        let pattern = format!("%{}%", query);
        let items = stmt
            .query_map(params![&pattern, query, limit as i64], |row| {
                Ok(MemoryItem {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    context: row.get(2)?,
                    weight: row.get(3)?,
                    created_at: row.get(4)?,
                    accessed_at: row.get(5)?,
                    metadata: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Update accessed_at for returned items
        for item in &items {
            self.conn.execute(
                "UPDATE memory SET accessed_at = strftime('%s', 'now') WHERE id = ?1",
                params![item.id],
            )?;
        }

        Ok(items)
    }

    /// Recall by metadata key-value matches (AND semantics).
    /// Keys use dot notation for nested paths: "author.name" -> $["author"]["name"]
    pub fn recall_by_metadata(
        &self,
        filters: &[(&str, &str)],
        limit: usize,
    ) -> Result<Vec<MemoryItem>, rusqlite::Error> {
        if filters.is_empty() {
            return Ok(Vec::new());
        }

        // Build WHERE clause with AND for each filter
        // Use bracket notation with escaped keys to prevent injection
        let conditions: Vec<String> = filters
            .iter()
            .enumerate()
            .map(|(i, (key, _))| {
                let json_path = key_to_json_path(key);
                format!("json_extract(metadata, '{}') = ?{}", json_path, i + 1)
            })
            .collect();

        let query = format!(
            "SELECT id, content, context, weight, created_at, accessed_at, metadata
             FROM memory
             WHERE {}
             ORDER BY weight DESC, accessed_at DESC
             LIMIT ?{}",
            conditions.join(" AND "),
            filters.len() + 1
        );

        let mut stmt = self.conn.prepare(&query)?;

        // Bind filter values and limit
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = filters
            .iter()
            .map(|(_, v)| Box::new(v.to_string()) as Box<dyn rusqlite::ToSql>)
            .collect();
        params.push(Box::new(limit as i64));

        let items = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| {
                    Ok(MemoryItem {
                        id: row.get(0)?,
                        content: row.get(1)?,
                        context: row.get(2)?,
                        weight: row.get(3)?,
                        created_at: row.get(4)?,
                        accessed_at: row.get(5)?,
                        metadata: row.get(6)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(items)
    }

    /// Forget (delete) items matching a query.
    pub fn forget(&self, query: &str) -> Result<usize, rusqlite::Error> {
        let pattern = format!("%{}%", query);
        let count = self.conn.execute(
            "DELETE FROM memory WHERE content LIKE ?1 OR context LIKE ?1 OR context = ?2",
            params![&pattern, query],
        )?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_recall() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path()).unwrap();

        // Store some items
        store
            .store("User prefers tabs", Some("formatting"), Some(1.0), None)
            .unwrap();
        store
            .store(
                "auth.py broke tests last time",
                Some("auth.py"),
                Some(0.8),
                None,
            )
            .unwrap();
        store
            .store("Project uses Rust", Some("general"), Some(0.5), None)
            .unwrap();

        // Recall by content
        let items = store.recall("tabs", 10).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].content.contains("tabs"));

        // Recall by context
        let items = store.recall("auth.py", 10).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].content.contains("auth.py"));
    }

    #[test]
    fn test_recall_ordering() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path()).unwrap();

        // Store with different weights
        store
            .store("low weight", Some("test"), Some(0.1), None)
            .unwrap();
        store
            .store("high weight", Some("test"), Some(0.9), None)
            .unwrap();
        store
            .store("medium weight", Some("test"), Some(0.5), None)
            .unwrap();

        let items = store.recall("test", 10).unwrap();
        assert_eq!(items.len(), 3);
        assert!(items[0].content.contains("high"));
        assert!(items[1].content.contains("medium"));
        assert!(items[2].content.contains("low"));
    }

    #[test]
    fn test_forget() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path()).unwrap();

        store
            .store("remember this", Some("ctx"), None, None)
            .unwrap();
        store.store("forget this", Some("ctx"), None, None).unwrap();

        let count = store.forget("forget").unwrap();
        assert_eq!(count, 1);

        let items = store.recall("", 10).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].content.contains("remember"));
    }

    #[test]
    fn test_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = MemoryStore::open(tmp.path()).unwrap();

        store
            .store("system prompt", None, None, Some(r#"{"slot": "system"}"#))
            .unwrap();
        store
            .store("user pref", None, None, Some(r#"{"slot": "preferences"}"#))
            .unwrap();

        let items = store.recall_by_metadata(&[("slot", "system")], 10).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].content.contains("system prompt"));
    }
}
