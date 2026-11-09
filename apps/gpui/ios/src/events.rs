//! What each [`Event`] does to the view.

use super::*;

/// The Mac's answer to a RoomPlan upload.
#[derive(Deserialize)]
struct RoomplanUploadAck {
    draft_id: String,
}

impl Capabilities {
    /// One event from the host, the network or a page, applied to the view.
    pub(super) fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Status(text) => self.status = text.into(),
            Event::StoreFailed(text) => self.store_error = Some(text.into()),
            Event::Copy(outcome) => {
                self.copying = false;
                match outcome {
                    Ok(copy) => {
                        self.copy = Some(copy.clone());
                        nsh_calendar::set_copy(Some(copy.clone()), None);
                        sjel_gpui_ui::nsh_people::set_copy(Some(copy.clone()), None);
                        nsh_today::set_copy(Some(copy), None);
                    }
                    // A refresh that fails leaves the copy already on screen alone:
                    // it is still true, only older, and the page says how old.
                    Err(error) => {
                        if self.copy.is_none() {
                            nsh_calendar::set_copy(None, Some(error.clone()));
                            sjel_gpui_ui::nsh_people::set_copy(None, Some(error.clone()));
                            nsh_today::set_copy(None, Some(error));
                        } else {
                            let note = format!("Could not fetch a newer copy: {error}");
                            nsh_calendar::set_note(Some(note.clone()));
                            sjel_gpui_ui::nsh_people::set_note(Some(note.clone()));
                            nsh_today::set_note(Some(note));
                        }
                    }
                }
            }
            Event::Board(board) => {
                if self.projects.is_none() {
                    if let Some(store) = board.store().cloned() {
                        let projects = self.open_projects(store.clone(), cx);
                        self.projects = Some(projects);
                        // The Calendar tab writes to the same store, and reads it as
                        // soon as its change feed has been read once — until then the
                        // Mac's copy keeps drawing the day (`ui/src/calendar.rs`).
                        sjel_gpui_ui::nsh_capture::set_store(Some(store.clone()));
                        let repo = self.repo_for(store);
                        nsh_today::set_store(Some(repo.clone()));
                        nsh_calendar::set_store(Some(repo.clone()));
                        // The reminder rebuild reads the same store: an entry written on
                        // the phone is a reminder on the same phone, without the Mac.
                        self.repo = Some(repo);
                    }
                }
                // The landing page draws the replica's tasks and projects from these
                // rows: `Board` already read them, and a second read here could only
                // disagree with the queue count beside them.
                let records = board.records().to_vec();
                let blocked = board.blocked().to_vec();
                if let Some(projects) = &self.projects {
                    projects.update(cx, |projects, cx| projects.set_blocked(blocked, cx));
                }
                sjel_gpui_ui::nsh_capture::load();
                nsh_today::set_records(records);
                nsh_today::set_travel(board.pending(), self.client.is_some());
                self.board = Some(board);
                // The replica moved: the lock screen follows it, whether or not the Mac
                // answered (`rebuild_reminders`).
                self.rebuild_reminders(cx);
                // A local write landed. If this launch has reached the Mac, it goes
                // there now instead of waiting for the next launch.
                self.maybe_sync(cx);
            }
            Event::Found(nodes) => {
                self.found = nodes;
                // A previous answer is not an error; keep it on screen.
                if self.rows.is_empty() && !self.joining {
                    self.status = if self.found.is_empty() {
                        "Looking for your Mac on this network…".into()
                    } else {
                        "Found your Mac. Tap Join to ask for access.".into()
                    };
                }
            }
            Event::Join(found) => {
                self.joining = true;
                self.status = format!(
                    "Asking \u{201c}{}\u{201d} to allow this device…",
                    found.name
                )
                .into();
                let events = self.events.clone();
                shell::spawn(async move {
                    let outcome = match node_file() {
                        Ok(path) => match join_and_wait(&found.join_target(), &events).await {
                            Ok(node) => connect(&path, node, &events).await,
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    };
                    if let Err(error) = outcome {
                        let _ = events.unbounded_send(Event::PairFailed(error));
                    }
                });
            }
            Event::Scanned(code) => {
                // A scanned code makes the same claim the `SJEL_PAIR` launch path makes;
                // it arrives after launch, so the whole claim runs here instead.
                self.joining = true;
                self.status = "Reading the code from your Mac…".into();
                let events = self.events.clone();
                shell::spawn(async move {
                    let outcome = match node_file() {
                        Ok(path) => match claim_scanned(&code).await {
                            Ok(node) => connect(&path, node, &events).await,
                            Err(error) => Err(error),
                        },
                        Err(error) => Err(error),
                    };
                    if let Err(error) = outcome {
                        let _ = events.unbounded_send(Event::PairFailed(error));
                    }
                });
            }
            Event::PairFailed(text) | Event::ScanFailed(text) => {
                self.status = text.into();
                self.joining = false;
            }
            Event::RoomplanResult(text) => {
                self.roomplan_status = text.into();
                if self.client.is_some() {
                    self.interior.update(cx, |history, cx| history.refresh(cx));
                }
            }
            Event::RoomplanArchive(records) => self
                .interior
                .update(cx, |history, cx| history.set_local_archive(records, cx)),
            Event::RoomplanLive(snapshot) => self
                .interior
                .update(cx, |history, cx| history.set_live_snapshot(snapshot, cx)),
            Event::RoomplanUpload(body) => {
                if let Some(client) = self.client.clone() {
                    self.roomplan_status = "Sending the room scan to your Mac…".into();
                    let events = self.events.clone();
                    shell::spawn(async move {
                        let body = body.into_bytes();
                        let expected_id = serde_json::from_slice::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|value| {
                                value
                                    .get("draft")?
                                    .get("draft_id")?
                                    .as_str()
                                    .map(str::to_owned)
                            });
                        let status = match client
                            .post::<RoomplanUploadAck>("/interior/api/roomplan/captures", &body)
                            .await
                        {
                            Ok(ack) if expected_id.as_deref() == Some(&ack.draft_id) => {
                                mark_roomplan_uploaded(&ack.draft_id);
                                format!("Sent to Mac · {}", ack.draft_id)
                            }
                            Ok(_) => "Mac returned a mismatched scan receipt; scan remains queued."
                                .to_owned(),
                            Err(error) => format!("Could not send room scan: {error}"),
                        };
                        let _ = events.unbounded_send(Event::RoomplanResult(status));
                    });
                } else {
                    self.roomplan_status = "Pair with your Mac before sending a scan.".into();
                }
            }
            Event::Ready(node, health) => {
                self.status = format!("Paired as {} with {}", node.device_id, node.base).into();
                self.rows = health
                    .capabilities
                    .iter()
                    .map(|(name, state)| (name.clone(), state.up))
                    .collect();
                self.found.clear();
                self.joining = false;
                match DeviceClient::new(node, Arc::new(identity::sign)) {
                    Ok(client) => {
                        let client = Arc::new(client);
                        self.interior.update(cx, |history, cx| {
                            history.set_source(Some(RoomPlanSource::Device(client.clone())), cx)
                        });
                        self.travel.update(cx, |travel, _| {
                            travel
                                .set_source(sjel_gpui_core::travel::Source::Device(client.clone()))
                        });
                        self.agent.update(cx, |seat, cx| {
                            seat.set_host(Some(Host::Device(client.clone())), cx);
                            if self.sidebar.is_open() && self.sidebar.seat() == Seat::Agent {
                                seat.refresh_sessions(cx);
                            }
                        });
                        sjel_gpui_ui::nsh_documents::connect(
                            sjel_gpui_core::device::Source::Device(client.clone()),
                        );
                        sjel_gpui_ui::nsh_mail::connect(sjel_gpui_core::device::Source::Device(
                            client.clone(),
                        ));
                        self.client = Some(client);
                    }
                    Err(error) => self.synced = Some(format!("Cannot sync: {error}").into()),
                }
                self.maybe_sync(cx);
                if self.client.is_some() {
                    load_roomplan_upload();
                }
                self.refresh_copy(cx);
                let paired = self.client.is_some();
                let pending = self.board.as_ref().map_or(0, Board::pending);
                nsh_today::set_travel(pending, paired);
            }
            Event::Wrote => {
                if let Some(board) = self.board.clone() {
                    let events = self.events.clone();
                    shell::spawn(async move {
                        let mut board = board;
                        let _ = board.reload().await;
                        let _ = events.unbounded_send(Event::Board(board));
                    });
                }
            }
            Event::Synced(outcome, board) => {
                self.syncing = false;
                self.board = Some(board);
                // The pull may have changed any record the page shows.
                if let Some(projects) = &self.projects {
                    projects.update(cx, |projects, cx| projects.reload(cx));
                }
                // A pull lands entries too, and the ids a create answered with.
                nsh_calendar::load();
                // A sync is what moves a capture from "waiting" to "on the Mac".
                sjel_gpui_ui::nsh_capture::load();
                let (pending, paired) = (
                    self.board.as_ref().map_or(0, Board::pending),
                    self.client.is_some(),
                );
                let records = self
                    .board
                    .as_ref()
                    .map(|board| board.records().to_vec())
                    .unwrap_or_default();
                // A sync is also how a refusal arrives, or stops being one.
                let blocked = self
                    .board
                    .as_ref()
                    .map(|board| board.blocked().to_vec())
                    .unwrap_or_default();
                if let Some(projects) = &self.projects {
                    projects.update(cx, |projects, cx| projects.set_blocked(blocked, cx));
                }
                sjel_gpui_ui::nsh_capture::load();
                nsh_today::set_records(records);
                nsh_today::set_travel(pending, paired);
                self.synced = Some(
                    match outcome {
                        Ok(report) => report.summary(),
                        Err(error) => format!("Nothing moved: {error}"),
                    }
                    .into(),
                );
                // A pull lands the ids a local entry was given, and the tasks the Mac
                // ticked off: both change what earns a reminder.
                self.rebuild_reminders(cx);
            }
            Event::Reminded(plan) => {
                self.notify_plan = plan;
            }
            Event::FareAlertsSeen(ids) => {
                self.notify_prefs.mark_fare_alerts_seen(&ids);
                let _ = self.notify_prefs.save();
            }
            Event::NotifyState(state) => {
                let newly_allowed = self.notify_state.authorization
                    != notify::Authorization::Allowed
                    && state.authorization == notify::Authorization::Allowed;
                self.notify_state = state;
                if newly_allowed {
                    self.notify_prefs.fare_alerts_last_check_ms = 0;
                    let _ = self.notify_prefs.save();
                    self.rebuild_reminders(cx);
                }
            }
            Event::NotifyOpen(link) => self.open_link(&link, cx),
            Event::Pinch(scale, ended) => {
                if self.sidebar.is_open() && self.sidebar.seat() == Seat::Terminal {
                    let steps = pinch_steps(&mut self.pinch_base, scale);
                    if steps != 0 {
                        self.zoom_terminal(steps, cx);
                    }
                } else if self.active_tab == Tab::Documents {
                    let steps = pinch_steps(&mut self.pinch_base, scale);
                    if steps != 0 {
                        self.documents.update(cx, |docs, cx| docs.toggle_zoom(cx));
                    }
                }
                if ended {
                    self.pinch_base = 1.0;
                }
            }
            Event::Background => {
                self.terminal_unlocked = false;
                self.terminal = None;
            }
            Event::Unlocked(ok) => {
                self.terminal_unlocked = ok;
                if !ok {
                    self.terminal_note = Some(
                        "Face ID did not confirm. Close and reopen the edge to try again.".into(),
                    );
                } else if self.sidebar.is_open() && self.sidebar.seat() == Seat::Terminal {
                    self.open_terminal(cx);
                }
            }
            Event::Foreground => {
                load_roomplan_archive();
                self.rebuild_reminders(cx);
                if self.client.is_some() {
                    load_roomplan_upload();
                }
                // The count is the system's, so it is asked for rather than assumed:
                // a reminder that fired while the phone was away is gone from it.
                notify::report();
            }
            Event::SelectTab(tab) => self.select_tab(tab, cx),
            Event::OpenEdge(seat) => self.open_edge(seat, cx),
            Event::Today(event) => self.on_today(event, cx),
            Event::RefreshCopy => self.refresh_copy(cx),
            Event::Settings(action) => self.on_settings(action, cx),
            // Newer data: a sync when this phone has a store to sync, the Mac's copy otherwise.
            Event::Calendar(CalendarEvent::Refresh) => {
                if self.board.is_some() {
                    self.maybe_sync(cx);
                } else {
                    self.refresh_copy(cx);
                }
            }
            // An entry landed in the replica: the queue moved, so the Mac can have it now.
            Event::Calendar(CalendarEvent::Wrote) => self.maybe_sync(cx),
        }
    }
}
