//! People, Calendar, Travel & Places shell client methods.

use super::*;

impl Shell {
    /// Reads the trips capability's cost response.
    ///
    /// # Errors
    /// Returns a refused route, transport or response error.
    pub async fn trip_cost(&self, id: &str) -> ShellResult<trips_api::cost::CostRollup> {
        let path = crate::travel::plan_path(id, &["cost"]).map_err(refused)?;
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn calendar_entry(&self, id: &str) -> ShellResult<CalendarEntryDetail> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::GET,
            &proxied_id(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_ENTRY_BY_ID,
                id,
            ),
            None,
            10,
        )
        .await
    }

    pub async fn travel_plans(&self) -> ShellResult<Vec<crate::travel::Plan>> {
        self.json(
            Method::GET,
            &proxied(trips_api::PROXY_PREFIX, trips_api::ROUTE_PLANS),
            None,
            10,
        )
        .await
    }

    /// A read from transit. Only `/transit/api/` paths, which `travel::connections` builds.
    pub async fn transit<T: DeserializeOwned>(&self, path: &str) -> ShellResult<T> {
        if !path.starts_with("/transit/api/") || path.contains("..") {
            return Err(ShellError::Refused(400, Some("Not a transit path.".into())));
        }
        // A split search asks for many fares, one stop pair at a time.
        self.json(Method::GET, path, None, 90).await
    }

    pub async fn travel_details(&self, id: &str) -> ShellResult<crate::travel::Details> {
        let path = crate::travel::details_path(id)
            .map_err(|error| ShellError::Refused(400, Some(error)))?;
        self.json(Method::GET, &path, None, 10).await
    }

    /// Deletes the plan only if it is still the revision the user saw.
    pub async fn trip_plan_delete(&self, id: &str) -> ShellResult<()> {
        let path = crate::travel::details_path(id)
            .map_err(|error| ShellError::Refused(400, Some(error)))?;
        let current: serde_json::Value = self.json(Method::GET, &path, None, 10).await?;
        let updated_at = current
            .get("updated_at")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ShellError::Refused(502, Some("The trip has no revision.".into())))?;
        let mut url = reqwest::Url::parse("http://localhost/")
            .map_err(|error| ShellError::Refused(400, Some(error.to_string())))?;
        url.set_path(&path);
        url.query_pairs_mut()
            .append_pair("expected_updated_at", updated_at);
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        self.send(Method::DELETE, &path, None, 10).await?;
        Ok(())
    }

    /// `POST /trips/api/plans` with a body from [`crate::travel::edit::TripDraft::body`].
    pub async fn trip_create(&self, body: serde_json::Value) -> ShellResult<crate::travel::Plan> {
        let plan: crate::travel::Plan = self
            .json(
                Method::POST,
                &proxied(trips_api::PROXY_PREFIX, trips_api::ROUTE_PLANS),
                Some(body),
                10,
            )
            .await?;
        self.trip_calendar_sync(&plan.id).await;
        Ok(plan)
    }

    /// Adds an itinerary item, including held transport offers and confirmed bookings.
    ///
    /// # Errors
    /// Returns an invalid plan identifier, transport or server error.
    pub async fn trip_item_add(&self, plan: &str, body: serde_json::Value) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items"]).map_err(refused)?;
        self.send(Method::POST, &path, Some(body), 10).await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Fields, then the day: only the item route moves an item between days.
    pub async fn trip_booking_edit(
        &self,
        plan: &str,
        item: &str,
        fields: serde_json::Value,
        day: Option<String>,
    ) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items", item, "fields"]).map_err(refused)?;
        self.send(Method::PATCH, &path, Some(fields), 10).await?;
        let path = crate::travel::plan_path(plan, &["items", item]).map_err(refused)?;
        self.send(
            Method::PATCH,
            &path,
            Some(serde_json::json!({ "day": day })),
            10,
        )
        .await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    pub async fn trip_item_delete(&self, plan: &str, item: &str) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items", item]).map_err(refused)?;
        self.send(Method::DELETE, &path, None, 10).await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Updates fields on a trip item in place (`PATCH /trips/api/plans/{plan}/items/{item}/fields`),
    /// such as setting or clearing a `stay` candidate's `rank`.
    pub async fn trip_item_fields(
        &self,
        plan: &str,
        item: &str,
        body: serde_json::Value,
    ) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &["items", item, "fields"]).map_err(refused)?;
        self.send(Method::PATCH, &path, Some(body), 10).await?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Read, edit, write back with the revision read, so a concurrent writer gets a 409.
    pub async fn trip_stages(
        &self,
        plan: &str,
        edit: &crate::travel::edit::StageEdit,
    ) -> ShellResult<()> {
        let path = crate::travel::plan_path(plan, &[]).map_err(refused)?;
        let current: serde_json::Value = self.json(Method::GET, &path, None, 10).await?;
        let mut stages = current
            .get("stages")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        edit.apply(&mut stages).map_err(refused)?;
        let body = serde_json::json!({
            "stages": stages,
            "expected_updated_at": current.get("updated_at"),
        });
        match self.send(Method::PATCH, &path, Some(body), 10).await {
            Err(ShellError::Refused(409, _)) => Err(ShellError::Refused(
                409,
                Some(
                    "The trip changed on the Mac while this was open. Refresh and try again."
                        .into(),
                ),
            )),
            other => other.map(drop),
        }?;
        self.trip_calendar_sync(plan).await;
        Ok(())
    }

    /// Asks trips to resolve coordinates for missing places on a plan, or refresh all coordinates.
    pub async fn trip_resolve_places(
        &self,
        id: &str,
        refresh: bool,
    ) -> ShellResult<crate::travel::ResolvePlacesResult> {
        let path = crate::travel::plan_path(id, &["resolve-places"]).map_err(refused)?;
        self.json(
            Method::POST,
            &path,
            Some(body(&trips_api::ResolvePlacesRequest { refresh })?),
            30,
        )
        .await
    }

    /// Calendar mirrors stages and booking deadlines only when asked. Best effort: the trip write
    /// already succeeded.
    async fn trip_calendar_sync(&self, plan: &str) {
        let path = proxied(
            calendar_api::PROXY_PREFIX,
            &calendar_api::ROUTE_TRIP_PLANS_SYNC.replace("{plan_id}", &segment(plan)),
        );
        let _ = self
            .send(Method::POST, &path, Some(serde_json::json!({})), 10)
            .await;
    }

    /// `places` resolves a typed name to a coordinate; a miss answers `place: null`.
    ///
    /// The 20 s budget covers the provider request behind the capability, which its own
    /// throttle may hold back — a shorter one would report a slow geocoder as a failure.
    pub async fn places_geocode(
        &self,
        query: &str,
    ) -> ShellResult<Option<crate::travel::Coordinate>> {
        let request = places_api::GeocodeRequest {
            query: Some(query.to_owned()),
            ..places_api::GeocodeRequest::default()
        };
        let answer: places_api::GeocodeResponse = self
            .json(
                Method::POST,
                &proxied(places_api::PROXY_PREFIX, places_api::ROUTE_PLACES_GEOCODE),
                Some(body(&request)?),
                20,
            )
            .await?;
        Ok(answer
            .place
            .and_then(|point| crate::travel::Coordinate::new(point.latitude?, point.longitude?)))
    }
    /// Entries overlapping `[from, to)`, both `YYYY-MM-DD`: `to` is exclusive.
    pub async fn calendar(&self, from: &str, to: &str) -> ShellResult<Vec<CalendarEntry>> {
        let path = format!(
            "{}?from={from}&to={to}",
            proxied(calendar_api::PROXY_PREFIX, calendar_api::ROUTE_ENTRIES)
        );
        self.json(Method::GET, &path, None, 5).await
    }

    /// Un-adopted external entries, including the bounded event proposals Comms writes from mail.
    pub async fn calendar_proposals(
        &self,
        from: &str,
        to: &str,
    ) -> ShellResult<Vec<CalendarEntry>> {
        let path = with_query(
            &proxied(calendar_api::PROXY_PREFIX, calendar_api::ROUTE_PROPOSALS),
            &[("from", from), ("to", to)],
        );
        self.json(Method::GET, &path, None, 10).await
    }

    pub async fn calendar_google_exports(&self) -> ShellResult<Vec<CalendarGoogleExportOptIn>> {
        self.json(
            Method::GET,
            &proxied(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_GOOGLE_EXPORTS,
            ),
            None,
            10,
        )
        .await
    }

    /// Dry-run previews the approved queue; a real run is requested separately by the operator.
    pub async fn calendar_google_export(
        &self,
        dry_run: bool,
    ) -> ShellResult<CalendarGoogleExportReport> {
        self.json(
            Method::POST,
            &proxied(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_GOOGLE_EXPORT,
            ),
            Some(serde_json::json!({ "dry_run": dry_run })),
            60,
        )
        .await
    }

    pub async fn calendar_google_opt_in(&self, id: &str) -> ShellResult<CalendarGoogleExportOptIn> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::PUT,
            &proxied_id(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_GOOGLE_ENTRY_EXPORT,
                id,
            ),
            Some(serde_json::json!({})),
            10,
        )
        .await
    }

    pub async fn calendar_google_opt_out(&self, id: &str) -> ShellResult<()> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json::<CalendarGoogleExportOptOut>(
            Method::DELETE,
            &proxied_id(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_GOOGLE_ENTRY_EXPORT,
                id,
            ),
            None,
            10,
        )
        .await
        .and_then(|response| {
            if response.opted_out {
                Ok(())
            } else {
                Err(ShellError::Refused(
                    500,
                    Some("Google export opt-out was not confirmed.".into()),
                ))
            }
        })
    }

    /// A mail event is only accepted after the operator makes it a planned Calendar entry.
    pub async fn update_calendar_entry(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.json(
            Method::PATCH,
            &proxied_id(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_ENTRY_BY_ID,
                id,
            ),
            Some(body),
            10,
        )
        .await
    }

    pub async fn calendar_trip_drafts(
        &self,
        from: &str,
        to: &str,
    ) -> ShellResult<crate::calendar::TripDrafts> {
        let path = with_query(
            &proxied(calendar_api::PROXY_PREFIX, calendar_api::ROUTE_TRIP_DRAFTS),
            &[("from", from), ("to", to), ("max_gap_days", "5")],
        );
        self.json(Method::GET, &path, None, 10).await
    }

    /// Materialization is explicit and idempotent at Calendar's public endpoint.
    pub async fn calendar_materialize_trip(
        &self,
        entry_ids: &[String],
        title: &str,
    ) -> ShellResult<crate::calendar::TripMaterialization> {
        self.json(
            Method::POST,
            &proxied(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_TRIP_DRAFTS_MATERIALIZE,
            ),
            Some(body(&calendar_api::TripMaterializeRequest {
                entry_ids: entry_ids.to_vec(),
                title: (!title.trim().is_empty()).then(|| title.trim().to_owned()),
                plan_id: None,
            })?),
            30,
        )
        .await
    }

    /// Name-free nearby-companion count from Places for one trip destination and date window.
    pub async fn people_presence(
        &self,
        latitude: f64,
        longitude: f64,
        from: &str,
        to: &str,
    ) -> ShellResult<crate::travel::places::Presence> {
        let pairs = [
            ("latitude", latitude.to_string()),
            ("longitude", longitude.to_string()),
            ("from", from.to_owned()),
            ("to", to.to_owned()),
        ];
        let path = with_query(
            &proxied(
                places_api::PROXY_PREFIX,
                places_api::ROUTE_PLACES_PEOPLE_PRESENCE,
            ),
            &pairs
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect::<Vec<_>>(),
        );
        self.json(Method::GET, &path, None, 10).await
    }

    /// Creates an entry from a `core::calendar::Draft` body.
    pub async fn create_calendar_entry(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        self.json(
            Method::POST,
            &proxied(calendar_api::PROXY_PREFIX, calendar_api::ROUTE_ENTRIES),
            Some(body),
            10,
        )
        .await
    }

    pub async fn delete_calendar_entry(&self, id: &str) -> ShellResult<()> {
        // Ids are `<prefix>:<hex>` (`generated_id` in the calendar store).
        if !valid_calendar_entry_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a calendar entry id.".into()),
            ));
        }
        self.send(
            Method::DELETE,
            &proxied_id(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_ENTRY_BY_ID,
                id,
            ),
            None,
            10,
        )
        .await
        .map(drop)
    }

    /// Read people entities (`GET /entities/api/entities?kind=person`).
    pub async fn people(&self, query: Option<&str>) -> ShellResult<Vec<EntityRecord>> {
        let mut pairs = vec![("kind", "person")];
        if let Some(q) = query {
            if !q.is_empty() {
                pairs.push(("q", q));
            }
        }
        let path = with_query(
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_ENTITIES),
            &pairs,
        );
        #[derive(Deserialize)]
        struct Envelope {
            entities: Vec<EntityRecord>,
        }
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.entities)
    }

    /// The fields `kind` carries, built in and declared (`GET /entities/api/fields?kind=`).
    pub async fn entity_fields(&self, kind: &str) -> ShellResult<Vec<EntityField>> {
        #[derive(Deserialize)]
        struct Envelope {
            fields: Vec<EntityField>,
        }
        let path = with_query(
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_FIELDS),
            &[("kind", kind)],
        );
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.fields)
    }

    /// Declares a new field on a kind (`POST /entities/api/fields`). An existing key is refused.
    pub async fn entity_declare_field(&self, body: serde_json::Value) -> ShellResult<EntityField> {
        self.json(
            Method::POST,
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_FIELDS),
            Some(body),
            10,
        )
        .await
    }

    /// Creates an entity (`POST /entities/api/entities`). Every key in `values` must be a field
    /// the kind declares, which is why the form offers the registry's own list.
    pub async fn entity_create(&self, body: serde_json::Value) -> ShellResult<EntityRecord> {
        self.json(
            Method::POST,
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_ENTITIES),
            Some(body),
            10,
        )
        .await
    }

    /// Changes an entity (`PATCH /entities/api/entities/:id`). An absent key is left alone and a
    /// null clears a field; `expected_revision` turns a lost update into a 409.
    pub async fn entity_update(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<EntityRecord> {
        self.json(
            Method::PATCH,
            &proxied_id(
                entities_api::PROXY_PREFIX,
                entities_api::ROUTE_ENTITY_BY_ID,
                id,
            ),
            Some(body),
            10,
        )
        .await
    }

    /// Deletes an entity and its facts (`DELETE /entities/api/entities/:id`).
    pub async fn entity_delete(&self, id: &str) -> ShellResult<()> {
        self.send(
            Method::DELETE,
            &proxied_id(
                entities_api::PROXY_PREFIX,
                entities_api::ROUTE_ENTITY_BY_ID,
                id,
            ),
            None,
            10,
        )
        .await
        .map(drop)
    }

    /// Adds a dated fact (`POST /entities/api/entities/:id/facts`) and answers with the stored one,
    /// coordinate included when `places` resolved the place text.
    pub async fn entity_fact_add(
        &self,
        id: &str,
        body: serde_json::Value,
    ) -> ShellResult<EntityFact> {
        #[derive(Deserialize)]
        struct Added {
            fact: EntityFact,
        }
        let added: Added = self
            .json(
                Method::POST,
                &proxied_id(
                    entities_api::PROXY_PREFIX,
                    entities_api::ROUTE_ENTITY_FACTS,
                    id,
                ),
                Some(body),
                15,
            )
            .await?;
        Ok(added.fact)
    }

    /// Removes one dated fact (`DELETE /entities/api/entities/:id/facts/:fact`).
    pub async fn entity_fact_delete(&self, id: &str, fact: &str) -> ShellResult<()> {
        self.send(
            Method::DELETE,
            &proxied_id(
                entities_api::PROXY_PREFIX,
                entities_api::ROUTE_ENTITY_FACT_BY_ID,
                id,
            )
            .replace("{fact_id}", fact),
            None,
            10,
        )
        .await
        .map(drop)
    }

    /// Probable duplicate people, strongest first (`GET /entities/api/duplicates`). Each pair
    /// carries the registry's default merge: which side to keep and the merged name.
    pub async fn entity_duplicates(&self, limit: u32) -> ShellResult<Vec<DuplicatePair>> {
        #[derive(Deserialize)]
        struct Envelope {
            candidates: Vec<DuplicatePair>,
        }
        let path = with_query(
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_DUPLICATES),
            &[("limit", &limit.to_string())],
        );
        let res: Envelope = self.json(Method::GET, &path, None, 15).await?;
        Ok(res.candidates)
    }

    /// Merges `other` into `keep` under `name` (`POST /entities/api/entities/:keep/merge`).
    pub async fn entity_merge(&self, keep: &str, other: &str, name: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "other": other, "name": name, "pick": [] });
        self.send(
            Method::POST,
            &proxied_id(entities_api::PROXY_PREFIX, entities_api::ROUTE_MERGE, keep),
            Some(body),
            15,
        )
        .await
        .map(drop)
    }

    /// Marks two people as different (`POST /entities/api/duplicates/distinct`).
    pub async fn entity_mark_distinct(&self, a: &str, b: &str) -> ShellResult<()> {
        let body = serde_json::json!({ "a": a, "b": b });
        self.send(
            Method::POST,
            &proxied(
                entities_api::PROXY_PREFIX,
                entities_api::ROUTE_DUPLICATES_DISTINCT,
            ),
            Some(body),
            10,
        )
        .await
        .map(drop)
    }

    /// Where each person is on `day` (`GET /entities/api/located?day=`). C2: names.
    pub async fn people_located(
        &self,
        day: &str,
    ) -> ShellResult<Vec<crate::travel::around::Located>> {
        #[derive(Deserialize)]
        struct Envelope {
            located: Vec<crate::travel::around::Located>,
        }
        let path = with_query(
            &proxied(entities_api::PROXY_PREFIX, entities_api::ROUTE_LOCATED),
            &[("day", day)],
        );
        let res: Envelope = self.json(Method::GET, &path, None, 10).await?;
        Ok(res.located)
    }

    /// Idempotently promotes a Scouting result into the Calendar capability.
    pub async fn calendar_upsert_external(
        &self,
        body: serde_json::Value,
    ) -> ShellResult<CalendarEntry> {
        self.json(
            Method::PUT,
            &proxied(
                calendar_api::PROXY_PREFIX,
                calendar_api::ROUTE_ENTRY_EXTERNAL,
            ),
            Some(body),
            10,
        )
        .await
    }

    /// Calendar-owned feasibility judgments for dated Scouting opportunities.
    pub async fn calendar_candidate_verdicts(
        &self,
        candidates: &[crate::scouting::CalendarCandidate],
    ) -> ShellResult<crate::scouting::CalendarVerdicts> {
        self.json(
            Method::POST,
            &proxied(calendar_api::PROXY_PREFIX, calendar_api::ROUTE_VERDICTS),
            Some(serde_json::json!({ "candidates": candidates })),
            10,
        )
        .await
    }
}
