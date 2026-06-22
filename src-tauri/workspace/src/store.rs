use crate::project::Project;
use agent_bus_core::{PipelineId, ProjectId};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectStoreError {
    #[error("project not found: {0}")]
    NotFound(ProjectId),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct ProjectStore {
    pool: SqlitePool,
}

impl ProjectStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, project: &Project) -> Result<(), ProjectStoreError> {
        sqlx::query(
            "INSERT INTO projects (id, name, root_path, active_pipeline_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&project.id.0)
        .bind(&project.name)
        .bind(project.root_path.to_string_lossy().to_string())
        .bind(project.active_pipeline_id.as_ref().map(|p| &p.0))
        .bind(project.created_at)
        .bind(project.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Project>, ProjectStoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, active_pipeline_id, created_at, updated_at
             FROM projects ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(id, name, root_path, active, created, updated)| Project {
            id: ProjectId(id),
            name,
            root_path: root_path.into(),
            active_pipeline_id: active.map(PipelineId),
            created_at: created,
            updated_at: updated,
        }).collect())
    }

    pub async fn get(&self, id: &ProjectId) -> Result<Project, ProjectStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, active_pipeline_id, created_at, updated_at
             FROM projects WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((id, name, root_path, active, created, updated)) => Ok(Project {
                id: ProjectId(id),
                name,
                root_path: root_path.into(),
                active_pipeline_id: active.map(PipelineId),
                created_at: created,
                updated_at: updated,
            }),
            None => Err(ProjectStoreError::NotFound(id.clone())),
        }
    }
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
        sqlx::query(include_str!("../../app/migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_and_list_round_trip() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let p = Project::new("Demo".into(), "/tmp/demo".into(), 1_700_000_000);
        store.insert(&p).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], p);
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let result = store.get(&ProjectId("nope".into())).await;
        assert!(matches!(result, Err(ProjectStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_orders_by_created_at_desc() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let mut earlier = Project::new("First".into(), "/p1".into(), 100);
        let later = Project::new("Second".into(), "/p2".into(), 200);
        earlier.updated_at = 100;
        store.insert(&earlier).await.unwrap();
        store.insert(&later).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed[0].name, "Second");
        assert_eq!(listed[1].name, "First");
    }
}
