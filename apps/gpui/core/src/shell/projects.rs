//! Projects, Records, Notes & Tasks shell client methods.

use super::*;
use projects_api::reads::{ProjectReadResponse, RecordListResponse, RecordReadResponse};
use projects_api::{
    proxy_route_mail_decision, AddNoteRequest, AddProjectRequest, AddTaskRequest,
    InventoryLinkDecisionRequest, MailDecisionRequest, RemoveRecordRequest, RenameRecordRequest,
    SaveRecordRequest, SetDoneRequest, SetPropertyRequest, PROXY_ROUTE_INVENTORY_LINKS,
    PROXY_ROUTE_INVENTORY_LINKS_REMOVE, PROXY_ROUTE_MAIL_PROPOSALS, PROXY_ROUTE_NOTES,
    PROXY_ROUTE_PROJECT, PROXY_ROUTE_PROJECTS, PROXY_ROUTE_RECORD, PROXY_ROUTE_RECORDS,
    PROXY_ROUTE_RECORD_PROPERTY, PROXY_ROUTE_RECORD_REMOVE, PROXY_ROUTE_RECORD_RENAME,
    PROXY_ROUTE_TASKS, PROXY_ROUTE_TASKS_DONE,
};

impl Shell {
    pub async fn project_inventory_links(
        &self,
        path: &str,
    ) -> ShellResult<Vec<ProjectInventoryLink>> {
        #[derive(Deserialize)]
        struct Envelope {
            links: Vec<ProjectInventoryLink>,
        }
        let path = with_query(PROXY_ROUTE_INVENTORY_LINKS, &[("path", path)]);
        let result: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(result.links)
    }

    pub async fn project_link_record(&self, path: &str) -> ShellResult<ProjectLinkRecord> {
        #[derive(Deserialize)]
        struct Envelope {
            record: ProjectLinkRecord,
        }
        let path = with_query(PROXY_ROUTE_RECORD, &[("path", path)]);
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
            PROXY_ROUTE_INVENTORY_LINKS_REMOVE
        } else {
            PROXY_ROUTE_INVENTORY_LINKS
        };
        let request = InventoryLinkDecisionRequest {
            project: project.to_owned(),
            path: record.path.clone(),
            base: record.content.clone(),
            updated_at: record.updated_at.clone(),
            item_ref: item_ref.to_owned(),
            reviewed: true,
        };
        let result: Result = self
            .json(Method::POST, path, Some(body(&request)?), 10)
            .await?;
        Ok(result.changed)
    }

    pub async fn project_mail_proposals(
        &self,
        before: Option<i64>,
    ) -> ShellResult<ProjectMailPage> {
        let path = before.map_or_else(
            || PROXY_ROUTE_MAIL_PROPOSALS.to_owned(),
            |id| with_query(PROXY_ROUTE_MAIL_PROPOSALS, &[("before", &id.to_string())]),
        );
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn project_mail_target(&self, path: &str) -> ShellResult<ProjectMailTarget> {
        #[derive(Deserialize)]
        struct One {
            record: ProjectMailTarget,
        }
        let path = with_query(PROXY_ROUTE_RECORD, &[("path", path)]);
        let one: One = self.json(Method::GET, &path, None, 10).await?;
        Ok(one.record)
    }

    pub async fn project_mail_decision(
        &self,
        id: i64,
        request: &MailDecisionRequest,
    ) -> ShellResult<ProjectMailDecision> {
        self.json(
            Method::POST,
            &proxy_route_mail_decision(id),
            Some(body(request)?),
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
            .json(Method::GET, PROXY_ROUTE_PROJECTS, None, 5)
            .await?;
        Ok(list.projects)
    }

    /// A project's records without their content: what a list of notes draws.
    pub async fn project(&self, name: &str) -> ShellResult<Vec<ProjectRecord>> {
        let path = with_query(PROXY_ROUTE_PROJECT, &[("name", name), ("content", "false")]);
        let read: ProjectReadResponse = self.json(Method::GET, &path, None, 10).await?;
        Ok(read.records.into_iter().map(ProjectRecord::from).collect())
    }

    /// One record with its content: what the note editor opens.
    pub async fn record(&self, path: &str) -> ShellResult<ProjectRecord> {
        let path = with_query(PROXY_ROUTE_RECORD, &[("path", path)]);
        let read: RecordReadResponse = self.json(Method::GET, &path, None, 10).await?;
        Ok(read.record.into())
    }

    /// Every note and project record across projects (`GET /projects/api/records`). A kind
    /// whose read fails is left out, so one failing list does not empty the other.
    pub async fn notes(&self) -> ShellResult<Vec<GlobalNote>> {
        let mut list = Vec::new();
        for kind in ["note", "project"] {
            let path = with_query(PROXY_ROUTE_RECORDS, &[("kind", kind), ("content", "false")]);
            if let Ok(read) = self
                .json::<RecordListResponse>(Method::GET, &path, None, 10)
                .await
            {
                list.extend(read.records.into_iter().map(GlobalNote::from));
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
        let request = SaveRecordRequest {
            path: path.to_owned(),
            content: content.to_owned(),
            base: Some(base.to_owned()),
        };
        self.send(Method::POST, PROXY_ROUTE_RECORD, Some(body(&request)?), 10)
            .await
            .map(drop)
    }

    /// Renames a record: its file takes the new title, and every record linking `[[old]]` follows.
    pub async fn rename_record(&self, path: &str, title: &str) -> ShellResult<RenamedRecord> {
        let request = RenameRecordRequest {
            path: path.to_owned(),
            title: title.to_owned(),
        };
        self.json(
            Method::POST,
            PROXY_ROUTE_RECORD_RENAME,
            Some(body(&request)?),
            30,
        )
        .await
    }

    /// Marks a task done, or open again, and answers with the record's new frontmatter.
    pub async fn set_task_done(&self, path: &str, done: bool) -> ShellResult<GlobalTask> {
        #[derive(Deserialize)]
        struct Task {
            task: GlobalTask,
        }
        let request = SetDoneRequest {
            id: path.to_owned(),
            done,
        };
        let task: Task = self
            .json(
                Method::POST,
                PROXY_ROUTE_TASKS_DONE,
                Some(body(&request)?),
                10,
            )
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
        let request = SetPropertyRequest {
            path: path.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let task: Task = self
            .json(
                Method::POST,
                PROXY_ROUTE_RECORD_PROPERTY,
                Some(body(&request)?),
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
        let request = AddTaskRequest {
            project: project.to_owned(),
            title: title.to_owned(),
            summary: String::new(),
            due: due.map(str::to_owned),
            priority,
        };
        self.send(Method::POST, PROXY_ROUTE_TASKS, Some(body(&request)?), 10)
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
            Some(s) => with_query(PROXY_ROUTE_TASKS, &[("status", s)]),
            None => PROXY_ROUTE_TASKS.to_owned(),
        };
        let list: List = self.json(Method::GET, &path, None, 10).await?;
        Ok(list.tasks)
    }

    /// Creates a project hub.
    pub async fn add_project(&self, name: &str, summary: &str) -> ShellResult<()> {
        let request = AddProjectRequest {
            name: name.to_owned(),
            summary: summary.to_owned(),
        };
        self.send(
            Method::POST,
            PROXY_ROUTE_PROJECTS,
            Some(body(&request)?),
            10,
        )
        .await
        .map(drop)
    }

    /// Creates a new note in a live project.
    pub async fn add_note(&self, project: &str, title: &str) -> ShellResult<()> {
        let request = AddNoteRequest {
            project: project.to_owned(),
            title: title.to_owned(),
            summary: String::new(),
        };
        self.send(Method::POST, PROXY_ROUTE_NOTES, Some(body(&request)?), 10)
            .await
            .map(drop)
    }

    /// Removes a record from the projects store and disk.
    pub async fn remove_record(&self, path: &str) -> ShellResult<()> {
        let request = RemoveRecordRequest {
            path: path.to_owned(),
        };
        self.send(
            Method::POST,
            PROXY_ROUTE_RECORD_REMOVE,
            Some(body(&request)?),
            10,
        )
        .await
        .map(drop)
    }
}
