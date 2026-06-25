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
            "INSERT INTO projects (id, name, root_path, target_repo, skill_sources, active_pipeline_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&project.id.0)
        .bind(&project.name)
        .bind(project.root_path.to_string_lossy().to_string())
        .bind(project.target_repo.as_ref())
        .bind(encode_skill_sources(&project.skill_sources))
        .bind(project.active_pipeline_id.as_ref().map(|p| &p.0))
        .bind(project.created_at)
        .bind(project.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<Project>, ProjectStoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, target_repo, skill_sources, active_pipeline_id, created_at, updated_at
             FROM projects ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(id, name, root_path, target_repo, skill_sources, active, created, updated)| Project {
            id: ProjectId(id),
            name,
            root_path: root_path.into(),
            target_repo,
            skill_sources: decode_skill_sources(skill_sources.as_deref()),
            active_pipeline_id: active.map(PipelineId),
            created_at: created,
            updated_at: updated,
        }).collect())
    }

    pub async fn get(&self, id: &ProjectId) -> Result<Project, ProjectStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, Option<String>, i64, i64)>(
            "SELECT id, name, root_path, target_repo, skill_sources, active_pipeline_id, created_at, updated_at
             FROM projects WHERE id = ?",
        )
        .bind(&id.0)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((id, name, root_path, target_repo, skill_sources, active, created, updated)) => Ok(Project {
                id: ProjectId(id),
                name,
                root_path: root_path.into(),
                target_repo,
                skill_sources: decode_skill_sources(skill_sources.as_deref()),
                active_pipeline_id: active.map(PipelineId),
                created_at: created,
                updated_at: updated,
            }),
            None => Err(ProjectStoreError::NotFound(id.clone())),
        }
    }

    pub async fn set_active_pipeline(
        &self,
        id: &ProjectId,
        pipeline_id: Option<&PipelineId>,
        now_unix: i64,
    ) -> Result<(), ProjectStoreError> {
        let result = sqlx::query(
            "UPDATE projects SET active_pipeline_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(pipeline_id.map(|p| &p.0))
        .bind(now_unix)
        .bind(&id.0)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(ProjectStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Set (or clear) a project's target repo (A5). `None` clears it. Returns
    /// NotFound when the id does not exist.
    pub async fn set_target_repo(
        &self,
        id: &ProjectId,
        target_repo: Option<&str>,
        now_unix: i64,
    ) -> Result<(), ProjectStoreError> {
        let result = sqlx::query(
            "UPDATE projects SET target_repo = ?, updated_at = ? WHERE id = ?",
        )
        .bind(target_repo)
        .bind(now_unix)
        .bind(&id.0)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(ProjectStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Set a project's skill sources (A4). Stored JSON-encoded; an empty list is
    /// stored as `null` (= global only). Returns NotFound when the id is unknown.
    pub async fn set_skill_sources(
        &self,
        id: &ProjectId,
        sources: &[String],
        now_unix: i64,
    ) -> Result<(), ProjectStoreError> {
        let result = sqlx::query(
            "UPDATE projects SET skill_sources = ?, updated_at = ? WHERE id = ?",
        )
        .bind(encode_skill_sources(sources))
        .bind(now_unix)
        .bind(&id.0)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(ProjectStoreError::NotFound(id.clone()));
        }
        Ok(())
    }

    /// Remove a project row. Returns NotFound when the id does not exist.
    /// Deletes only the row — on-disk artifacts under the project root are NOT
    /// touched (Workspace owns the registry, not a destructive filesystem wipe).
    pub async fn remove(&self, id: &ProjectId) -> Result<(), ProjectStoreError> {
        // FK-safe cascade: many tables reference a project directly or
        // transitively, and the runtime pool runs with `PRAGMA foreign_keys = ON`
        // (sqlx's default), so a bare `DELETE FROM projects` is blocked with FK
        // 787 whenever the project has any children. Delete the whole subtree in
        // a single transaction, leaf-first, then the project itself. Subqueries
        // (not row-by-row) so it works regardless of how many children exist.
        let mut tx = self.pool.begin().await?;

        // Leaves first, walking up the FK chain.

        // comments → tasks → projects
        sqlx::query("DELETE FROM comments WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?)")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        // invocation_audit → tasks → projects (no declared FK, cascaded for safety)
        sqlx::query("DELETE FROM invocation_audit WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?)")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        // workers reference tasks (nullable, no declared FK). Delete the project's
        // worker rows outright (before the tasks they point at) so deleting a
        // project leaves ZERO worker remnants — an idle worker tied to a
        // just-deleted project's task has no reason to survive.
        sqlx::query("DELETE FROM workers WHERE task_id IN (SELECT id FROM tasks WHERE project_id = ?)")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;

        // stores / generator_ledger → runs → projects (no declared FK)
        sqlx::query("DELETE FROM stores WHERE run_id IN (SELECT id FROM runs WHERE project_id = ?)")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM generator_ledger WHERE run_id IN (SELECT id FROM runs WHERE project_id = ?)")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;

        // Now the direct children of the project.
        sqlx::query("DELETE FROM tasks WHERE project_id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM runs WHERE project_id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM conversations WHERE project_id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;

        // Finally the project row itself.
        let result = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            // Roll back (drop) the empty transaction; nothing to commit.
            return Err(ProjectStoreError::NotFound(id.clone()));
        }

        tx.commit().await?;
        Ok(())
    }
}

/// Encode a skill-sources list for the TEXT column: an empty list → `None`
/// (stored as SQL NULL = global only); otherwise a JSON array string.
fn encode_skill_sources(sources: &[String]) -> Option<String> {
    if sources.is_empty() {
        None
    } else {
        serde_json::to_string(sources).ok()
    }
}

/// Decode the TEXT column back to a list. NULL / unparseable → empty (lenient:
/// a corrupt cell degrades to "global only", never an error).
fn decode_skill_sources(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
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
        sqlx::query(include_str!("../../app/migrations/010_project_target_repo.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/011_skill_sources.sql"))
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

    #[tokio::test]
    async fn set_active_pipeline_updates_the_column() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);

        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();

        store
            .set_active_pipeline(&p.id, Some(&PipelineId("ddd-spec-plan-impl".into())), 200)
            .await
            .unwrap();

        let reloaded = store.get(&p.id).await.unwrap();
        assert_eq!(
            reloaded.active_pipeline_id,
            Some(PipelineId("ddd-spec-plan-impl".into()))
        );
        assert_eq!(reloaded.updated_at, 200);
    }

    #[tokio::test]
    async fn set_target_repo_updates_the_column() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        store.set_target_repo(&p.id, Some("/repo"), 200).await.unwrap();
        let reloaded = store.get(&p.id).await.unwrap();
        assert_eq!(reloaded.target_repo, Some("/repo".to_string()));
        assert_eq!(reloaded.updated_at, 200);
    }

    #[tokio::test]
    async fn skill_sources_round_trip_through_insert_and_get() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let mut p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        p.skill_sources = vec!["/a/.claude".into(), "/b/.claude".into()];
        store.insert(&p).await.unwrap();
        let reloaded = store.get(&p.id).await.unwrap();
        assert_eq!(reloaded.skill_sources, vec!["/a/.claude".to_string(), "/b/.claude".into()]);
    }

    #[tokio::test]
    async fn empty_skill_sources_default_when_unset() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        let reloaded = store.get(&p.id).await.unwrap();
        assert!(reloaded.skill_sources.is_empty());
    }

    #[tokio::test]
    async fn set_skill_sources_updates_the_column() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        store.set_skill_sources(&p.id, &["/x/.claude".into()], 200).await.unwrap();
        let reloaded = store.get(&p.id).await.unwrap();
        assert_eq!(reloaded.skill_sources, vec!["/x/.claude".to_string()]);
        assert_eq!(reloaded.updated_at, 200);
        // Clearing back to empty stores NULL and decodes to empty.
        store.set_skill_sources(&p.id, &[], 300).await.unwrap();
        assert!(store.get(&p.id).await.unwrap().skill_sources.is_empty());
    }

    #[tokio::test]
    async fn set_skill_sources_on_missing_project_is_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let result = store.set_skill_sources(&ProjectId("nope".into()), &["/r".into()], 1).await;
        assert!(matches!(result, Err(ProjectStoreError::NotFound(_))));
    }

    #[test]
    fn encode_empty_is_none_decode_null_is_empty() {
        assert_eq!(encode_skill_sources(&[]), None);
        assert!(decode_skill_sources(None).is_empty());
        assert!(decode_skill_sources(Some("not json")).is_empty());
        let enc = encode_skill_sources(&["/p".to_string()]).unwrap();
        assert_eq!(decode_skill_sources(Some(&enc)), vec!["/p".to_string()]);
    }

    #[tokio::test]
    async fn set_target_repo_on_missing_project_is_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let result = store.set_target_repo(&ProjectId("nope".into()), Some("/r"), 1).await;
        assert!(matches!(result, Err(ProjectStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn remove_deletes_the_project() {
        // Full schema (and FKs on): the cascade references the child tables, so
        // they must exist for even the childless case.
        let pool = fresh_pool_full_schema().await;
        let store = ProjectStore::new(pool);
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        store.remove(&p.id).await.unwrap();
        assert!(matches!(store.get(&p.id).await, Err(ProjectStoreError::NotFound(_))));
    }

    /// A pool with the FULL child schema applied AND `PRAGMA foreign_keys = ON`,
    /// so the FK 787 the live app hit is actually enforced here. We enable the
    /// pragma explicitly (don't rely on the driver default) so the test provably
    /// exercises the constraint that broke the bare delete.
    async fn fresh_pool_full_schema() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .after_connect(|conn, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON;")
                        .execute(conn)
                        .await
                        .map(|_| ())
                })
            })
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            include_str!("../../app/migrations/001_initial.sql"),
            include_str!("../../app/migrations/003_runtime.sql"),
            include_str!("../../app/migrations/004_comments_kind.sql"),
            include_str!("../../app/migrations/006_fanout.sql"),
            include_str!("../../app/migrations/007_invocation_audit.sql"),
            include_str!("../../app/migrations/008_nested_groups.sql"),
            include_str!("../../app/migrations/010_project_target_repo.sql"),
            include_str!("../../app/migrations/011_skill_sources.sql"),
            include_str!("../../app/migrations/012_runtime_stores.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced_in_full_schema_pool() {
        // Guard: prove the test pool actually rejects an orphan insert, so the
        // cascade test below is meaningful (a non-enforcing pool would pass
        // even with the old buggy `remove`).
        let pool = fresh_pool_full_schema().await;
        let res = sqlx::query(
            "INSERT INTO conversations (id, project_id, started_at, last_message_at, history_json)
             VALUES ('c', 'no-such-project', 0, 0, '[]')",
        )
        .execute(&pool)
        .await;
        assert!(res.is_err(), "FKs must be enforced in the test pool");
    }

    #[tokio::test]
    async fn remove_cascades_children_in_a_transaction() {
        let pool = fresh_pool_full_schema().await;
        let store = ProjectStore::new(pool.clone());

        // A project with children spanning the whole FK chain.
        let p = Project::new("Demo".into(), "/tmp/demo".into(), 100);
        store.insert(&p).await.unwrap();
        let pid = &p.id.0;

        // conversation → project
        sqlx::query(
            "INSERT INTO conversations (id, project_id, started_at, last_message_at, history_json)
             VALUES ('conv1', ?, 0, 0, '[]')",
        )
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

        // task → project
        sqlx::query(
            "INSERT INTO tasks (id, project_id, pipeline, topic, current_stage, state, created_at, updated_at)
             VALUES ('task1', ?, 'pl', 'topic', 'stage', 'queued', 0, 0)",
        )
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();

        // comment → task
        sqlx::query(
            "INSERT INTO comments (id, task_id, artifact_path, note, created_at)
             VALUES ('cmt1', 'task1', '/a', 'n', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // invocation_audit → task
        sqlx::query(
            "INSERT INTO invocation_audit (invocation_id, task_id, team_id, model, started_at)
             VALUES ('inv1', 'task1', 'team', 'model', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // worker referencing the task
        sqlx::query(
            "INSERT INTO workers (id, team_id, task_id, started_at) VALUES ('w1', 'team', 'task1', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        // run → project, plus store + generator_ledger → run
        sqlx::query(
            "INSERT INTO runs (id, pipeline, project_id) VALUES ('run1', 'pl', ?)",
        )
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO stores (run_id, stage, capacity, occupancy) VALUES ('run1', 'st', 4, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO generator_ledger (run_id, stage, candidate_key) VALUES ('run1', 'st', 'k')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // The bug: with FKs on, the old bare DELETE would 787 here. The fix must
        // delete cleanly and leave no orphans.
        store.remove(&p.id).await.unwrap();

        // Project gone.
        assert!(matches!(store.get(&p.id).await, Err(ProjectStoreError::NotFound(_))));

        // No orphaned children anywhere in the chain — including the project's
        // worker rows, which are now DELETED (not nulled): deleting a project
        // leaves zero worker remnants.
        for (table, predicate) in [
            ("conversations", "1=1"),
            ("tasks", "1=1"),
            ("runs", "1=1"),
            ("comments", "task_id = 'task1'"),
            ("invocation_audit", "task_id = 'task1'"),
            ("stores", "run_id = 'run1'"),
            ("generator_ledger", "run_id = 'run1'"),
            ("workers", "id = 'w1'"),
        ] {
            let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(n, 0, "expected no rows left in {table}, found {n}");
        }
    }

    #[tokio::test]
    async fn remove_missing_is_not_found() {
        let pool = fresh_pool_full_schema().await;
        let store = ProjectStore::new(pool);
        assert!(matches!(
            store.remove(&ProjectId("nope".into())).await,
            Err(ProjectStoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn set_active_pipeline_on_missing_project_is_not_found() {
        let pool = fresh_pool().await;
        let store = ProjectStore::new(pool);
        let result = store
            .set_active_pipeline(&ProjectId("nope".into()), None, 1)
            .await;
        assert!(matches!(result, Err(ProjectStoreError::NotFound(_))));
    }
}
