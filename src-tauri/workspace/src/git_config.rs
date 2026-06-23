//! Git author identity config — the name/email used for commits workers make in
//! worktrees. Single-row store (id=1), mirrors usage_config. Persisted in v1.1;
//! the worktree-commit path that reads it lands in a later item.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitConfig {
    pub author_name: String,
    pub author_email: String,
}

pub struct GitConfigState {
    pub pool: SqlitePool,
}

pub async fn load_git_config(pool: &SqlitePool) -> GitConfig {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT author_name, author_email FROM git_config WHERE id = 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row {
        Some((author_name, author_email)) => GitConfig {
            author_name,
            author_email,
        },
        None => GitConfig::default(),
    }
}

pub async fn set_git_config_inner(
    pool: &SqlitePool,
    author_name: &str,
    author_email: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE git_config SET author_name = ?, author_email = ? WHERE id = 1")
        .bind(author_name)
        .bind(author_email)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn git_config_get(state: tauri::State<'_, GitConfigState>) -> Result<GitConfig, String> {
    Ok(load_git_config(&state.pool).await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn git_config_set(
    state: tauri::State<'_, GitConfigState>,
    author_name: String,
    author_email: String,
) -> Result<GitConfig, String> {
    set_git_config_inner(&state.pool, &author_name, &author_email).await?;
    Ok(load_git_config(&state.pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/009_git_config.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn defaults_to_empty() {
        let pool = fresh_pool().await;
        assert_eq!(load_git_config(&pool).await, GitConfig::default());
    }

    #[tokio::test]
    async fn set_then_load_round_trip() {
        let pool = fresh_pool().await;
        set_git_config_inner(&pool, "Ada Lovelace", "ada@example.com")
            .await
            .unwrap();
        let cfg = load_git_config(&pool).await;
        assert_eq!(cfg.author_name, "Ada Lovelace");
        assert_eq!(cfg.author_email, "ada@example.com");
    }
}
