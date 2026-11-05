//! Projects, Records, Notes & Tasks shell client methods.

use super::*;

impl Shell {
    pub async fn project_inventory_links(
        &self,
        path: &str,
    ) -> ShellResult<Vec<ProjectInventoryLink>> {
        #[derive(Deserialize)]
        struct Envelope {
            links: Vec<ProjectInventoryLink>,
        }
        let path = with_query("/projects/api/inventory-links", &[("path", path)]);
        let result: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(result.links)
    }

    pub async fn project_link_record(&self, path: &str) -> ShellResult<ProjectLinkRecord> {
        #[derive(Deserialize)]
        struct Envelope {
            record: ProjectLinkRecord,
        }
        let path = with_query("/projects/api/record", &[("path", path)]);
        let result: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(result.record)
    }

    pub async fn project_inventory_link_decision(
        &self,
        remove: bool,
        project: &str,
        record: &ProjectLinkRecord,
        item_ref: &str,
    ) -> ShellResult<bool> {
        #[derive(Deserialize)]
        struct Result {
            changed: bool,
        }
        let path = if remove {
            "/projects/api/inventory-links/remove"
        } else {
            "/projects/api/inventory-links"
        };
        let result: Result = self.json(Method::POST, path, Some(serde_json::json!({"project": project, "path": record.path, "base": record.content, "updated_at": record.updated_at, "item_ref": item_ref, "reviewed": true})), 10).await?;
        Ok(result.changed)
    }

    pub async fn project_mail_proposals(
        &self,
        before: Option<i64>,
    ) -> ShellResult<ProjectMailPage> {
        let path = before.map_or_else(
            || "/projects/api/mail-proposals".to_owned(),
            |id| format!("/projects/api/mail-proposals?before={id}"),
        );
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn project_mail_target(&self, path: &str) -> ShellResult<ProjectMailTarget> {
        #[derive(Deserialize)]
        struct One {
            record: ProjectMailTarget,
        }
        let path = with_query("/projects/api/record", &[("path", path)]);
        let one: One = self.json(Method::GET, &path, None, 10).await?;
        Ok(one.record)
    }

    pub async fn project_mail_decision(
        &self,
        id: i64,
        body: serde_json::Value,
    ) -> ShellResult<ProjectMailDecision> {
        self.json(
            Method::POST,
            &format!("/projects/api/mail-proposals/{id}/decision"),
            Some(body),
            10,
        )
        .await
    }

    pub async fn projects(&self) -> ShellResult<Vec<ProjectSummary>> {
        #[derive(Deserialize)]
        struct List {
            projects: Vec<ProjectSummary>,
        }
        let list: List = self
            .json(Method::GET, "/projects/api/projects", None, 5)
            .await?;
        Ok(list.projects)
    }

    /// A project's records without their content: what a list of notes draws.
    pub async fn project(&self, name: &str) -> ShellResult<Vec<ProjectRecord>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<ProjectRecord>,
        }
        let path = with_query(
            "/projects/api/project",
            &[("name", name), ("content", "false")],
        );
        let records: Records = self.json(Method::GET, &path, None, 10).await?;
        Ok(records.records)
    }

    /// One record with its content: what the note editor opens.
    pub async fn record(&self, path: &str) -> ShellResult<ProjectRecord> {
        #[derive(Deserialize)]
        struct One {
            record: ProjectRecord,
        }
        let path = with_query("/projects/api/record", &[("path", path)]);
        let one: One = self.json(Method::GET, &path, None, 10).await?;
        Ok(one.record)
    }

    /// Every note across projects (`GET /projects/api/records`).
    pub async fn notes(&self) -> ShellResult<Vec<GlobalNote>> {
        #[derive(Deserialize)]
        struct Records {
            records: Vec<RecordItem>,
        }
        #[derive(Deserialize)]
        struct RecordItem {
            id: String,
            #[serde(default)]
            kind: String,
            title: String,
            #[serde(default)]
            project: String,
        }
        let notes_req: Result<Records, _> = self
            .json(
                Method::GET,
                "/projects/api/records?kind=note&content=false",
                None,
                10,
            )
            .await;
        let mut list = Vec::new();
        if let Ok(records) = notes_req {
            for r in records.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        if let Ok(projects) = self
            .json::<Records>(
                Method::GET,
                "/projects/api/records?kind=project&content=false",
                None,
                10,
            )
            .await
        {
            for r in projects.records {
                list.push(GlobalNote {
                    path: r.id,
                    kind: r.kind,
                    title: r.title,
                    project: r.project,
                });
            }
        }
        list.sort_by(|a, b| {
            a.project
                .cmp(&b.project)
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(list)
    }

    /// Replaces a record's content. `base` is the content the edit started from; the capability
    /// answers 409 and writes nothing when the record changed since.
    pub async fn save_record(&self, path: &str, content: &str, base: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path, "content": content, "base": base });
        self.send(Method::POST, "/projects/api/record", Some(body), 10)
            .await
            .map(drop)
    }

    /// Renames a record: its file takes the new title, and every record linking `[[old]]` follows.
    pub async fn rename_record(&self, path: &str, title: &str) -> ShellResult<RenamedRecord> {
        let body = serde_json::json!({ "path": path, "title": title });
        self.json(Method::POST, "/projects/api/record/rename", Some(body), 30)
            .await
    }

    /// Marks a task done, or open again, and answers with the record's new frontmatter.
    pub async fn set_task_done(&self, path: &str, done: bool) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "id": path, "done": done });
        let task: Task = self
            .json(Method::POST, "/projects/api/tasks/done", Some(body), 10)
            .await?;
        Ok(task.task)
    }

    /// Sets one frontmatter key and answers with the record's new frontmatter, so a caller can
    /// update one row instead of reading the project again.
    pub async fn set_property(
        &self,
        path: &str,
        key: &str,
        value: &str,
    ) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let body = serde_json::json!({ "path": path, "key": key, "value": value });
        let task: Task = self
            .json(
                Method::POST,
                "/projects/api/record/property",
                Some(body),
                10,
            )
            .await?;
        Ok(task.task)
    }

    pub async fn add_task(&self, project: &str, title: &str) -> ShellResult<()> {
        self.add_task_with_details(project, title, None, None).await
    }

    pub async fn add_task_with_details(
        &self,
        project: &str,
        title: &str,
        due: Option<&str>,
        priority: Option<u8>,
    ) -> ShellResult<()> {
        let mut body = serde_json::json!({ "project": project, "title": title });
        if let Some(due) = due {
            body["due"] = serde_json::Value::String(due.to_string());
        }
        if let Some(p) = priority {
            body["priority"] = serde_json::Value::Number(p.into());
        }
        self.send(Method::POST, "/projects/api/tasks", Some(body), 10)
            .await
            .map(drop)
    }

    /// Tasks across all live projects, optionally filtered by ?status=open|done.
    pub async fn tasks(&self, status: Option<&str>) -> ShellResult<Vec<GlobalTask>> {
        #[derive(Deserialize)]
        struct List {
            tasks: Vec<GlobalTask>,
        }
        let path = match status {
            Some(s) => format!("/projects/api/tasks?status={s}"),
            None => "/projects/api/tasks".to_string(),
        };
        let list: List = self.json(Method::GET, &path, None, 10).await?;
        Ok(list.tasks)
    }

    /// Creates a project hub.
    pub async fn add_project(&self, name: &str, summary: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "name": name, "summary": summary });
        self.send(Method::POST, "/projects/api/projects", Some(body), 10)
            .await
            .map(drop)
    }

    /// Creates a new note in a live project.
    pub async fn add_note(&self, project: &str, title: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "project": project, "title": title, "summary": "" });
        self.send(Method::POST, "/projects/api/notes", Some(body), 10)
            .await
            .map(drop)
    }

    /// Removes a record from the projects store and disk.
    pub async fn remove_record(&self, path: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "path": path });
        self.send(Method::POST, "/projects/api/record/remove", Some(body), 10)
            .await
            .map(drop)
    }
}
