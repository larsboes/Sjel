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
        let path = with_query(
            &proxied(finance_api::PROXY_PREFIX, finance_api::ROUTE_DASHBOARD),
            &query,
        );
        self.json(Method::GET, &path, None, 10).await
    }

    /// Read all subscriptions with prices and state history (`GET /finance/api/subscriptions`).
    pub async fn subscriptions(&self) -> ShellResult<Vec<SubscriptionRecord>> {
        self.json(
            Method::GET,
            &proxied(finance_api::PROXY_PREFIX, finance_api::ROUTE_SUBSCRIPTIONS),
            None,
            10,
        )
        .await
    }

    pub async fn finance_csv_mappings(&self) -> ShellResult<Vec<FinanceCsvMappingProfile>> {
        self.json(
            Method::GET,
            &proxied(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CSV_MAPPINGS,
            ),
            None,
            10,
        )
        .await
    }

    pub async fn finance_candidates(&self) -> ShellResult<Vec<FinanceCandidate>> {
        self.json(
            Method::GET,
            &proxied(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATES,
            ),
            None,
            10,
        )
        .await
    }

    pub async fn finance_preview_csv(
        &self,
        content: String,
        mapping: serde_json::Value,
    ) -> ShellResult<FinanceCsvPreview> {
        self.json(
            Method::POST,
            &proxied(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CSV_PREVIEW,
            ),
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
            &proxied(finance_api::PROXY_PREFIX, finance_api::ROUTE_IMPORT_CSV),
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
            &proxied_id(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATE_REVIEW,
                &id.replace(':', "%3A"),
            ),
            Some(body(&finance_api::ReviewCandidateRequest {
                decision: decision.to_owned(),
                account: account.map(str::to_owned),
                source_account: source_account.map(str::to_owned),
            })?),
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
            &proxied_id(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATE_RECONCILE_TRANSFER,
                &id.replace(':', "%3A"),
            ),
            Some(body(&finance_api::ReconcileTransferRequest {
                counterpart_id: counterpart_id.to_owned(),
            })?),
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
            &proxied(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATES_RECLASSIFY_BATCH,
            ),
            Some(body(&finance_api::ReclassifyBatchRequest {
                items: items
                    .into_iter()
                    .map(|(id, account)| finance_api::ReclassifyItem { id, account })
                    .collect(),
            })?),
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
            &proxied_id(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATE_ALLOCATION,
                &id.replace(':', "%3A"),
            ),
            Some(body(&finance_api::AllocateExpenseRequest {
                personal_cents,
                purpose: purpose.to_owned(),
                trip_id: trip_id.map(str::to_owned),
            })?),
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
            &proxied_id(
                finance_api::PROXY_PREFIX,
                finance_api::ROUTE_IMPORT_CANDIDATE_REIMBURSEMENT,
                &id.replace(':', "%3A"),
            ),
            Some(body(&finance_api::LinkReimbursementRequest {
                expense_candidate_id: expense_candidate_id.to_owned(),
            })?),
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
                &proxied(
                    finance_api::PROXY_PREFIX,
                    finance_api::ROUTE_BALANCE_SNAPSHOT,
                ),
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
        let path = proxied(
            inventory_api::PROXY_PREFIX,
            &inventory_api::ROUTE_ITEM_REFS.replace("{reference}", &segment(reference)),
        );
        self.json(Method::GET, &path, None, 10).await
    }
    pub async fn inventory_mail_proposals(
        &self,
    ) -> ShellResult<Vec<crate::inventory::MailProposal>> {
        let result: crate::inventory::MailProposals = self
            .json(
                Method::GET,
                &proxied(
                    inventory_api::PROXY_PREFIX,
                    inventory_api::ROUTE_MAIL_PROPOSALS,
                ),
                None,
                10,
            )
            .await?;
        Ok(result.proposals)
    }

    pub async fn inventory_mail_snapshot(
        &self,
        id: &str,
    ) -> ShellResult<crate::inventory::MailMatchSnapshot> {
        let path = proxied_id(
            inventory_api::PROXY_PREFIX,
            inventory_api::ROUTE_MAIL_MATCH_SNAPSHOT,
            &segment(id),
        );
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn inventory_mail_decision(
        &self,
        id: i64,
        body: serde_json::Value,
    ) -> ShellResult<()> {
        self.send(
            Method::POST,
            &proxied_id(
                inventory_api::PROXY_PREFIX,
                inventory_api::ROUTE_MAIL_PROPOSAL_DECISION,
                &id.to_string(),
            ),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    pub async fn inventory_rows(&self) -> ShellResult<Vec<crate::inventory::InventoryRow>> {
        self.json(
            Method::GET,
            &proxied(inventory_api::PROXY_PREFIX, inventory_api::ROUTE_INVENTORY),
            None,
            10,
        )
        .await
    }

    pub async fn inventory_wishlist(&self) -> ShellResult<crate::inventory::InventoryWishlist> {
        self.json(
            Method::GET,
            &proxied(inventory_api::PROXY_PREFIX, inventory_api::ROUTE_WISHLIST),
            None,
            10,
        )
        .await
    }

    /// The wish groups, each with its members and what they cost.
    pub async fn inventory_wish_groups(&self) -> ShellResult<Vec<crate::inventory::WishGroup>> {
        let envelope: crate::inventory::WishGroups = self
            .json(
                Method::GET,
                &proxied(
                    inventory_api::PROXY_PREFIX,
                    inventory_api::ROUTE_WISH_GROUPS,
                ),
                None,
                10,
            )
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
            &proxied(
                inventory_api::PROXY_PREFIX,
                inventory_api::ROUTE_WISH_GROUPS,
            ),
            Some(body(&inventory_api::SaveWishGroupRequest {
                id: id.to_owned(),
                title: title.to_owned(),
                constraint_text: constraint_text.to_owned(),
                item_ids: None,
                alternatives: None,
            })?),
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
        self.send(
            Method::PUT,
            &proxied_id(
                inventory_api::PROXY_PREFIX,
                inventory_api::ROUTE_WISH_GROUP_ITEMS,
                &segment(id),
            ),
            Some(body(&inventory_api::SetWishGroupItemsRequest {
                item_ids: item_ids.to_vec(),
            })?),
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
        let path = proxied_id(
            inventory_api::PROXY_PREFIX,
            inventory_api::ROUTE_WISH_GROUP_BY_ID,
            &segment(id),
        );
        self.send(Method::DELETE, &path, None, 10).await.map(drop)
    }

    /// Creates an inventory entry through the inventory capability.
    pub async fn inventory_create_item(
        &self,
        item: &crate::inventory::NewInventoryItem,
    ) -> ShellResult<()> {
        self.send(
            Method::POST,
            &proxied(inventory_api::PROXY_PREFIX, inventory_api::ROUTE_ITEMS),
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
        self.json(
            Method::POST,
            &proxied_id(
                inventory_api::PROXY_PREFIX,
                inventory_api::ROUTE_ITEM_STATE,
                &segment(id),
            ),
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
        let path = proxied_id(
            inventory_api::PROXY_PREFIX,
            inventory_api::ROUTE_ITEM_BY_ID,
            &segment(id),
        );
        self.json(Method::PATCH, &path, Some(patch), 10).await
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
        let path = proxied_id(
            inventory_api::PROXY_PREFIX,
            inventory_api::ROUTE_ITEM_STATE,
            &segment(id),
        );
        self.json(Method::GET, &path, None, 10).await
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
        self.json(
            Method::GET,
            interior_api::PROXY_ROUTE_INTERIOR_LAYOUTS,
            None,
            10,
        )
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
            &format!("{}/{id}", interior_api::PROXY_ROUTE_INTERIOR_LAYOUTS),
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
            &format!("{}/preview", interior_api::PROXY_ROUTE_INTERIOR_PLACEMENTS),
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
        let request = interior_api::SearchRequest {
            layout: layout.to_owned(),
            move_refs: move_refs.to_vec(),
            ..interior_api::SearchRequest::default()
        };
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_SEARCH,
            Some(body(&request)?),
            10,
        )
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
        let request = interior_api::ComposeRequest {
            refs: refs.to_vec(),
            ..interior_api::ComposeRequest::default()
        };
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_COMPOSE,
            Some(body(&request)?),
            10,
        )
        .await
    }

    pub async fn interior_planning_job(
        &self,
        id: u64,
    ) -> ShellResult<crate::interior::RoomPlanJob> {
        self.json(
            Method::GET,
            &format!("{}/{id}", interior_api::PROXY_ROUTE_INTERIOR_JOBS),
            None,
            10,
        )
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
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_LAYOUTS,
            Some(body),
            30,
        )
        .await
    }

    pub async fn interior_model(&self) -> ShellResult<crate::interior::RoomPlanModel> {
        self.json(
            Method::GET,
            interior_api::PROXY_ROUTE_INTERIOR_MODEL,
            None,
            10,
        )
        .await
    }

    pub async fn interior_placements(
        &self,
        flat: &str,
    ) -> ShellResult<Vec<crate::interior::RoomPlanPlacement>> {
        let path = format!("{}/{flat}", interior_api::PROXY_ROUTE_INTERIOR_PLACEMENTS);
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn interior_roomplan_apply_placement(
        &self,
        update: crate::interior::PlacementUpdate<'_>,
    ) -> ShellResult<serde_json::Value> {
        let path = format!(
            "{}/{}/placements/{}",
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES,
            update.space_id,
            update.item_id
        );
        let request = interior_api::RoomPlanPlacementRequest {
            revision_id: update.revision_id.to_owned(),
            observation_id: update.observation_id.to_owned(),
            expected_revision: update.expected_revision,
            x: update.x,
            y: update.y,
            rot: update.rotation,
        };
        self.json(Method::PUT, &path, Some(body(&request)?), 30)
            .await
    }

    pub async fn interior_roomplan_propose_placement(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanPlacementProposalResponse> {
        let request = interior_api::RoomPlanPlacementProposalRequest {
            space_id: space_id.to_owned(),
            revision_id: revision_id.to_owned(),
            observation_id: observation_id.to_owned(),
            item_id: item_id.to_owned(),
        };
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_PROPOSE_PLACEMENT,
            Some(body(&request)?),
            30,
        )
        .await
    }

    /// Reads the semantic RoomPlan scans archived by the interior capability.
    pub async fn interior_roomplan_revisions(
        &self,
    ) -> ShellResult<crate::interior::RoomPlanRevisionSet> {
        self.json(
            Method::GET,
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_REVISIONS,
            None,
            10,
        )
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
                &format!(
                    "{}/{revision_id}/asset",
                    interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_REVISIONS
                ),
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
        self.json(
            Method::GET,
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES,
            None,
            10,
        )
        .await
    }

    pub async fn interior_roomplan_create_space(
        &self,
        revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanSpaceReceipt> {
        let request = interior_api::RoomPlanSpaceCreateRequest {
            revision_id: revision_id.to_owned(),
            label: None,
        };
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES,
            Some(body(&request)?),
            10,
        )
        .await
    }

    pub async fn interior_roomplan_add_revision(
        &self,
        space_id: &str,
        revision_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!(
            "{}/{space_id}/revisions",
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES
        );
        let request = interior_api::RoomPlanSpaceRevisionRequest {
            revision_id: revision_id.to_owned(),
        };
        self.json(Method::POST, &path, Some(body(&request)?), 10)
            .await
    }

    pub async fn interior_roomplan_link_object(
        &self,
        space_id: &str,
        revision_id: &str,
        observation_id: &str,
        item_id: &str,
    ) -> ShellResult<serde_json::Value> {
        let path = format!(
            "{}/{space_id}/links",
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_SPACES
        );
        let request = interior_api::RoomPlanObjectLinkRequest {
            revision_id: revision_id.to_owned(),
            observation_id: observation_id.to_owned(),
            item_id: item_id.to_owned(),
        };
        self.json(Method::PUT, &path, Some(body(&request)?), 10)
            .await
    }

    /// Aligns two archived RoomPlan captures and returns review-only match suggestions.
    pub async fn interior_roomplan_reconcile(
        &self,
        before_revision_id: &str,
        after_revision_id: &str,
    ) -> ShellResult<crate::interior::RoomPlanReconciliationResponse> {
        let request = interior_api::RoomPlanReconcileRequest {
            before_revision_id: before_revision_id.to_owned(),
            after_revision_id: after_revision_id.to_owned(),
        };
        self.json(
            Method::POST,
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_RECONCILE,
            Some(body(&request)?),
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
        let path = format!(
            "{}/{revision_id}/review",
            interior_api::PROXY_ROUTE_INTERIOR_ROOMPLAN_REVISIONS
        );
        let request = interior_api::RoomPlanReviewRequest {
            decision: match decision {
                crate::interior::RoomPlanDecision::Accept => "accept",
                crate::interior::RoomPlanDecision::Reject => "reject",
            }
            .to_owned(),
            note: None,
        };
        self.json(Method::POST, &path, Some(body(&request)?), 10)
            .await
    }
}

fn valid_idea_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
