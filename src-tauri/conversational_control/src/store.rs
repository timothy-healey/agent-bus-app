//! ConversationStore — persistence for the Conversation aggregate over the
//! EXISTING `conversations` table (migration 001; D7 — no new migration). The
//! singleton-per-project conversation is keyed by project_id; turns serialise
//! into `history_json`.

use crate::conversation::Conversation;
use crate::turn::Turn;
use sqlx::{Row, SqlitePool};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sql error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub struct ConversationStore {
    pool: SqlitePool,
}

impl ConversationStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Load the project's conversation, or None if it has never been saved.
    pub async fn load(&self, project_id: &str) -> Result<Option<Conversation>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, started_at, last_message_at, history_json, summary_of_prior_sessions
             FROM conversations WHERE project_id = ? LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let turns: Vec<Turn> = serde_json::from_str(row.get::<String, _>("history_json").as_str())?;
        Ok(Some(Conversation {
            project_id: row.get("project_id"),
            session_id: row.get("id"),
            started_at: row.get("started_at"),
            last_message_at: row.get("last_message_at"),
            turns,
            summary_of_prior_sessions: row.get("summary_of_prior_sessions"),
            history_budget_tokens: crate::conversation::DEFAULT_HISTORY_BUDGET_TOKENS,
        }))
    }

    /// Insert-or-update the project's conversation row (keyed on project_id; the
    /// PK `id` carries the current session_id). Upsert via delete+insert keeps
    /// the singleton-per-project invariant simple.
    pub async fn save(&self, c: &Conversation) -> Result<(), StoreError> {
        let history = serde_json::to_string(&c.turns)?;
        sqlx::query("DELETE FROM conversations WHERE project_id = ?")
            .bind(&c.project_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO conversations
               (id, project_id, started_at, last_message_at, history_json, summary_of_prior_sessions)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.session_id)
        .bind(&c.project_id)
        .bind(c.started_at)
        .bind(c.last_message_at)
        .bind(history)
        .bind(&c.summary_of_prior_sessions)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::Turn;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        // Use the EXISTING migration 001 (D7) — no new migration for Plan 6.
        sqlx::query(include_str!("../../app/migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        // FK to projects: insert a project row so the FK is satisfiable.
        sqlx::query("INSERT INTO projects (id, name, root_path, created_at, updated_at) VALUES ('p','n','/p',0,0)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn load_returns_none_before_first_save() {
        let pool = fresh_pool().await;
        let store = ConversationStore::new(pool);
        assert!(store.load("p").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn save_then_load_round_trips_turns() {
        let pool = fresh_pool().await;
        let store = ConversationStore::new(pool);
        let mut c = Conversation::new("p", "sess-1", 100);
        c.append(Turn::user("/inject x", 100)).unwrap();
        c.append(Turn::assistant("Done: inject_topic.", vec![], 101)).unwrap();
        store.save(&c).await.unwrap();

        let back = store.load("p").await.unwrap().unwrap();
        assert_eq!(back.session_id, "sess-1");
        assert_eq!(back.turns.len(), 2);
        assert_eq!(back.last_message_at, 101);
        assert_eq!(back.turns[0].text, "/inject x");
    }

    #[tokio::test]
    async fn save_is_idempotent_on_project_id() {
        let pool = fresh_pool().await;
        let store = ConversationStore::new(pool.clone());
        let c1 = Conversation::new("p", "sess-1", 100);
        store.save(&c1).await.unwrap();
        let mut c2 = Conversation::new("p", "sess-2", 200);
        c2.append(Turn::user("hi", 200)).unwrap();
        store.save(&c2).await.unwrap();

        // exactly one row per project; the latest save wins
        let n: i64 = sqlx::query("SELECT COUNT(*) AS n FROM conversations WHERE project_id='p'")
            .fetch_one(&pool).await.unwrap().get("n");
        assert_eq!(n, 1);
        assert_eq!(store.load("p").await.unwrap().unwrap().session_id, "sess-2");
    }
}
