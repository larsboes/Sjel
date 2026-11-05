//! Systems, Backup, Setup, Import, Software, WebLinks, Distribution, Credentials, Ask, Music & Soundscape shell client methods.

use super::*;

impl Shell {
    pub async fn agent(&self) -> ShellResult<AgentView> {
        self.json(Method::GET, "/api/sjel-status/agent", None, 5)
            .await
    }

    /// Sets one capability's agent mode (`off` · `read-only` · `ask` · `auto`).
    ///
    /// Every gate reads the same policy on its next request, so nothing restarts. The shell
    /// refuses a mode it does not know and a capability with no gate, both as a 400 that names
    /// the reason; that text arrives as `ShellError::Refused`.
    pub async fn set_agent_mode(&self, capability: &str, mode: &str) -> ShellResult<()> {
        if !is_registry_name(capability) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let body = serde_json::json!({ "capability": capability, "mode": mode });
        self.send(Method::POST, "/api/sjel-status/agent/mode", Some(body), 5)
            .await
            .map(drop)
    }

    /// The live hardware sample. macmon is a capability, so the shell proxies `/macmon` to it;
    /// a stopped macmon is a refused request, which the page reports as "not running" rather
    /// than as a fault.
    pub async fn macmon(&self) -> ShellResult<crate::systems::MacmonSample> {
        // macmon samples the SMC every 5 s, so a shorter budget than that would report a slow
        // read as a failure on a loaded machine.
        self.json(Method::GET, "/macmon/json", None, 10).await
    }

    /// What fills the disk, from `tools/storage report --json`. A `du` walk per class, so it
    /// takes seconds and the page reads it on demand rather than on a timer.
    pub async fn storage(&self) -> ShellResult<crate::systems::StorageReport> {
        self.json(Method::GET, "/api/sjel-status/storage", None, 120)
            .await
    }

    /// The disk's own hourly readings, kept by sjel-status.
    pub async fn storage_history(&self) -> ShellResult<crate::systems::StorageHistory> {
        self.json(Method::GET, "/api/sjel-status/storage-history", None, 10)
            .await
    }

    /// Installed software outside this checkout, from `tools/updates --json`. The shell answers
    /// a cached report for up to 15 minutes, so a read is cheap.
    pub async fn updates(&self) -> ShellResult<crate::systems::UpdatesReport> {
        self.json(Method::GET, "/api/sjel-status/updates", None, 30)
            .await
    }

    /// Starts one class moving. The shell answers `202` before the work starts — a cargo step
    /// compiles for minutes — and the outcome arrives on the next report as `lastApply`.
    pub async fn apply_updates(&self, class: &str) -> ShellResult<()> {
        if !is_registry_name(class) {
            return Err(ShellError::Refused(
                400,
                Some("Not an update class.".into()),
            ));
        }
        let body = serde_json::json!({ "class": class });
        self.send(
            Method::POST,
            "/api/sjel-status/updates/apply",
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// The hourly host watch's open findings.
    pub async fn host_watch(&self) -> ShellResult<Vec<crate::systems::Finding>> {
        let findings: crate::systems::Findings = self
            .json(Method::GET, "/api/sjel-status/host-watch", None, 10)
            .await?;
        Ok(findings.findings)
    }

    /// The runtime profile in force on this machine.
    pub async fn runtime(&self) -> ShellResult<crate::systems::RuntimeStatus> {
        self.json(Method::GET, "/api/sjel-status/runtime", None, 10)
            .await
    }

    /// Saves a selection or an exception set. `expected_revision` makes a change from a stale
    /// read a `409` instead of silently overwriting whatever was saved in between.
    pub async fn set_runtime(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<crate::systems::RuntimeStatus> {
        self.json(Method::POST, "/api/sjel-status/runtime", Some(body), 10)
            .await
    }

    /// Agent skills across installed harnesses from `tools/harnesses status --json`.
    pub async fn packs(&self) -> ShellResult<crate::packs::PacksView> {
        self.json(Method::GET, "/api/sjel-status/packs", None, 30)
            .await
    }

    /// Deploy, sync or remove Packs on one harness.
    pub async fn write_pack(
        &self,
        action: &str,
        harness: &str,
        packs: &[String],
    ) -> ShellResult<crate::packs::PackWriteResult> {
        let body = serde_json::json!({
            "action": action,
            "harness": harness,
            "packs": packs,
        });
        self.json(Method::POST, "/api/sjel-status/packs", Some(body), 60)
            .await
    }

    /// Architectural self-model and live state from `GET /api/sjel-status/self`.
    pub async fn self_model(&self) -> ShellResult<crate::self_model::SelfModelResponse> {
        self.json(Method::GET, "/api/sjel-status/self", None, 15)
            .await
    }

    /// CV status, applications, profiles and built PDFs from `GET /api/sjel-status/cv`.
    pub async fn cv(&self) -> ShellResult<crate::cv::CvView> {
        self.json(Method::GET, "/api/sjel-status/cv", None, 20)
            .await
    }

    /// Run one CV verb (build, app-new, app-status, linkedin-diff, etc.) via `POST /api/sjel-status/cv`.
    pub async fn write_cv(
        &self,
        req: &crate::cv::CvWriteRequest,
    ) -> ShellResult<crate::cv::CvWriteResult> {
        let body = serde_json::to_value(req).map_err(|_| ShellError::Unreadable)?;
        self.json(Method::POST, "/api/sjel-status/cv", Some(body), 60)
            .await
    }

    /// Read index status totals and indexed volumes via `GET /media/api/status`.
    pub async fn media_status(&self) -> ShellResult<crate::media::MediaStatus> {
        self.json(Method::GET, "/media/api/status", None, 15).await
    }

    /// Read the recent ingest runs via `GET /media/api/ingests`.
    pub async fn media_ingests(&self) -> ShellResult<crate::media::MediaIngestRunsResponse> {
        self.json(Method::GET, "/media/api/ingests", None, 15).await
    }

    /// Read full details of one ingest run via `GET /media/api/ingests/{id}`.
    pub async fn media_ingest(&self, id: i64) -> ShellResult<crate::media::MediaIngestDetail> {
        let path = format!("/media/api/ingests/{id}");
        self.json(Method::GET, &path, None, 20).await
    }

    /// Read combined media status and recent runs for the primary media view.
    pub async fn media_view(&self) -> ShellResult<crate::media::MediaView> {
        let (status, ingests) = futures::try_join!(self.media_status(), self.media_ingests())?;
        Ok(crate::media::MediaView {
            status,
            runs: ingests.runs,
        })
    }

    /// Read the research view containing markdown articles, systems, upstreams, and benchmarks.
    pub async fn research_view(&self) -> ShellResult<crate::research::ResearchView> {
        Ok(crate::research::default_research_view())
    }

    /// Read what this machine still needs via `GET /api/sjel-status/setup`.
    pub async fn setup_plans(&self) -> ShellResult<crate::setup::SetupPlans> {
        self.json(Method::GET, "/api/sjel-status/setup", None, 30)
            .await
    }

    /// Check connection to primary via `GET /api/sjel-status/setup/verify`.
    pub async fn setup_verify(&self) -> ShellResult<crate::setup::SetupVerification> {
        self.json(Method::GET, "/api/sjel-status/setup/verify", None, 30)
            .await
    }

    /// Apply setup steps via `POST /api/sjel-status/setup/apply`.
    pub async fn setup_apply(
        &self,
        request: &crate::setup::SetupApplyRequest,
    ) -> ShellResult<crate::setup::SetupApplied> {
        let body = serde_json::to_value(request).unwrap_or(serde_json::Value::Null);
        self.json(Method::POST, "/api/sjel-status/setup/apply", Some(body), 60)
            .await
    }

    /// List paired devices via `GET /devices/api/devices`.
    pub async fn setup_devices(&self) -> ShellResult<Vec<crate::setup::PairedDevice>> {
        match self
            .json::<DevicesResp>(Method::GET, "/devices/api/devices", None, 10)
            .await
        {
            Ok(resp) => Ok(resp.devices),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Revoke one device's key via `POST /devices/api/devices/{id}/revoke`. The answer is the
    /// device as the node now holds it, which is `revoked`.
    pub async fn setup_revoke_device(&self, id: &str) -> ShellResult<crate::setup::PairedDevice> {
        let path = format!("/devices/api/devices/{id}/revoke");
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// This node's Same-Wi-Fi listener via `GET /api/sjel-status/lan`, or `None` when it has none.
    pub async fn setup_lan(&self) -> ShellResult<crate::setup::LanStatus> {
        self.json(Method::GET, "/api/sjel-status/lan", None, 10)
            .await
    }

    /// Mint a one-time code for one new device via `POST /devices/api/pairing/challenges`.
    pub async fn setup_create_challenge(&self) -> ShellResult<crate::setup::PairingChallenge> {
        self.json(
            Method::POST,
            "/devices/api/pairing/challenges",
            Some(serde_json::json!({})),
            15,
        )
        .await
    }

    /// List pending join requests via `GET /devices/api/pairing/requests`.
    pub async fn setup_pending_joins(&self) -> ShellResult<Vec<crate::setup::SetupJoinRequest>> {
        match self
            .json::<JoinsResp>(Method::GET, "/devices/api/pairing/requests", None, 10)
            .await
        {
            Ok(resp) => Ok(resp.requests),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Approve a device join request via `POST /devices/api/pairing/requests/{id}/approve`.
    pub async fn setup_approve_join(
        &self,
        id: &str,
    ) -> ShellResult<crate::setup::SetupJoinRequest> {
        let path = format!("/devices/api/pairing/requests/{}/approve", id);
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// Deny a device join request via `POST /devices/api/pairing/requests/{id}/deny`.
    pub async fn setup_deny_join(&self, id: &str) -> ShellResult<crate::setup::SetupJoinRequest> {
        let path = format!("/devices/api/pairing/requests/{}/deny", id);
        self.json(Method::POST, &path, Some(serde_json::json!({})), 15)
            .await
    }

    /// Load the combined view model for `/setup`: what this machine still needs, which devices it
    /// trusts, which joins wait, and whether a phone can reach it at all.
    pub async fn setup_view(&self) -> ShellResult<crate::setup::SetupView> {
        let plans = match self.setup_plans().await {
            Ok(p) => p.components,
            Err(_) => Vec::new(),
        };
        let devices = self.setup_devices().await.unwrap_or_default();
        let pending_joins = self.setup_pending_joins().await.unwrap_or_default();
        // A read that fails is not the same answer as a listener that is off, so the page is told
        // which one happened (its Devices tab says so in words).
        let (lan, lan_error) = match self.setup_lan().await {
            Ok(status) => (status.listener().cloned(), None),
            Err(error) => (None, Some(error.to_string())),
        };

        Ok(crate::setup::SetupView {
            plans,
            devices,
            pending_joins,
            lan,
            lan_error,
        })
    }

    /// Add a watch entry to `upstreams.toml` via `POST /api/sjel-status/upstreams/watch`.
    pub async fn watch_upstream(
        &self,
        url: &str,
        summary: &str,
        name: Option<&str>,
    ) -> ShellResult<crate::research::UpstreamRow> {
        let mut body = serde_json::json!({
            "url": url,
            "summary": summary,
        });
        if let Some(n) = name {
            body["name"] = serde_json::Value::String(n.to_string());
        }
        self.json(
            Method::POST,
            "/api/sjel-status/upstreams/watch",
            Some(body),
            30,
        )
        .await
    }

    /// The finder's server results (`GET /api/sjel-status/search?q=`): rows from the shared
    /// database that the app does not hold, each with a `sjel://` link to open it by.
    pub async fn search(&self, query: &str) -> ShellResult<Vec<ServerHit>> {
        #[derive(Deserialize)]
        struct Envelope {
            hits: Vec<ServerHit>,
        }
        let path = with_query("/api/sjel-status/search", &[("q", query)]);
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.hits)
    }

    /// Allows or denies one waiting agent write.
    pub async fn decide(&self, id: &str, allow: bool) -> ShellResult<()> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not an approval id.".into())));
        }
        let path = format!("/api/sjel-status/agent/approvals/{id}");
        let body = serde_json::json!({ "allow": allow });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Starts or stops one capability. A start waits for the service to come up.
    pub async fn set_running(&self, name: &str, running: bool) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let action = if running { "start" } else { "stop" };
        let path = format!("/api/sjel-status/capabilities/{name}/{action}");
        self.send(Method::POST, &path, None, 60).await.map(drop)
    }

    pub async fn backups(&self) -> ShellResult<Backups> {
        self.json(Method::GET, "/api/sjel-status/backups", None, 10)
            .await
    }

    pub async fn backup_targets(&self) -> ShellResult<Vec<crate::backup::BackupTargetRecord>> {
        let res: crate::backup::BackupTargetsResponse = self
            .json(Method::GET, "/api/sjel-status/backup/targets", None, 10)
            .await?;
        Ok(res.targets)
    }

    pub async fn backup_runs(
        &self,
        limit: usize,
    ) -> ShellResult<(Vec<crate::backup::BackupRunAttemptRecord>, usize)> {
        let path = format!("/api/sjel-status/backup/runs?limit={limit}");
        let res: crate::backup::BackupRunsResponse =
            self.json(Method::GET, &path, None, 10).await?;
        Ok((res.runs, res.running))
    }

    pub async fn verify_backup_target(
        &self,
        target: &str,
    ) -> ShellResult<crate::backup::VerifyTargetResult> {
        let body = serde_json::json!({ "target": target });
        self.json(
            Method::POST,
            "/api/sjel-status/backup/verify",
            Some(body),
            60,
        )
        .await
    }

    pub async fn set_backup_policy(
        &self,
        target: &str,
        interval_hours: Option<i64>,
    ) -> ShellResult<()> {
        let body = serde_json::json!({
            "target": target,
            "interval_hours": interval_hours,
        });
        self.send(
            Method::POST,
            "/api/sjel-status/backup/policy",
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// Starts one capability's backup to the target its manifest declares or an optional specific target.
    pub async fn backup(&self, name: &str) -> ShellResult<()> {
        self.backup_capability(name, None).await
    }

    pub async fn backup_capability(&self, name: &str, target: Option<&str>) -> ShellResult<()> {
        if !is_registry_name(name) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability name.".into()),
            ));
        }
        let path = format!("/api/sjel-status/capabilities/{name}/backup");
        let body = target.map(|t| serde_json::json!({ "target": t }));
        self.send(Method::POST, &path, body, 10).await.map(drop)
    }

    /// Sorts every file under `folder` without importing. Walks the whole tree, so a large
    /// archive takes seconds.
    pub async fn import_plan(&self, folder: &str) -> ShellResult<ImportPlan> {
        let path = with_query("/import/api/import/plan", &[("folder", folder)]);
        self.json(Method::GET, &path, None, 120).await
    }

    pub async fn import_status(&self, folder: &str) -> ShellResult<ImportStatus> {
        let path = with_query("/import/api/import/status", &[("folder", folder)]);
        self.json(Method::GET, &path, None, 10).await
    }

    /// Starts an import in the background. 409 while another runs.
    pub async fn import_start(&self, folder: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "folder": folder });
        self.send(Method::POST, "/import/api/import/runs", Some(body), 10)
            .await
            .map(drop)
    }

    pub async fn import_current(&self) -> ShellResult<Option<ImportJob>> {
        let envelope: ImportJobEnvelope = self
            .json(Method::GET, "/import/api/import/runs/current", None, 10)
            .await?;
        Ok(envelope.job)
    }

    pub async fn software_apps(&self) -> ShellResult<SoftwareApps> {
        self.json(Method::GET, "/software/api/apps", None, 10).await
    }

    pub async fn web_links(&self) -> ShellResult<crate::web_links::WebLinks> {
        self.json(
            Method::GET,
            crate::web_links::PROXY_ROUTE_WEB_LINKS,
            None,
            10,
        )
        .await
    }

    pub async fn web_link_add(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<crate::web_links::WebLink> {
        self.json(
            Method::POST,
            crate::web_links::PROXY_ROUTE_WEB_LINKS,
            Some(body),
            10,
        )
        .await
    }

    pub async fn web_link_edit(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<crate::web_links::WebLink> {
        let id = web_link_key(id)?;
        self.json(
            Method::PATCH,
            &format!("{}/{id}", crate::web_links::PROXY_ROUTE_WEB_LINKS),
            Some(body),
            10,
        )
        .await
    }

    pub async fn web_link_remove(&self, id: &str) -> ShellResult<()> {
        let id = web_link_key(id)?;
        self.send(
            Method::DELETE,
            &format!("{}/{id}", crate::web_links::PROXY_ROUTE_WEB_LINKS),
            None,
            10,
        )
        .await
        .map(drop)
    }

    pub async fn web_link_open(&self, id: &str) -> ShellResult<crate::web_links::WebLink> {
        let id = web_link_key(id)?;
        self.json(
            Method::POST,
            &format!("{}/{id}/open", crate::web_links::PROXY_ROUTE_WEB_LINKS),
            Some(serde_json::json!({})),
            10,
        )
        .await
    }

    pub async fn web_link_history_scan(
        &self,
        days: u16,
        min_visits: u16,
    ) -> ShellResult<crate::web_links::WebLinkHistoryReply> {
        self.json(
            Method::POST,
            crate::web_links::PROXY_ROUTE_HISTORY_SCAN,
            Some(serde_json::json!({"days":days, "min_visits":min_visits})),
            120,
        )
        .await
    }

    pub async fn web_link_history_candidates(
        &self,
    ) -> ShellResult<Vec<crate::web_links::WebLinkCandidate>> {
        self.json(
            Method::GET,
            crate::web_links::PROXY_ROUTE_HISTORY_CANDIDATES,
            None,
            10,
        )
        .await
    }

    pub async fn web_link_history_decide(&self, id: &str, decision: &str) -> ShellResult<()> {
        let id = web_link_key(id)?;
        if !matches!(decision, "keep" | "dismiss") {
            return Err(ShellError::Refused(
                400,
                Some("Not a history decision.".into()),
            ));
        }
        self.send(
            Method::POST,
            &format!(
                "{}/{id}/decision",
                crate::web_links::PROXY_ROUTE_HISTORY_CANDIDATES
            ),
            Some(serde_json::json!({"decision":decision})),
            10,
        )
        .await
        .map(drop)
    }

    /// Scans now. Waits for the scan, which takes tens of seconds on a full /Applications.
    pub async fn software_scan(&self) -> ShellResult<()> {
        let body = serde_json::json!({});
        self.send(Method::POST, "/software/api/apps/scan", Some(body), 120)
            .await
            .map(drop)
    }

    /// Appends a decision about one app. `decision` is `use`, `remove`, `replace` or `rebuild`.
    /// Uninstalls a row whose decision is `remove`. Waits for it: a cask or a large app takes a
    /// while, and the service rescans afterwards.
    pub async fn software_uninstall(&self, bundle_id: &str) -> ShellResult<()> {
        let path = format!("/software/api/apps/{}/uninstall", software_key(bundle_id)?);
        let body = serde_json::json!({});
        self.send(Method::POST, &path, Some(body), 300)
            .await
            .map(drop)
    }

    pub async fn software_decide(
        &self,
        bundle_id: &str,
        decision: &str,
        note: &str,
    ) -> ShellResult<()> {
        let path = format!("/software/api/apps/{}/decision", software_key(bundle_id)?);
        let body = serde_json::json!({ "decision": decision, "note": note });
        self.send(Method::POST, &path, Some(body), 10)
            .await
            .map(drop)
    }

    /// Follows the shell's change stream. The receiver ends when the shell has no stream to
    /// send, so a caller that gets no frame keeps reading on its own timer; after a restart the
    /// connection is reopened silently.
    ///
    /// # Errors
    /// `NotSetUp`, `NotLoopback` or `Unreadable`, before any connection is opened.
    pub fn subscribe(&self) -> ShellResult<UnboundedReceiver<Snapshot>> {
        let token = self.token()?;
        let url = self
            .base
            .join(EVENTS_PATH)
            .map_err(|_| ShellError::Unreadable)?;
        let (to, from) = futures::channel::mpsc::unbounded();
        let client = self.client.clone();
        let token_cache = Arc::clone(&self.token);
        spawn(async move {
            let mut token = token;
            while !to.is_closed() {
                match follow(&client, &url, &token, &to).await {
                    Followed::Unsupported => return,
                    Followed::Lost => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Followed::Unauthorized => {
                        *token_cache.lock().expect("token lock") = None;
                        let _ = to.unbounded_send(Snapshot {
                            error: Some(
                                "The local shell rejected its saved token. Refreshing it from Keychain."
                                    .into(),
                            ),
                            ..Snapshot::default()
                        });
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        token = match cached_token(&token_cache) {
                            Ok(token) => token,
                            Err(error) => {
                                let _ = to.unbounded_send(Snapshot {
                                    error: Some(error.to_string()),
                                    ..Snapshot::default()
                                });
                                return;
                            }
                        };
                    }
                }
            }
        });
        Ok(from)
    }

    /// Runs one turn in a native Ask session through `POST /assistant/api/ask`, starting the
    /// on-demand `assistant` capability if it is stopped, and falling back to a direct capability
    /// summary when the local model role is unavailable.
    pub async fn ask(&self, prompt: &str, history: &[AskTurn]) -> ShellResult<AskReply> {
        let body = serde_json::json!({ "prompt": prompt, "history": history });
        match self
            .json::<AskReply>(
                Method::POST,
                PROXY_ROUTE_ASSISTANT_ASK,
                Some(body.clone()),
                60,
            )
            .await
        {
            Ok(reply) => return Ok(reply),
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                if let Ok(reply) = self
                    .json::<AskReply>(Method::POST, PROXY_ROUTE_ASSISTANT_ASK, Some(body), 60)
                    .await
                {
                    return Ok(reply);
                }
            }
            Err(ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback) => {
                return self
                    .json::<AskReply>(Method::POST, PROXY_ROUTE_ASSISTANT_ASK, Some(body), 5)
                    .await;
            }
            Err(_) => {}
        }
        self.capability_fallback(prompt).await
    }

    /// Lists configured provider roles without retrieving credential values or endpoint URLs.
    pub async fn ask_providers(&self) -> ShellResult<Vec<AskProvider>> {
        let result = self
            .json::<Vec<AskProvider>>(Method::GET, PROXY_ROUTE_ASSISTANT_PROVIDERS, None, 10)
            .await;
        match result {
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                self.json::<Vec<AskProvider>>(
                    Method::GET,
                    PROXY_ROUTE_ASSISTANT_PROVIDERS,
                    None,
                    10,
                )
                .await
            }
            other => other,
        }
    }

    /// Reports whether provider API keys are present without returning key values.
    pub async fn provider_key_status(&self) -> ShellResult<Vec<ProviderKeyStatus>> {
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider, Presence,
            };
            use std::sync::Arc;

            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            ["openai", "deepseek", "opencode-go", "nvidia-nim", "gemini"]
                .into_iter()
                .map(|slug| {
                    let id = CredentialId::parse(format!("inference-{slug}"))
                        .map_err(|_| ShellError::CredentialUnavailable)?;
                    let present = match manager.status(&id) {
                        Ok(Presence::Present) => true,
                        Ok(Presence::Missing)
                        | Err(sjel_credentials::CredentialError::NotFound) => false,
                        Err(_) => return Err(ShellError::CredentialUnavailable),
                    };
                    Ok(ProviderKeyStatus {
                        provider: slug.to_owned(),
                        present,
                    })
                })
                .collect()
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Lists receipt-backed packages from the writable runtime selected by the desktop launcher.
    pub async fn installed_features(&self) -> ShellResult<Vec<InstalledFeature>> {
        let executable = std::env::var_os("SJEL_CLI_BIN").ok_or_else(|| {
            ShellError::PackageManager("this app was not started from a placed runtime".into())
        })?;
        tokio::task::spawn_blocking(move || {
            let output = Command::new(executable)
                .args(["distribution", "list", "--json"])
                .output()
                .map_err(|error| ShellError::PackageManager(error.to_string()))?;
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(ShellError::PackageManager(if detail.is_empty() {
                    "the runtime could not read its package inventory".into()
                } else {
                    detail
                }));
            }
            serde_json::from_slice(&output.stdout).map_err(|_| ShellError::Unreadable)
        })
        .await
        .map_err(|error| ShellError::PackageManager(error.to_string()))?
    }

    /// Uninstalls a receipt-backed package, retaining its capability data.
    pub async fn uninstall_feature(&self, name: &str) -> ShellResult<()> {
        if name.is_empty()
            || name.len() > 80
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || name.starts_with('-')
        {
            return Err(ShellError::PackageManager("invalid feature name".into()));
        }
        let executable = std::env::var_os("SJEL_CLI_BIN").ok_or_else(|| {
            ShellError::PackageManager("this app was not started from a placed runtime".into())
        })?;
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let output = Command::new(executable)
                .args([
                    "distribution",
                    "uninstall-package",
                    &name,
                    "--preserve-data",
                ])
                .output()
                .map_err(|error| ShellError::PackageManager(error.to_string()))?;
            if output.status.success() {
                Ok(())
            } else {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                Err(ShellError::PackageManager(if detail.is_empty() {
                    "the runtime refused to uninstall this feature".into()
                } else {
                    detail
                }))
            }
        })
        .await
        .map_err(|error| ShellError::PackageManager(error.to_string()))?
    }

    /// `GET /api/sjel-status/credentials`: every declared credential with its presence. No value.
    pub async fn credentials(&self) -> ShellResult<Vec<crate::accounts::CredentialRow>> {
        self.json(Method::GET, "/api/sjel-status/credentials", None, 10)
            .await
    }

    /// Store a provider key without sending it to the Sjel HTTP service. The input is held in a
    /// zeroizing buffer, and the private catalog receives only the stable credential ID.
    pub async fn store_provider_key(
        &self,
        provider: &str,
        key: zeroize::Zeroizing<String>,
    ) -> ShellResult<()> {
        let (slug, label) = match provider {
            "openai" => ("openai", "OpenAI API"),
            "deepseek" => ("deepseek", "DeepSeek API"),
            "opencode-go" => ("opencode-go", "OpenCode Go API"),
            "nvidia-nim" => ("nvidia-nim", "NVIDIA NIM API"),
            "gemini" => ("gemini", "Google Gemini API (AI Studio)"),
            _ => return Err(ShellError::CredentialUnavailable),
        };
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider,
                ProviderKind, SecretValue,
            };
            use std::sync::Arc;

            let id = CredentialId::parse(format!("inference-{slug}"))
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let secret = SecretValue::new(key.as_bytes().to_vec())
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let exists = manager
                .list()
                .map_err(|_| ShellError::CredentialUnavailable)?
                .iter()
                .any(|entry| entry.id == id);
            if exists {
                manager.update(&id, &secret)
            } else {
                manager
                    .create(
                        id,
                        label.to_owned(),
                        ProviderKind::Keychain,
                        vec!["sjel-inference".to_owned()],
                        &secret,
                    )
                    .map(|_| ())
            }
            .map_err(|_| ShellError::CredentialUnavailable)
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Remove a saved provider key by its public provider slug.
    pub async fn remove_provider_key(&self, provider: &str) -> ShellResult<()> {
        let slug = match provider {
            "openai" | "deepseek" | "opencode-go" | "nvidia-nim" | "gemini" => provider,
            _ => return Err(ShellError::CredentialUnavailable),
        }
        .to_owned();
        let catalog = sjel_config::overlay_config("credentials.toml")
            .ok_or(ShellError::CredentialUnavailable)?;
        tokio::task::spawn_blocking(move || {
            use sjel_credentials::{
                CredentialId, CredentialManager, CredentialProvider, KeychainProvider,
            };
            use std::sync::Arc;

            let id = CredentialId::parse(format!("inference-{slug}"))
                .map_err(|_| ShellError::CredentialUnavailable)?;
            let providers: Vec<Arc<dyn CredentialProvider>> =
                vec![Arc::new(KeychainProvider::system())];
            let manager = CredentialManager::new(catalog, providers)
                .map_err(|_| ShellError::CredentialUnavailable)?;
            manager
                .delete(&id)
                .map_err(|_| ShellError::CredentialUnavailable)
        })
        .await
        .map_err(|_| ShellError::CredentialUnavailable)?
    }

    /// Starts an event-streamed Ask run and returns its steering ID with a receiver for events.
    pub async fn ask_stream(
        &self,
        prompt: &str,
        history: &[AskTurn],
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        self.ask_stream_with_provider(prompt, history, None, false)
            .await
    }

    /// Stream Ask with an explicit cloud role and per-prompt public-data acknowledgement.
    pub async fn ask_stream_with_provider(
        &self,
        prompt: &str,
        history: &[AskTurn],
        provider_role: Option<&str>,
        confirm_public_send: bool,
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        let mut body = serde_json::json!({
            "prompt": prompt,
            "history": history,
            "confirm_public_send": confirm_public_send,
        });
        if let Some(role) = provider_role {
            body["provider_role"] = serde_json::Value::String(role.to_owned());
        }
        if provider_role.is_some() && !history.is_empty() {
            return Err(ShellError::Refused(
                409,
                Some("cloud Ask does not send local conversation history".into()),
            ));
        }
        let response = match self
            .send(
                Method::POST,
                PROXY_ROUTE_ASSISTANT_ASK_STREAM,
                Some(body.clone()),
                600,
            )
            .await
        {
            Ok(response) => response,
            Err(ShellError::Refused(502, _)) => {
                let _ = self.set_running("assistant", true).await;
                match self
                    .send(
                        Method::POST,
                        PROXY_ROUTE_ASSISTANT_ASK_STREAM,
                        Some(body),
                        600,
                    )
                    .await
                {
                    Ok(response) => response,
                    Err(error) if provider_role.is_some() => return Err(error),
                    Err(_) => return self.fallback_ask_stream(prompt).await,
                }
            }
            Err(ShellError::NotSetUp | ShellError::Unreachable | ShellError::NotLoopback) => {
                if provider_role.is_some() {
                    return Err(ShellError::Unreachable);
                }
                return self.fallback_ask_stream(prompt).await;
            }
            Err(ShellError::Refused(404 | 405, _)) => {
                if provider_role.is_some() {
                    return Err(ShellError::Refused(
                        503,
                        Some("the assistant API does not support cloud Ask".into()),
                    ));
                }
                let reply = self.ask(prompt, history).await?;
                return Ok(Self::completed_ask_stream(reply));
            }
            Err(ShellError::Refused(503, _)) if provider_role.is_none() => {
                return self.fallback_ask_stream(prompt).await;
            }
            Err(error) => return Err(error),
        };
        let run_id = response
            .headers()
            .get("x-sjel-ask-run")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(ShellError::Unreadable)?;
        let (to, from) = futures::channel::mpsc::unbounded();
        spawn(async move {
            let mut response = response;
            let mut buffer = Vec::new();
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) | Err(_) => break,
                };
                buffer.extend_from_slice(&chunk);
                while let Some(data) = next_frame(&mut buffer) {
                    if let Ok(event) = serde_json::from_str::<AskEvent>(&data) {
                        let terminal = matches!(event.kind.as_str(), "done" | "error" | "stopped");
                        if to.unbounded_send(event).is_err() || terminal {
                            return;
                        }
                    }
                }
            }
        });
        Ok((run_id, from))
    }

    async fn fallback_ask_stream(
        &self,
        prompt: &str,
    ) -> ShellResult<(u64, UnboundedReceiver<AskEvent>)> {
        let reply = self.capability_fallback(prompt).await?;
        Ok(Self::completed_ask_stream(reply))
    }

    fn completed_ask_stream(reply: AskReply) -> (u64, UnboundedReceiver<AskEvent>) {
        let (to, from) = futures::channel::mpsc::unbounded();
        let _ = to.unbounded_send(AskEvent {
            kind: "done".into(),
            content: String::new(),
            name: String::new(),
            arguments: String::new(),
            text: reply.text,
            model: reply.model,
            source: reply.source,
            tools: reply.tools,
            message: String::new(),
        });
        (0, from)
    }

    /// Queues an instruction for the next model request in an active Ask run.
    pub async fn steer_ask(&self, run_id: u64, message: &str) -> ShellResult<()> {
        let path = format!("/assistant/api/ask/{run_id}/steer");
        let body = serde_json::json!({ "message": message });
        self.send(Method::POST, &path, Some(body), 5)
            .await
            .map(drop)
    }

    /// Requests cooperative cancellation of an active Ask run.
    pub async fn stop_ask(&self, run_id: u64) -> ShellResult<()> {
        let path = format!("/assistant/api/ask/{run_id}/stop");
        self.send(Method::POST, &path, Some(serde_json::json!({})), 5)
            .await
            .map(drop)
    }

    async fn capability_fallback(&self, prompt: &str) -> ShellResult<AskReply> {
        if !is_capability_status_question(prompt) {
            return Ok(AskReply {
                text: "The local assistant is unavailable, so I can’t answer that prompt yet. I can still show a live capability summary when you ask which capabilities are up.".into(),
                model: "assistant unavailable".into(),
                source: "fallback".into(),
                tools: Vec::new(),
            });
        }
        let health = self.health().await?;
        let up = health.capabilities.values().filter(|c| c.up).count();
        let down = health.down();
        let projects = self.projects().await.unwrap_or_default();
        let mut lines = vec![format!(
            "Sjel {} (up {}m): {} of {} capabilities running{}.",
            health.version,
            health.uptime_seconds / 60,
            up,
            health.capabilities.len(),
            if down.is_empty() {
                String::new()
            } else {
                format!("; down: {}", down.join(", "))
            }
        )];
        if !projects.is_empty() {
            let names: Vec<String> = projects
                .iter()
                .filter(|p| !p.archived)
                .map(|p| format!("{} ({})", p.name, p.notes))
                .collect();
            lines.push(format!("Active projects: {}.", names.join(", ")));
        }
        Ok(AskReply {
            text: lines.join("\n"),
            model: "capabilities".into(),
            source: "deterministic".into(),
            tools: vec!["sjel_health".into(), "projects_list".into()],
        })
    }

    /// Reads a YouTube set's metadata and chapter-derived tracklist without downloading audio.
    pub async fn music_probe(&self, url: &str) -> ShellResult<MusicProbe> {
        self.json(
            Method::POST,
            "/music/api/probe",
            Some(serde_json::json!({ "url": url })),
            90,
        )
        .await
    }

    /// Starts the explicit album build in music.
    pub async fn music_build(&self, build: &MusicBuild) -> ShellResult<MusicJob> {
        self.json(
            Method::POST,
            "/music/api/jobs",
            Some(serde_json::to_value(build).map_err(|_| ShellError::Unreadable)?),
            10,
        )
        .await
    }

    /// Reads one build's progress and recent log.
    pub async fn music_job(&self, id: &str) -> ShellResult<MusicJob> {
        self.json(Method::GET, &format!("/music/api/jobs/{id}"), None, 10)
            .await
    }

    pub async fn soundscape_state(&self) -> ShellResult<SoundscapeState> {
        self.json(Method::GET, PROXY_ROUTE_SOUNDSCAPE_STATE, None, 3)
            .await
    }

    /// Update playback status (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_playing(&self, playing: bool) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "playing": playing,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Update soundscape preset (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_preset(&self, preset: &str) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "preset": preset,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Update the scene parameters (`POST /soundscape/api/soundscape/state`).
    ///
    /// The conductor's update is a patch, so this asserts exactly the six values the operator
    /// moved and leaves preset, layers and volume to whoever else is driving them.
    pub async fn set_soundscape_params(
        &self,
        params: SoundscapeParams,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "params": params,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Update the layer mix (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_layers(
        &self,
        layers: SoundscapeLayers,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "layers": layers,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Update the master volume (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_volume(&self, volume: f32) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "volume": volume.clamp(0., 1.),
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Update how full the arrangement is (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_energy(&self, energy: f32) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "energy": energy.clamp(0., 1.),
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Start a timed session (`POST /soundscape/api/soundscape/state`).
    pub async fn set_soundscape_session(
        &self,
        session: Option<SoundscapeSession>,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "session": session,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }
}
