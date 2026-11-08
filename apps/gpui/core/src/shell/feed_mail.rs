//! Feed, Captures, Mail & Scouting shell client methods.

use super::*;

impl Shell {
    /// The feed for the last `days` days, dismissed entries included so Kept and Dismissed can
    /// be shown without a second read.
    pub async fn feed(&self, days: u32) -> ShellResult<Vec<FeedEntry>> {
        let days = days.to_string();
        let path = with_query(
            ROUTE_COMMS_FEED,
            &[("days", &days), ("include_dismissed", "true")],
        );
        match self.json(Method::GET, &path, None, 15).await? {
            FeedList::Rows(rows) => Ok(rows),
            FeedList::Failed { error } => Err(ShellError::Refused(500, Some(error))),
        }
    }

    pub async fn feed_detail(&self, id: &str) -> ShellResult<FeedDetail> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        self.json(Method::GET, &format!("{ROUTE_COMMS_FEED}/{id}"), None, 10)
            .await
    }

    pub async fn content_item_detail(
        &self,
        source: &str,
        id: &str,
    ) -> ShellResult<FeedContentDetail> {
        let valid_source = matches!(source, "feed" | "mail" | "calendar");
        let allowed_id_byte = |byte: u8| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-')
                || (source == "calendar" && byte == b':')
        };
        let valid_id = !id.is_empty() && id.len() <= 128 && id.bytes().all(allowed_id_byte);
        if !valid_source || !valid_id || (source == "feed" && !is_sha256(id)) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid content source or id.".into()),
            ));
        }
        let path = match source {
            "feed" | "mail" => format!("/comms/content/{source}/{id}"),
            "calendar" => format!("/calendar/api/content/calendar/{id}"),
            _ => unreachable!("validated content source"),
        };
        self.json(Method::GET, &path, None, 10).await
    }

    /// Build the local-only derivative the operator can inspect before approval.
    pub async fn content_cloud_preview(
        &self,
        source: &str,
        id: &str,
    ) -> ShellResult<CloudDerivativePreview> {
        if !valid_cloud_content_id(source, id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid cloud preview source or id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/{source}/{id}/cloud-preview"),
            None,
            30,
        )
        .await
    }

    /// Approval binds to the exact hash the operator reviewed.
    pub async fn content_cloud_approve(
        &self,
        source: &str,
        id: &str,
        preview_hash: &str,
    ) -> ShellResult<FeedCloudProcessingState> {
        if !valid_cloud_content_id(source, id) || !is_sha256(preview_hash) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid cloud approval source, id, or preview hash.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/{source}/{id}/cloud-approval"),
            Some(serde_json::json!({ "preview_hash": preview_hash })),
            30,
        )
        .await
    }

    pub async fn content_cloud_providers(&self) -> ShellResult<Vec<CloudProviderOption>> {
        self.json(Method::GET, ROUTE_COMMS_CONTENT_CLOUD_PROVIDERS, None, 10)
            .await
    }

    pub async fn content_cloud_queue(
        &self,
        source: &str,
        id: &str,
        preview_hash: &str,
        provider_role: &str,
    ) -> ShellResult<FeedCloudProcessingState> {
        let role = provider_role.trim();
        if !valid_cloud_content_id(source, id) || !is_sha256(preview_hash) || role.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Invalid cloud queue source, id, preview hash, or provider role.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/{source}/{id}/cloud-queue"),
            Some(serde_json::json!({
                "preview_hash": preview_hash,
                "provider_role": role,
            })),
            30,
        )
        .await
    }

    pub async fn content_cloud_run_job(
        &self,
        job_id: &str,
    ) -> ShellResult<FeedCloudProcessingState> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Cloud job id is required.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/comms/content/cloud-jobs/{job_id}/run"),
            Some(serde_json::json!({})),
            60,
        )
        .await
    }

    pub async fn set_content_data_class(
        &self,
        source: &str,
        id: &str,
        data_class: &str,
        rationale: Option<&str>,
    ) -> ShellResult<()> {
        if !valid_cloud_content_id(source, id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid content source or id.".into()),
            ));
        }
        if !matches!(data_class, "c0" | "c1" | "c2" | "c3") {
            return Err(ShellError::Refused(
                400,
                Some("Data class must be c0, c1, c2, or c3.".into()),
            ));
        }
        let path = match source {
            "feed" => format!("/comms/feed/{id}/data-class"),
            "mail" => format!("/comms/triage/{id}/data-class"),
            _ => unreachable!("validated content source"),
        };
        let rationale = rationale.map(str::trim).filter(|r| !r.is_empty());
        let body = serde_json::json!({
            "data_class": data_class,
            "rationale": rationale,
        });
        self.send(Method::POST, &path, Some(body), 10)
            .await
            .map(drop)
    }

    /// Keep (`keeper`), dismiss (`dismissed`) or undo (`new`). The only route that writes those
    /// decisions; it records the ledger row in the same transaction.
    pub async fn set_feed_status(
        &self,
        id: &str,
        status: &str,
        surface: FeedSurface,
    ) -> ShellResult<()> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        let body = serde_json::json!({"status": status, "surface": surface.as_str()});
        self.send(
            Method::POST,
            &format!("/comms/feed/{id}/status"),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// Record that the reader opened an entry: `opened` the first time, `reopened` after a
    /// decision. The learned ranking factor counts these.
    pub async fn record_feed_open(&self, id: &str, first: bool) -> ShellResult<()> {
        if !is_sha256(id) {
            return Err(ShellError::Refused(400, Some("Not a feed id.".into())));
        }
        let event = if first { "opened" } else { "reopened" };
        let body = serde_json::json!({"event": event, "surface": FeedSurface::Reader.as_str()});
        self.send(
            Method::POST,
            &format!("/comms/feed/{id}/interactions"),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// The stored review queue: one row per signal, as the last refresh computed it.
    pub async fn feed_quality(&self) -> ShellResult<Vec<FeedQualityFlag>> {
        self.json(
            Method::GET,
            &format!("{ROUTE_COMMS_FEED_QUALITY}?limit=500"),
            None,
            10,
        )
        .await
    }

    /// Recomputes the queue from stored evidence. Comms checks at most 500 entries and calls no
    /// provider, so this changes no feed decision.
    pub async fn refresh_feed_quality(&self) -> ShellResult<FeedQualityRefresh> {
        let body = serde_json::json!({"days": 3650});
        self.json(
            Method::POST,
            ROUTE_COMMS_FEED_QUALITY_REFRESH,
            Some(body),
            60,
        )
        .await
    }

    /// Configured Scouting sources. This includes source paths from the machine configuration;
    /// callers should use them only for source identity and labels, never display local paths.
    pub async fn scouting_sources(&self) -> ShellResult<crate::scouting::ScoutingSources> {
        self.json(
            Method::GET,
            crate::scouting::PROXY_ROUTE_SCOUTING_SOURCES,
            None,
            10,
        )
        .await
    }

    /// Stored opportunities for the Discover queue, including decisions the operator made.
    pub async fn scouting_opportunities(
        &self,
    ) -> ShellResult<crate::scouting::ScoutingOpportunities> {
        let path = with_query(
            crate::scouting::PROXY_ROUTE_SCOUTING_OPPORTUNITIES,
            &[("limit", "500"), ("include_dismissed", "true")],
        );
        self.json(Method::GET, &path, None, 20).await
    }

    /// Runs one selected source. The server intentionally treats this GET as a write because it
    /// fetches third-party pages and persists ranked results.
    pub async fn scouting_discover(
        &self,
        adapter: &str,
        location: Option<&str>,
        query: Option<&str>,
    ) -> ShellResult<crate::scouting::DiscoverResponse> {
        if !valid_scouting_id(adapter) {
            return Err(ShellError::Refused(
                400,
                Some("Not a Scouting adapter name.".into()),
            ));
        }
        let mut pairs = vec![("adapter", adapter), ("limit", "20")];
        if let Some(location) = location.filter(|value| !value.trim().is_empty()) {
            pairs.push(("location", location));
        }
        if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            pairs.push(("query", query));
        }
        let path = with_query(crate::scouting::PROXY_ROUTE_SCOUTING_DISCOVER, &pairs);
        let value: serde_json::Value = self.json(Method::GET, &path, None, 120).await?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return Err(ShellError::Refused(502, Some(error.to_owned())));
        }
        serde_json::from_value(value).map_err(|_| ShellError::Unreadable)
    }

    /// Saves, dismisses, or restores one stored opportunity.
    pub async fn scouting_set_status(
        &self,
        id: &str,
        status: crate::scouting::OpportunityStatus,
    ) -> ShellResult<()> {
        if !valid_scouting_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a Scouting opportunity id.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["scouting", "opportunities", id, "status"]);
        let path = url.path().to_owned();
        let body = serde_json::json!({"status": status.as_str()});
        self.send(Method::POST, &path, Some(body), 10)
            .await
            .map(drop)
    }

    /// The capture inbox, newest first.
    pub async fn captures(&self) -> ShellResult<Vec<Capture>> {
        self.json(Method::GET, "/comms/captures", None, 10).await
    }

    /// Stores `text` as typed; comms takes a leading `#buy`, `#look`, `#go` or `#learn` as the tag.
    pub async fn add_capture(&self, text: Option<&str>, url: Option<&str>) -> ShellResult<Capture> {
        let body = serde_json::json!({"text": text, "url": url, "client": "desktop"});
        self.json(Method::POST, "/comms/captures", Some(body), 10)
            .await
    }

    /// Changes the fields `patch` names (`text`, `tag`, `status`, `routed_to`).
    pub async fn patch_capture(&self, id: &str, patch: serde_json::Value) -> ShellResult<Capture> {
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ShellError::Refused(400, Some("Not a capture id.".into())));
        }
        self.json(
            Method::PATCH,
            &format!("/comms/captures/{id}"),
            Some(patch),
            10,
        )
        .await
    }

    /// Sends a capture on and marks it done. The capture stays as it is when the target refuses.
    /// Follows `route` in `dashboard/src/lib/capture.ts`.
    pub async fn route_capture(&self, capture: &Capture, to: Route) -> ShellResult<Capture> {
        let target = match to {
            Route::Feed => {
                #[derive(Deserialize)]
                struct Ingested {
                    id: String,
                }
                let body = serde_json::json!({"url": capture.url, "client": "inbox"});
                let item: Ingested = self
                    .json(Method::POST, "/comms/ingest", Some(body), 60)
                    .await?;
                format!("feed:{}", item.id)
            }
            Route::Places => {
                #[derive(Deserialize)]
                struct Created {
                    id: String,
                }
                let text = capture
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty());
                let url = capture
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|u| !u.is_empty());
                let name = text
                    .map(str::to_owned)
                    .or_else(|| capture::host(url))
                    .or_else(|| url.map(str::to_owned))
                    .unwrap_or_else(|| "Unbenannter Ort".into());
                let body = serde_json::json!({
                    "name": name, "link": url, "note": text, "query": text.or(url),
                });
                let place: Created = self
                    .json(Method::POST, "/places/api/places", Some(body), 20)
                    .await?;
                format!("places:{}", place.id)
            }
            Route::Wishlist => {
                let (label, price_cents) = capture::extract_price(capture.text.as_deref());
                let label = Some(label)
                    .filter(|l| !l.is_empty())
                    .or_else(|| capture::host(capture.url.as_deref()))
                    .or_else(|| capture.url.clone())
                    .unwrap_or_else(|| "wunsch".into());
                let base = capture::slug(&label);
                let mut n = 1;
                loop {
                    let id = if n == 1 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    };
                    let mut body = serde_json::json!({
                        "id": id, "kind": "piece", "label": label, "link": capture.url,
                        "state": "wanted", "note": "aus der Inbox",
                    });
                    if let Some(cents) = price_cents {
                        body["price_cents"] = cents.into();
                    }
                    // A taken id answers 409; the next suffix is tried, twenty at most.
                    match self
                        .send(Method::POST, "/inventory/api/items", Some(body), 10)
                        .await
                    {
                        Ok(_) => break format!("inventory:{id}"),
                        Err(ShellError::Refused(409, _)) if n <= 20 => n += 1,
                        Err(error) => return Err(error),
                    }
                }
            }
        };
        self.patch_capture(
            &capture.id,
            serde_json::json!({"status": "done", "routed_to": target}),
        )
        .await
    }

    /// Every mail proposal; the client filters by status and category.
    pub async fn mail_proposals(&self) -> ShellResult<Vec<MailProposal>> {
        self.json(Method::GET, "/comms/triage", None, 15).await
    }

    /// One action across `ids`. Comms answers per id, so a partial failure is a result, not an
    /// error. Archive and trash reach Gmail.
    pub async fn mail_action(
        &self,
        ids: &[String],
        action: &MailAction,
    ) -> ShellResult<BulkResult> {
        let body = mail_action_body(ids, action);
        self.json(Method::POST, "/comms/triage/bulk", Some(body), 60)
            .await
    }

    /// Retries or cancels a Gmail action that stopped after its automatic retries
    /// (`POST /comms/triage/:id/gmail-job`, `decision` is `retry` or `cancel`).
    pub async fn mail_gmail_job(&self, id: &str, decision: &str) -> ShellResult<()> {
        let Some(path) = mail_gmail_job_path(id) else {
            return Err(ShellError::Refused(400, Some("Invalid mail id.".into())));
        };
        let body = serde_json::json!({ "decision": decision });
        self.send(Method::POST, &path, Some(body), 60)
            .await
            .map(drop)
    }
}

fn valid_mail_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn mail_content_path(id: &str) -> Option<String> {
    valid_mail_id(id).then(|| format!("/comms/content/mail/{id}"))
}

fn mail_gmail_job_path(id: &str) -> Option<String> {
    valid_mail_id(id).then(|| format!("/comms/triage/{id}/gmail-job"))
}

fn mail_action_body(ids: &[String], action: &MailAction) -> serde_json::Value {
    let stream = match action {
        MailAction::Categorize(key) => Some(key.as_str()),
        _ => None,
    };
    let data_class = match action {
        MailAction::SetDataClass(class) => Some(class.as_str()),
        _ => None,
    };
    serde_json::json!({
        "ids": ids,
        "action": action.key(),
        "stream": stream,
        "data_class": data_class,
    })
}

impl crate::device::Source {
    /// Every mail proposal from the Mac's shell or the paired phone's client.
    pub async fn mail_proposals(&self) -> Result<Vec<MailProposal>, String> {
        match self {
            Self::Shell(shell) => shell.mail_proposals().await.map_err(|e| e.to_string()),
            Self::Device(client) => client.get("/comms/triage").await,
        }
    }

    /// The stored message content for one mail thread (`GET /comms/content/mail/{id}`).
    pub async fn mail_detail(&self, id: &str) -> Result<FeedContentDetail, String> {
        let path = mail_content_path(id).ok_or_else(|| "Not a mail id.".to_string())?;
        match self {
            Self::Shell(shell) => shell
                .content_item_detail("mail", id)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => client.get(&path).await,
        }
    }

    /// Apply one triage action across `ids` (`POST /comms/triage/bulk`).
    pub async fn mail_action(
        &self,
        ids: &[String],
        action: &MailAction,
    ) -> Result<BulkResult, String> {
        match self {
            Self::Shell(shell) => shell
                .mail_action(ids, action)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let body = serde_json::to_vec(&mail_action_body(ids, action))
                    .map_err(|e| e.to_string())?;
                client.post("/comms/triage/bulk", &body).await
            }
        }
    }

    /// Retry or cancel a stopped Gmail job (`POST /comms/triage/{id}/gmail-job`).
    pub async fn mail_gmail_job(&self, id: &str, decision: &str) -> Result<(), String> {
        let path = mail_gmail_job_path(id).ok_or_else(|| "Not a mail id.".to_string())?;
        match self {
            Self::Shell(shell) => shell
                .mail_gmail_job(id, decision)
                .await
                .map_err(|e| e.to_string()),
            Self::Device(client) => {
                let body = serde_json::to_vec(&serde_json::json!({ "decision": decision }))
                    .map_err(|e| e.to_string())?;
                client.send(Method::POST, &path, &body).await
            }
        }
    }
}

#[cfg(test)]
mod mail_path_tests {
    use super::*;

    #[test]
    fn mail_paths_accept_alphanumeric_ids_and_refuse_traversal() {
        assert_eq!(
            mail_content_path("18f9a_b-2"),
            Some("/comms/content/mail/18f9a_b-2".into())
        );
        assert_eq!(
            mail_gmail_job_path("18f9a_b-2"),
            Some("/comms/triage/18f9a_b-2/gmail-job".into())
        );
        assert_eq!(mail_content_path(""), None);
        assert_eq!(mail_content_path("../etc/passwd"), None);
        assert_eq!(mail_gmail_job_path("bad/id"), None);
    }
}
