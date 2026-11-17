//! Owner-supplied classification for registry reads from the offline replica.

use projects_api::reads::{ProjectListResponse, ProjectReadResponse, RecordRead};

use super::{StoreError, StoreResult, TursoStore};

const PROJECTS: &str = "registry:projects";

fn unreadable(error: impl std::fmt::Display) -> StoreError {
    StoreError::Libsql(format!("registry replica: {error}"))
}

impl TursoStore {
    pub(crate) async fn forget_registry_record(&self, path: &str) -> StoreResult<()> {
        self.connection()
            .await?
            .execute(
                "UPDATE projects_records SET registry_class = NULL, registry_rationale = NULL, \
             registry_archived = NULL WHERE path = ?1",
                [path],
            )
            .await
            .map_err(unreadable)?;
        Ok(())
    }
    pub(crate) async fn archive_registry_record(&self, path: &str) -> StoreResult<()> {
        self.connection()
            .await?
            .execute(
                "UPDATE projects_records SET registry_archived = 1 WHERE path = ?1",
                [path],
            )
            .await
            .map_err(unreadable)?;
        Ok(())
    }
    pub(crate) async fn cache_registry_projects(
        &self,
        response: &ProjectListResponse,
    ) -> StoreResult<()> {
        self.set_meta(
            PROJECTS,
            &serde_json::to_string(response).map_err(unreadable)?,
        )
        .await
    }

    pub(crate) async fn cache_registry_record(
        &self,
        project: &str,
        record: &RecordRead,
    ) -> StoreResult<()> {
        if !matches!(record.data_class.as_str(), "c0" | "c1" | "c2" | "c3") {
            self.forget_registry_record(&record.path).await?;
            return Err(unreadable("invalid owner classification"));
        }
        self.connection()
            .await?
            .execute(
                "UPDATE projects_records SET registry_class = ?1, registry_rationale = ?2, \
                 registry_archived = ?3, project = ?6 WHERE path = ?4 AND remote_at = ?5",
                libsql::params![
                    record.data_class.clone(),
                    record.data_class_rationale.clone(),
                    i64::from(record.archived),
                    record.path.clone(),
                    record.updated_at.clone(),
                    project
                ],
            )
            .await
            .map_err(unreadable)?;
        Ok(())
    }

    pub(crate) async fn registry_projects(&self) -> StoreResult<ProjectListResponse> {
        if !self.dirty_paths().await?.is_empty() {
            return Err(unreadable(
                "synchronize queued project edits before browsing objects",
            ));
        }
        let cached = self
            .meta(PROJECTS)
            .await?
            .ok_or_else(|| unreadable("synchronize Projects to obtain classification metadata"))?;
        serde_json::from_str(&cached).map_err(unreadable)
    }

    pub(crate) async fn registry_records(&self, project: &str) -> StoreResult<ProjectReadResponse> {
        let projects = self.registry_projects().await?;
        if !projects.projects.iter().any(|p| p.summary.name == project) {
            return Err(unreadable("project is not in this replica"));
        }
        let mut rows = self
            .connection()
            .await?
            .query(
                "SELECT path, kind, title, done, due, priority, registry_archived, registry_class, \
                 registry_rationale, remote_at FROM projects_records \
                 WHERE project = ?1 AND (registry_archived IS NULL OR registry_archived = 0) ORDER BY path",
                [project],
            )
            .await
            .map_err(unreadable)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().await.map_err(unreadable)? {
            let archived: i64 = row.get(6).map_err(|_| {
                unreadable("synchronize Projects to obtain record classification metadata")
            })?;
            records.push(RecordRead {
                path: row.get(0).map_err(unreadable)?,
                kind: serde_json::from_value(serde_json::json!(row
                    .get::<String>(1)
                    .map_err(unreadable)?))
                .map_err(unreadable)?,
                title: row.get(2).map_err(unreadable)?,
                done: row.get::<i64>(3).map_err(unreadable)? != 0,
                due: row.get(4).map_err(unreadable)?,
                priority: u8::try_from(row.get::<i64>(5).map_err(unreadable)?)
                    .map_err(unreadable)?,
                archived: archived != 0,
                data_class: row.get(7).map_err(unreadable)?,
                data_class_rationale: row.get(8).map_err(unreadable)?,
                updated_at: row.get(9).map_err(unreadable)?,
                content: None,
            });
        }
        Ok(ProjectReadResponse { records })
    }
}
