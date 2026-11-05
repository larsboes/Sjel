//! Finance, Inventory & Interior shell client methods.

use super::*;

impl Shell {
    /// Read the finance projection and transactions for a date window (or all if None).
    pub async fn finance_dashboard(
        &self,
        start: Option<&str>,
        end: Option<&str>,
    ) -> ShellResult<FinanceDashboardProjection> {
        let mut query = Vec::new();
        if let Some(s) = start {
            query.push(("start", s));
        }
        if let Some(e) = end {
            query.push(("end", e));
        }
        let path = with_query("/finance/api/dashboard", &query);
        self.json(Method::GET, &path, None, 10).await
    }

    /// Read all subscriptions with prices and state history (`GET /finance/api/subscriptions`).
    pub async fn subscriptions(&self) -> ShellResult<Vec<SubscriptionRecord>> {
        self.json(Method::GET, "/finance/api/subscriptions", None, 10)
            .await
    }

    pub async fn finance_csv_mappings(&self) -> ShellResult<Vec<FinanceCsvMappingProfile>> {
        self.json(Method::GET, "/finance/api/import/csv/mappings", None, 10)
            .await
    }

    pub async fn finance_candidates(&self) -> ShellResult<Vec<FinanceCandidate>> {
        self.json(Method::GET, "/finance/api/import/candidates", None, 10)
            .await
    }

    pub async fn finance_preview_csv(
        &self,
        content: String,
        mapping: serde_json::Value,
    ) -> ShellResult<FinanceCsvPreview> {
        self.json(
            Method::POST,
            "/finance/api/import/csv/preview",
            Some(serde_json::json!({ "content": content, "mapping": mapping })),
            30,
        )
        .await
    }

    pub async fn finance_import_csv(
        &self,
        content: String,
        mapping: serde_json::Value,
        expected_preview_id: String,
    ) -> ShellResult<serde_json::Value> {
        self.json(
            Method::POST,
            "/finance/api/import/csv",
            Some(serde_json::json!({ "content": content, "mapping": mapping, "expected_preview_id": expected_preview_id })),
            30,
        )
        .await
    }

    pub async fn finance_review_candidate(
        &self,
        id: &str,
        decision: &str,
        account: Option<&str>,
        source_account: Option<&str>,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !matches!(decision, "confirm" | "reject") {
            return Err(ShellError::Refused(
                400,
                Some("Invalid finance review request.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/import/candidates/{}/review", id.replace(':', "%3A")),
            Some(serde_json::json!({ "decision": decision, "account": account, "source_account": source_account })),
            30,
        )
        .await
    }

    pub async fn finance_ideas(&self) -> ShellResult<Vec<super::FinanceIdea>> {
        self.json(Method::GET, "/finance/api/ideas", None, 10).await
    }

    /// Creates an idea when `id` is `None`, else replaces it with a new snapshot.
    pub async fn finance_save_idea(
        &self,
        id: Option<&str>,
        draft: &super::FinanceIdeaDraft,
    ) -> ShellResult<serde_json::Value> {
        let body = serde_json::to_value(draft).map_err(|_| ShellError::Unreachable)?;
        match id {
            None => {
                self.json(Method::POST, "/finance/api/ideas", Some(body), 30)
                    .await
            }
            Some(id) if valid_idea_id(id) => {
                self.json(
                    Method::PUT,
                    &format!("/finance/api/ideas/{id}"),
                    Some(body),
                    30,
                )
                .await
            }
            Some(_) => Err(ShellError::Refused(400, Some("Invalid idea id.".into()))),
        }
    }

    pub async fn finance_add_idea_note(
        &self,
        id: &str,
        text: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_idea_id(id) {
            return Err(ShellError::Refused(400, Some("Invalid idea id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/ideas/{id}/notes"),
            Some(serde_json::json!({ "text": text })),
            30,
        )
        .await
    }

    /// Asks the model for a review of one idea now. Slow: one cloud model call.
    pub async fn finance_review_idea(&self, id: &str) -> ShellResult<serde_json::Value> {
        if !valid_idea_id(id) {
            return Err(ShellError::Refused(400, Some("Invalid idea id.".into())));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/ideas/{id}/review"),
            Some(serde_json::json!({})),
            120,
        )
        .await
    }

    pub async fn finance_review_verdict(
        &self,
        id: &str,
        review_id: i64,
        verdict: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_idea_id(id) || !matches!(verdict, "accepted" | "rejected") {
            return Err(ShellError::Refused(
                400,
                Some("Invalid review verdict.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/ideas/{id}/reviews/{review_id}/verdict"),
            Some(serde_json::json!({ "verdict": verdict })),
            30,
        )
        .await
    }

    pub async fn finance_reconcile_transfer(
        &self,
        id: &str,
        counterpart_id: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !valid_typed_id(counterpart_id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid transfer candidate id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!(
                "/finance/api/import/candidates/{}/reconcile-transfer",
                id.replace(':', "%3A")
            ),
            Some(serde_json::json!({ "counterpart_id": counterpart_id })),
            30,
        )
        .await
    }

    pub async fn finance_reclassify_candidates(
        &self,
        items: Vec<(String, String)>,
    ) -> ShellResult<serde_json::Value> {
        self.json(
            Method::POST,
            "/finance/api/import/candidates/reclassify-batch",
            Some(serde_json::json!({
                "items": items.into_iter().map(|(id, account)| serde_json::json!({ "id": id, "account": account })).collect::<Vec<_>>()
            })),
            30,
        )
        .await
    }

    pub async fn finance_allocate_expense(
        &self,
        id: &str,
        personal_cents: i64,
        purpose: &str,
        trip_id: Option<&str>,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id)
            || personal_cents < 0
            || !matches!(
                purpose,
                "day_to_day" | "trip" | "work" | "housing" | "other"
            )
            || (purpose == "trip" && trip_id.is_none_or(str::is_empty))
        {
            return Err(ShellError::Refused(
                400,
                Some("Invalid expense allocation.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!("/finance/api/import/candidates/{}/allocation", id.replace(':', "%3A")),
            Some(serde_json::json!({ "personal_cents": personal_cents, "purpose": purpose, "trip_id": trip_id })),
            30,
        )
        .await
    }

    pub async fn finance_link_reimbursement(
        &self,
        id: &str,
        expense_candidate_id: &str,
    ) -> ShellResult<serde_json::Value> {
        if !valid_typed_id(id) || !valid_typed_id(expense_candidate_id) {
            return Err(ShellError::Refused(
                400,
                Some("Invalid reimbursement candidate id.".into()),
            ));
        }
        self.json(
            Method::POST,
            &format!(
                "/finance/api/import/candidates/{}/reimbursement",
                id.replace(':', "%3A")
            ),
            Some(serde_json::json!({ "expense_candidate_id": expense_candidate_id })),
            30,
        )
        .await
    }

    pub async fn finance_update_balance_snapshot(
        &self,
        update: ManualBalanceUpdate,
    ) -> ShellResult<ManualBalanceSnapshot> {
        let value: serde_json::Value = self
            .json(
                Method::POST,
                "/finance/api/balance-snapshot",
                Some(serde_json::to_value(update).map_err(|_| ShellError::Unreadable)?),
                30,
            )
            .await?;
        serde_json::from_value(
            value
                .get("snapshot")
                .cloned()
                .ok_or(ShellError::Unreadable)?,
        )
        .map_err(|_| ShellError::Unreadable)
    }

    pub async fn inventory_identity(&self, reference: &str) -> ShellResult<InventoryIdentity> {
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "item-refs", reference]);
        self.json(Method::GET, url.path(), None, 10).await
    }
    pub async fn inventory_mail_proposals(
        &self,
    ) -> ShellResult<Vec<crate::inventory::MailProposal>> {
        let result: crate::inventory::MailProposals = self
            .json(Method::GET, "/inventory/api/mail-proposals", None, 10)
            .await?;
        Ok(result.proposals)
    }

    pub async fn inventory_mail_snapshot(
        &self,
        id: &str,
    ) -> ShellResult<crate::inventory::MailMatchSnapshot> {
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id, "mail-match-snapshot"]);
        self.json(Method::GET, url.path(), None, 10).await
    }

    pub async fn inventory_mail_decision(
        &self,
        id: i64,
        body: serde_json::Value,
    ) -> ShellResult<()> {
        self.send(
            Method::POST,
            &format!("/inventory/api/mail-proposals/{id}/decision"),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    pub async fn inventory_rows(&self) -> ShellResult<Vec<crate::inventory::InventoryRow>> {
        self.json(Method::GET, "/inventory/api/inventory", None, 10)
            .await
    }

    pub async fn inventory_wishlist(&self) -> ShellResult<crate::inventory::InventoryWishlist> {
        self.json(Method::GET, "/inventory/api/wishlist", None, 10)
            .await
    }

    /// The wish groups, each with its members and what they cost.
    pub async fn inventory_wish_groups(&self) -> ShellResult<Vec<crate::inventory::WishGroup>> {
        let envelope: crate::inventory::WishGroups = self
            .json(Method::GET, "/inventory/api/wish-groups", None, 10)
            .await?;
        Ok(envelope.wish_groups)
    }

    /// Creates a wish group, or retitles the one that already has this id.
    pub async fn inventory_save_wish_group(
        &self,
        id: &str,
        title: &str,
        constraint_text: &str,
    ) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        self.send(
            Method::POST,
            "/inventory/api/wish-groups",
            Some(serde_json::json!({
                "id": id,
                "title": title,
                "constraint_text": constraint_text,
            })),
            10,
        )
        .await
        .map(drop)
    }

    /// Replaces a group's members with exactly these item ids.
    pub async fn inventory_set_wish_group_items(
        &self,
        id: &str,
        item_ids: &[String],
    ) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "wish-groups", id, "items"]);
        self.send(
            Method::PUT,
            url.path(),
            Some(serde_json::json!({ "item_ids": item_ids })),
            10,
        )
        .await
        .map(drop)
    }

    /// Deletes a wish group and its memberships. The wishes themselves stay.
    pub async fn inventory_delete_wish_group(&self, id: &str) -> ShellResult<()> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A wish group id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "wish-groups", id]);
        self.send(Method::DELETE, url.path(), None, 10)
            .await
            .map(drop)
    }

    /// Creates an inventory entry through the inventory capability.
    pub async fn inventory_create_item(
        &self,
        item: &crate::inventory::NewInventoryItem,
    ) -> ShellResult<()> {
        self.send(
            Method::POST,
            "/inventory/api/items",
            Some(serde_json::to_value(item).map_err(|_| ShellError::Unreadable)?),
            10,
        )
        .await
        .map(drop)
    }

    /// Appends a state transition for an inventory item.
    pub async fn inventory_set_state(
        &self,
        id: &str,
        state: crate::inventory::InventoryState,
        note: Option<&str>,
    ) -> ShellResult<crate::inventory::InventoryStateChange> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id, "state"]);
        self.json(
            Method::POST,
            url.path(),
            Some(serde_json::json!({"state": state.as_str(), "note": note})),
            10,
        )
        .await
    }

    /// Applies only named item fields and refuses a stale revision rather than overwriting it.
    pub async fn inventory_patch_item(
        &self,
        id: &str,
        revision: i64,
        mut patch: serde_json::Value,
    ) -> ShellResult<crate::inventory::InventoryItemWrite> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let Some(fields) = patch.as_object_mut() else {
            return Err(ShellError::Refused(
                400,
                Some("An inventory patch must be an object.".into()),
            ));
        };
        fields.insert("expected_revision".to_owned(), revision.into());
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id]);
        self.json(Method::PATCH, url.path(), Some(patch), 10).await
    }

    pub async fn inventory_state_history(
        &self,
        id: &str,
    ) -> ShellResult<Vec<crate::inventory::InventoryStateHistoryEntry>> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["inventory", "api", "items", id, "state"]);
        self.json(Method::GET, url.path(), None, 10).await
    }

    pub async fn interior_item_impact(
        &self,
        id: &str,
        patch: serde_json::Value,
    ) -> ShellResult<crate::inventory::InteriorItemImpact> {
        if id.trim().is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("An inventory item id is required.".into()),
            ));
        }
        let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
        url.path_segments_mut()
            .expect("a hierarchical URL")
            .extend(["interior", "api", "items", id, "impact"]);
        self.json(Method::POST, url.path(), Some(patch), 15).await
    }

    pub async fn interior_layouts(
        &self,
    ) -> ShellResult<Vec<crate::inventory::InteriorLayoutSummary>> {
        self.json(Method::GET, "/interior/api/layouts", None, 10)
            .await
    }

    pub async fn interior_layout_preview(
        &self,
        id: &str,
    ) -> ShellResult<crate::interior::RoomPlanSavedLayoutPreview> {
        if !crate::interior::valid_roomplan_layout_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not an interior layout id.".into()),
            ));
        }
        self.json(
            Method::GET,
            &format!("/interior/api/layouts/{id}"),
            None,
            20,
        )
        .await
    }

    pub async fn interior_preview_placements(
        &self,
        items: &[serde_json::Value],
    ) -> ShellResult<serde_json::Value> {
        let body = serde_json::json!({ "items": items });
        self.json(
            Method::POST,
            "/interior/api/placements/preview",
            Some(body),
            30,
        )
        .await
    }

    pub async fn interior_search_layout(
        &self,
        layout: &str,
        move_refs: &[String],
    ) -> ShellResult<crate::interior::RoomPlanJobReceipt> {
        if !crate::interior::valid_roomplan_layout_id(layout) || move_refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose a saved layout with movable items.".into()),
            ));
        }
        let body =
            serde_json::json!({ "layout": layout, "move_refs": move_refs, "step": 20, "limit": 6 });
        self.json(Method::POST, "/interior/api/search", Some(body), 10)
            .await
    }

    pub async fn interior_compose_layout(
        &self,
        refs: &[String],
    ) -> ShellResult<crate::interior::RoomPlanJobReceipt> {
        if refs.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("Choose at least one item to arrange.".into()),
            ));
        }
        let body = serde_json::json!({ "refs": refs, "step": 25, "beam": 60, "limit": 5, "rotations": [0, 90] });
        self.json(Method::POST, "/interior/api/compose", Some(body), 10)
            .await
    }

    pub async fn interior_planning_job(
        &self,
        id: u64,
    ) -> ShellResult<crate::interior::RoomPlanJob> {
        self.json(Method::GET, &format!("/interior/api/jobs/{id}"), None, 10)
            .await
    }

    pub async fn interior_save_proposal(
        &self,
        id: &str,
        name: &str,
        items: Vec<serde_json::Value>,
        note: &str,
    ) -> ShellResult<serde_json::Value> {
        if !crate::interior::valid_roomplan_layout_id(id) || items.is_empty() {
            return Err(ShellError::Refused(
                400,
                Some("A valid layout name and at least one placed item are required.".into()),
            ));
        }
        let body = serde_json::json!({ "id": id, "name": name, "items": items, "note_text": note });
        self.json(Method::POST, "/interior/api/layouts", Some(body), 30)
            .await
    }

    pub async fn interior_model(&self) -> ShellResult<crate::interior::RoomPlanModel> {
        self.json(Method::GET, "/interior/api/model", None, 10)
            .await
    }

    pub async fn interior_placements(
        &self,
        flat: &str,
    ) -> ShellResult<Vec<crate::interior::RoomPlanPlacement>> {
        let path = format!("/interior/api/placements/{flat}");
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn interior_roomplan_apply_placement(
        &self,
        update: crate::interior::PlacementUpdate<'_>,
    ) -> ShellResult<serde_json::Value> {
        let path = format!(
            "/interior/api/roomplan/spaces/{}/placements/{}",
            update.space_id, update.item_id
        );
        let body = serde_json::json!({
            "revision_id": update.revision_id,
            "observation_id": update.observation_id,
            "expected_revision": update.expected_revision,
            "x": update.x,
            "y": update.y,
            "rot": update.rotation
        });
        self.json(Method::PUT, &path, Some(body), 30).await
    }

    pub async fn interior_roomplan_propose_placement(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanPlacementProposalResponse> {
        let body = serde_json::json!({
            "space_id": space_id,
            "revision_id": revision_id,
            "observation_id": observation_id,
            "item_id": item_id
        });
        self.json(
            Method::POST,
            "/interior/api/roomplan/propose-placement",
            Some(body),
            30,
        )
        .await
    }

    /// Reads the semantic RoomPlan scans archived by the interior capability.
    pub async fn interior_roomplan_revisions(
        &self,
    ) -> ShellResult<crate::interior::RoomPlanRevisionSet> {
        self.json(Method::GET, "/interior/api/roomplan/revisions", None, 10)
            .await
    }

    /// Reads one immutable RoomPlan revision's parametric USDZ asset.
    pub async fn interior_roomplan_revision_asset(
        &self,
        revision_id: &str,
    ) -> ShellResult<Vec<u8>> {
        let response = self
            .send(
                Method::GET,
                &format!("/interior/api/roomplan/revisions/{revision_id}/asset"),
                None,
                60,
            )
            .await?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|_| ShellError::Unreadable)
    }

    pub async fn interior_roomplan_spaces(&self) -> ShellResult<crate::interior::RoomPlanSpaceSet> {
        self.json(Method::GET, "/interior/api/roomplan/spaces", None, 10)
            .await
    }

    pub async fn interior_roomplan_create_space(
        &self,
        revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanSpaceReceipt> {
        let body = serde_json::json!({ "revision_id": revision_id });
        self.json(
            Method::POST,
            "/interior/api/roomplan/spaces",
            Some(body),
            10,
        )
        .await
    }

    pub async fn interior_roomplan_add_revision(
        &self,
        space_id: &str,
        revision_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!("/interior/api/roomplan/spaces/{space_id}/revisions");
        let body = serde_json::json!({ "revision_id": revision_id });
        self.json(Method::POST, &path, Some(body), 10).await
    }

    pub async fn interior_roomplan_link_object(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!("/interior/api/roomplan/spaces/{space_id}/links");
        let body = serde_json::json!({
            "revision_id": revision_id,
            "observation_id": observation_id,
            "item_id": item_id
        });
        self.json(Method::PUT, &path, Some(body), 10).await
    }

    /// Aligns two archived RoomPlan captures and returns review-only match suggestions.
    pub async fn interior_roomplan_reconcile(
        &self,
        before_revision_id: &str,
        after_revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanReconciliationResponse> {
        let body = serde_json::json!({
            "before_revision_id": before_revision_id,
            "after_revision_id": after_revision_id
        });
        self.json(
            Method::POST,
            "/interior/api/roomplan/reconcile",
            Some(body),
            30,
        )
        .await
    }

    /// Records an accept or reject decision for one RoomPlan revision.
    pub async fn interior_roomplan_review(
        &self,
        revision_id: &str,
        decision: crate::interior::RoomPlanDecision,
    ) -> ShellResult<crate::interior::RoomPlanReview> {
        let path = format!("/interior/api/roomplan/revisions/{revision_id}/review");
        let body = serde_json::json!({
            "decision": match decision {
                crate::interior::RoomPlanDecision::Accept => "accept",
                crate::interior::RoomPlanDecision::Reject => "reject",
            }
        });
        self.json(Method::POST, &path, Some(body), 10).await
    }
}

fn valid_idea_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
