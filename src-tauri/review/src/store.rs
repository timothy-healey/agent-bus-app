use crate::comment::{Comment, CommentKind, NewComment};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CommentStoreError {
    #[error("unknown comment kind in db: {0}")]
    BadKind(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Persistence for the Comment aggregate over the `comments` table
/// (migration 003 + 004).
pub struct CommentStore {
    pool: SqlitePool,
}

impl CommentStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        nc: NewComment,
        now_unix: i64,
    ) -> Result<Comment, CommentStoreError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO comments \
             (id, task_id, artifact_path, anchor_text, anchor_offset, note, kind, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&nc.task_id)
        .bind(&nc.artifact_path)
        .bind(&nc.anchor_text)
        .bind(nc.anchor_offset)
        .bind(&nc.note)
        .bind(nc.kind.as_str())
        .bind(now_unix)
        .execute(&self.pool)
        .await?;

        Ok(Comment {
            id,
            task_id: nc.task_id,
            artifact_path: nc.artifact_path,
            anchor_text: nc.anchor_text,
            anchor_offset: nc.anchor_offset,
            note: nc.note,
            kind: nc.kind,
            created_at: now_unix,
        })
    }

    pub async fn list_for_task(
        &self,
        task_id: &str,
    ) -> Result<Vec<Comment>, CommentStoreError> {
        let rows = sqlx::query(
            "SELECT id, task_id, artifact_path, anchor_text, anchor_offset, note, kind, created_at \
             FROM comments WHERE task_id = ? \
             ORDER BY created_at ASC, anchor_offset ASC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let kind_str: String = row.try_get("kind")?;
            let kind = CommentKind::parse(&kind_str)
                .ok_or_else(|| CommentStoreError::BadKind(kind_str.clone()))?;
            out.push(Comment {
                id: row.try_get("id")?,
                task_id: row.try_get("task_id")?,
                artifact_path: row.try_get("artifact_path")?,
                anchor_text: row.try_get("anchor_text")?,
                anchor_offset: row.try_get("anchor_offset")?,
                note: row.try_get("note")?,
                kind,
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(out)
    }

    pub async fn delete(&self, id: &str) -> Result<(), CommentStoreError> {
        sqlx::query("DELETE FROM comments WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comment::{CommentKind, NewComment};

    async fn fresh_pool() -> sqlx::SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        // FK enforcement defaults ON in sqlx's sqlite driver; the sibling
        // runtime crate's test pools disable it so a store can be exercised
        // without seeding parent `tasks` rows. Same convention here.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .unwrap();
        for sql in [
            include_str!("../../app/migrations/001_initial.sql"),
            include_str!("../../app/migrations/003_runtime.sql"),
            include_str!("../../app/migrations/004_comments_kind.sql"),
        ] {
            for stmt in sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() {
                    sqlx::query(s).execute(&pool).await.unwrap();
                }
            }
        }
        pool
    }

    fn inline(task: &str, note: &str, offset: i64) -> NewComment {
        NewComment {
            task_id: task.into(),
            artifact_path: "artifacts/specs/T-1-v1.md".into(),
            anchor_text: Some("span".into()),
            anchor_offset: Some(offset),
            note: note.into(),
            kind: CommentKind::Inline,
        }
    }

    #[tokio::test]
    async fn insert_then_list_returns_the_comment() {
        let store = CommentStore::new(fresh_pool().await);
        let saved = store.insert(inline("T-1", "first", 10), 5000).await.unwrap();
        assert_eq!(saved.note, "first");
        assert!(!saved.id.is_empty());
        assert_eq!(saved.created_at, 5000);

        let listed = store.list_for_task("T-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved.id);
        assert_eq!(listed[0].kind, CommentKind::Inline);
    }

    #[tokio::test]
    async fn list_is_ordered_by_created_at_then_offset() {
        let store = CommentStore::new(fresh_pool().await);
        store.insert(inline("T-1", "b", 20), 5001).await.unwrap();
        store.insert(inline("T-1", "a", 10), 5000).await.unwrap();
        let listed = store.list_for_task("T-1").await.unwrap();
        assert_eq!(listed[0].note, "a");
        assert_eq!(listed[1].note, "b");
    }

    #[tokio::test]
    async fn direction_comment_round_trips_with_null_anchor() {
        let store = CommentStore::new(fresh_pool().await);
        let nc = NewComment {
            task_id: "T-1".into(),
            artifact_path: "artifacts/specs/T-1-v1.md".into(),
            anchor_text: None,
            anchor_offset: None,
            note: "overall".into(),
            kind: CommentKind::Direction,
        };
        store.insert(nc, 6000).await.unwrap();
        let listed = store.list_for_task("T-1").await.unwrap();
        assert_eq!(listed[0].kind, CommentKind::Direction);
        assert!(listed[0].anchor_text.is_none());
    }

    #[tokio::test]
    async fn delete_removes_only_that_comment() {
        let store = CommentStore::new(fresh_pool().await);
        let a = store.insert(inline("T-1", "a", 10), 5000).await.unwrap();
        store.insert(inline("T-1", "b", 20), 5001).await.unwrap();
        store.delete(&a.id).await.unwrap();
        let listed = store.list_for_task("T-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].note, "b");
    }
}
