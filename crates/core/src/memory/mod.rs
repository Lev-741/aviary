use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use crate::error::Result;
use crate::types::{ChatMessage, Role};

pub const DEFAULT_HISTORY_LIMIT: usize = 10;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_user_id ON messages (user_id, id);
";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredMessage {
    pub id: i64,
    pub role: Role,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub last_message: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct Memory {
    pool: SqlitePool,
    limit: usize,
}

impl Memory {
    pub async fn open(url: &str) -> Result<Self> {
        let in_memory = url.contains(":memory:") || url.contains("mode=memory");
        let mut options = SqliteConnectOptions::from_str(url)?.create_if_missing(true);
        if !in_memory {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(if in_memory { 1 } else { 4 })
            .connect_with(options)
            .await?;
        sqlx::raw_sql(SCHEMA).execute(&pool).await?;
        Ok(Self {
            pool,
            limit: DEFAULT_HISTORY_LIMIT,
        })
    }

    pub async fn in_memory() -> Result<Self> {
        Self::open("sqlite::memory:").await
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(2);
        self
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub async fn append_turn(&self, user_id: &str, user: &str, assistant: &str) -> Result<()> {
        let now = now();
        let mut tx = self.pool.begin().await?;
        for (role, content) in [(Role::User, user), (Role::Assistant, assistant)] {
            sqlx::query(
                "INSERT INTO messages (user_id, role, content, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(user_id)
            .bind(role.as_str())
            .bind(content)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "DELETE FROM messages WHERE user_id = ? AND id NOT IN \
             (SELECT id FROM messages WHERE user_id = ? ORDER BY id DESC LIMIT ?)",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(self.limit as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn stored(&self, user_id: &str) -> Result<Vec<StoredMessage>> {
        let rows = sqlx::query(
            "SELECT id, role, content, created_at FROM \
             (SELECT * FROM messages WHERE user_id = ? ORDER BY id DESC LIMIT ?) ORDER BY id ASC",
        )
        .bind(user_id)
        .bind(self.limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut messages: Vec<StoredMessage> = rows
            .into_iter()
            .filter_map(|row| {
                let role = Role::parse(row.get::<&str, _>("role"))?;
                Some(StoredMessage {
                    id: row.get("id"),
                    role,
                    content: row.get("content"),
                    created_at: row.get("created_at"),
                })
            })
            .collect();
        let first_user = messages
            .iter()
            .position(|m| m.role == Role::User)
            .unwrap_or(messages.len());
        messages.drain(..first_user);
        Ok(messages)
    }

    pub async fn recent(&self, user_id: &str) -> Result<Vec<ChatMessage>> {
        Ok(self
            .stored(user_id)
            .await?
            .into_iter()
            .map(|m| ChatMessage::new(m.role, m.content))
            .collect())
    }

    pub async fn clear(&self, user_id: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM messages WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn users(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT DISTINCT user_id FROM messages ORDER BY user_id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.get("user_id")).collect())
    }

    pub async fn conversations(&self, prefix: &str) -> Result<Vec<Conversation>> {
        let pattern = format!(
            "{}%",
            prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let rows = sqlx::query(
            "SELECT m.user_id AS id, \
                    (SELECT content FROM messages f WHERE f.user_id = m.user_id AND f.role = 'user' \
                     ORDER BY f.id ASC LIMIT 1) AS title, \
                    (SELECT content FROM messages l WHERE l.user_id = m.user_id \
                     ORDER BY l.id DESC LIMIT 1) AS last_message, \
                    MAX(m.created_at) AS updated_at \
             FROM messages m WHERE m.user_id LIKE ? ESCAPE '\\' \
             GROUP BY m.user_id ORDER BY MAX(m.id) DESC",
        )
        .bind(pattern)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| Conversation {
                id: row.get("id"),
                title: row.get::<Option<String>, _>("title").unwrap_or_default(),
                last_message: row
                    .get::<Option<String>, _>("last_message")
                    .unwrap_or_default(),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keeps_last_ten_messages_per_user() {
        let memory = Memory::in_memory().await.unwrap();
        for i in 0..8 {
            memory
                .append_turn("alice", &format!("q{i}"), &format!("a{i}"))
                .await
                .unwrap();
        }
        memory.append_turn("bob", "hello", "hi").await.unwrap();

        let alice = memory.recent("alice").await.unwrap();
        assert_eq!(alice.len(), 10);
        assert_eq!(alice[0].content, "q3");
        assert_eq!(alice[0].role, Role::User);
        assert_eq!(alice[9].content, "a7");

        let bob = memory.recent("bob").await.unwrap();
        assert_eq!(bob.len(), 2);
        assert_eq!(memory.users().await.unwrap(), vec!["alice", "bob"]);
    }

    #[tokio::test]
    async fn odd_limit_never_starts_with_assistant() {
        let memory = Memory::in_memory().await.unwrap().with_limit(3);
        memory.append_turn("u", "q1", "a1").await.unwrap();
        memory.append_turn("u", "q2", "a2").await.unwrap();
        let history = memory.recent("u").await.unwrap();
        assert_eq!(history[0].role, Role::User);
        assert_eq!(history[0].content, "q2");
    }

    #[tokio::test]
    async fn clear_removes_only_one_user() {
        let memory = Memory::in_memory().await.unwrap();
        memory.append_turn("a", "1", "2").await.unwrap();
        memory.append_turn("b", "3", "4").await.unwrap();
        assert_eq!(memory.clear("a").await.unwrap(), 2);
        assert!(memory.recent("a").await.unwrap().is_empty());
        assert_eq!(memory.recent("b").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn lists_conversations_by_prefix_newest_first() {
        let memory = Memory::in_memory().await.unwrap();
        memory
            .append_turn("desktop:1", "first chat", "a")
            .await
            .unwrap();
        memory.append_turn("tg:9", "telegram", "b").await.unwrap();
        memory
            .append_turn("desktop:2", "second chat", "c")
            .await
            .unwrap();
        memory
            .append_turn("desktop:1", "follow up", "d")
            .await
            .unwrap();

        let list = memory.conversations("desktop:").await.unwrap();
        let ids: Vec<_> = list.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["desktop:1", "desktop:2"]);
        assert_eq!(list[0].title, "first chat");
        assert_eq!(list[0].last_message, "d");
    }

    #[tokio::test]
    async fn persists_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("aviary.db").display());
        {
            let memory = Memory::open(&url).await.unwrap();
            memory.append_turn("u", "q", "a").await.unwrap();
            memory.close().await;
        }
        let memory = Memory::open(&url).await.unwrap();
        assert_eq!(memory.recent("u").await.unwrap().len(), 2);
    }
}
