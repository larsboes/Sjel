//! Native and capability events applied to the phone's rendering-independent state.
use super::*;
#[derive(Deserialize)]
struct RoomplanUploadAck {
    draft_id: String,
}
impl AppState {
    fn board_changed(&mut self, board: Board) {
        if self.repo.is_none() {
            if let Some(store) = board.store().cloned() {
                let repo = self.repo_for(store.clone());
                sjel_gpui_ui::phone_capture::set_store(Some(store));
                phone_today::set_store(Some(repo.clone()));
                phone_calendar::set_store(Some(repo.clone()));
                #[cfg(not(target_os = "ios"))]
                sjel_gpui_ui::nsh_projects::set_repo(repo.clone());
                self.repo = Some(repo);
            }
        }
        #[cfg(target_os = "ios")]
        crate::projects::set_board(self.repo.clone(), board.blocked().to_vec());
        #[cfg(not(target_os = "ios"))]
        {
            sjel_gpui_ui::nsh_projects::set_blocked(board.blocked().to_vec());
            sjel_gpui_ui::nsh_projects::load();
        }
        sjel_gpui_ui::phone_capture::load();
        phone_today::set_records(board.records().to_vec());
        phone_today::set_travel(board.pending(), self.client.is_some());
        self.board = Some(board);
        self.rebuild_reminders();
    }
    pub(crate) fn handle(&mut self, event: Event) {
        match event {
            Event::Status(text) => self.status = text,
            Event::StoreFailed(text) => {
                self.status = text.clone();
                phone_today::set_note(Some(text));
            }
            Event::Copy(outcome) => {
                self.copying = false;
                match outcome {
                    Ok(copy) => {
                        self.copy = Some(copy.clone());
                        phone_calendar::set_copy(Some(copy.clone()), None);
                        sjel_gpui_ui::phone_people::set_copy(Some(copy.clone()), None);
                        phone_today::set_copy(Some(copy), None);
                    }
                    Err(error) if self.copy.is_none() => {
                        phone_calendar::set_copy(None, Some(error.clone()));
                        sjel_gpui_ui::phone_people::set_copy(None, Some(error.clone()));
                        phone_today::set_copy(None, Some(error));
                    }
                    Err(error) => {
                        let note = Some(format!("Could not fetch a newer copy: {error}"));
                        phone_calendar::set_note(note.clone());
                        sjel_gpui_ui::phone_people::set_note(note.clone());
                        phone_today::set_note(note);
                    }
                }
            }
            Event::Board(board) => {
                self.board_changed(board);
                self.maybe_sync();
            }
            Event::Found(nodes) => {
                self.found = nodes;
                if self.rows.is_empty() && !self.joining {
                    self.status = if self.found.is_empty() {
                        "Looking for your Mac on this network…"
                    } else {
                        "Found your Mac. Tap Join to ask for access."
                    }
                    .into();
                }
            }
            Event::Join(found) => {
                if self.joining {
                    return;
                }
                self.joining = true;
                self.status = format!("Asking {} to allow this device…", found.name);
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
                        events.post(Event::PairFailed(error));
                    }
                });
            }
            Event::Scanned(code) => {
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
                        events.post(Event::PairFailed(error));
                    }
                });
            }
            Event::PairFailed(error) | Event::ScanFailed(error) => {
                self.status = error;
                self.joining = false;
            }
            Event::RoomplanResult(text) => {
                self.roomplan_status = text;
                sjel_gpui_ui::nsh_interior::load();
                load_roomplan_archive();
            }
            Event::RoomplanArchive(records) => sjel_gpui_ui::nsh_interior::set_local(records),
            Event::RoomplanLive(live) => sjel_gpui_ui::nsh_interior::set_live(live),
            Event::HealthState(state) => self.health = state,
            Event::HealthUpload(body) => {
                let Some(client) = self.client.clone() else {
                    health::finished(Err("Connect to your paired Mac and try again.".into()));
                    return;
                };
                shell::spawn(async move {
                    let result = client
                        .post::<health::Ack>(&health::route(), body.as_bytes())
                        .await
                        .map_err(|_| {
                            "Could not sync Health to your Mac. Check the connection and try again."
                                .to_owned()
                        })
                        .and_then(|ack| ack.confirm(health::through(&body)));
                    health::finished(result);
                });
            }
            Event::RoomplanUpload(body) => {
                let Some(client) = self.client.clone() else {
                    self.roomplan_status = "Pair with your Mac before sending a scan.".into();
                    return;
                };
                self.roomplan_status = "Sending the room scan to your Mac…".into();
                let events = self.events.clone();
                shell::spawn(async move {
                    let body = body.into_bytes();
                    let expected = serde_json::from_slice::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("draft")?.get("draft_id")?.as_str().map(str::to_owned));
                    let status = match client
                        .post::<RoomplanUploadAck>("/interior/api/roomplan/captures", &body)
                        .await
                    {
                        Ok(ack) if expected.as_deref() == Some(&ack.draft_id) => {
                            mark_roomplan_uploaded(&ack.draft_id);
                            format!("Sent to Mac · {}", ack.draft_id)
                        }
                        Ok(_) => "Mac returned a mismatched scan receipt; the scan remains queued."
                            .into(),
                        Err(error) => format!("Could not send room scan: {error}"),
                    };
                    events.post(Event::RoomplanResult(status));
                });
            }
            Event::Ready(node, health) => {
                self.status = format!("Paired as {} with {}", node.device_id, node.base);
                self.rows = health
                    .capabilities
                    .iter()
                    .map(|(name, status)| (name.clone(), status.up))
                    .collect();
                self.found.clear();
                self.joining = false;
                match DeviceClient::new(node, Arc::new(identity::sign)) {
                    Ok(client) => {
                        let client = Arc::new(client);
                        sjel_gpui_ui::nsh_travel::connect(sjel_gpui_core::travel::Source::Device(
                            client.clone(),
                        ));
                        sjel_gpui_ui::nsh_interior::connect(RoomPlanSource::Device(client.clone()));
                        sjel_gpui_ui::nsh_agent::connect(Host::Device(client.clone()));
                        sjel_gpui_ui::nsh_terminal::connect(Host::Device(client.clone()));
                        #[cfg(target_os = "ios")]
                        crate::documents::connect(client.clone());
                        sjel_gpui_ui::phone_mail::connect(sjel_gpui_core::device::Source::Device(
                            client.clone(),
                        ));
                        sjel_gpui_ui::phone_finance::connect(
                            sjel_gpui_core::device::Source::Device(client.clone()),
                        );
                        sjel_gpui_ui::phone_inventory::connect(
                            sjel_gpui_core::device::Source::Device(client.clone()),
                        );
                        self.client = Some(client);
                    }
                    Err(error) => self.synced = Some(format!("Cannot sync: {error}")),
                }
                self.maybe_sync();
                self.refresh_copy();
                if self.client.is_some() {
                    load_roomplan_upload();
                    // At launch the foreground event fires before the pairing is restored.
                    health::collect(false);
                }
                phone_today::set_travel(
                    self.board.as_ref().map_or(0, Board::pending),
                    self.client.is_some(),
                );
            }
            Event::Wrote => {
                if let Some(board) = self.board.clone() {
                    let events = self.events.clone();
                    shell::spawn(async move {
                        let mut board = board;
                        match board.reload().await {
                            Ok(()) => events.post(Event::Board(board)),
                            Err(e) => events.post(Event::Status(format!(
                                "The saved record could not be reloaded: {e}"
                            ))),
                        }
                    });
                }
            }
            Event::Synced(outcome, board) => {
                self.syncing = false;
                self.board_changed(board);
                phone_calendar::load();
                self.synced = Some(match outcome {
                    Ok(report) => report.summary(),
                    Err(error) => format!("Nothing moved: {error}"),
                });
            }
            Event::Retry => {
                if self.client.is_some() {
                    self.maybe_sync();
                } else {
                    self.synced =
                        Some("Not connected: pair with the Mac in Settings, then retry.".into());
                }
            }
            Event::Reminded(plan) => self.notify_plan = plan,
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
                    self.rebuild_reminders();
                }
            }
            Event::NotifyOpen(link) => self.open_link(&link),
            Event::Background => {
                self.terminal_unlocked = false;
                sjel_gpui_ui::nsh_terminal::set_unlocked(false);
            }
            Event::Unlocked(ok) => {
                self.terminal_unlocked = ok;
                sjel_gpui_ui::nsh_terminal::set_unlocked(ok);
            }
            Event::Foreground => {
                load_roomplan_archive();
                self.rebuild_reminders();
                if self.client.is_some() {
                    load_roomplan_upload();
                    health::collect(false);
                }
                notify::report();
            }
            Event::Pinch(scale, _location, ended) => {
                let relative = scale / self.pinch_base;
                #[cfg(target_os = "ios")]
                if current_page() == PhonePage::Map {
                    if relative.is_finite()
                        && relative > 0.
                        && (relative >= 1.12 || relative <= 1. / 1.12 || ended)
                    {
                        sjel_gpui_ui::nsh_map::pinch(relative.log2(), _location);
                        self.pinch_base = scale;
                    }
                }
                if current_page() == PhonePage::Edge && nsh_edge::is_terminal() {
                    if relative >= 1.2 {
                        self.handle(Event::Zoom(1));
                        self.pinch_base = scale;
                    } else if relative <= 1. / 1.2 {
                        self.handle(Event::Zoom(-1));
                        self.pinch_base = scale;
                    }
                }
                if ended {
                    self.pinch_base = 1.;
                }
            }
            Event::Zoom(steps) => {
                self.appearance.terminal_font_size = sjel_gpui_ui::step_terminal_font_size(
                    self.appearance.terminal_font_size,
                    steps,
                );
                let _ = self.appearance.save();
            }
            Event::SelectTab(tab) => self.select_tab(tab),
            Event::OpenEdge(seat) => {
                if seat == Seat::Soundscape && !self.audio_started {
                    self.audio_started = true;
                    sjel_gpui_ui::nsh_soundscape::set_device(audio::start());
                }
                if seat == Seat::Terminal {
                    if self.terminal_unlocked {
                        sjel_gpui_ui::nsh_terminal::refresh();
                    } else if self.client.is_some() {
                        unlock();
                    }
                }
                if seat == Seat::Activity {
                    notify::report();
                }
            }
            Event::Today(event) => self.on_today(event),
            Event::RefreshCopy => self.refresh_copy(),
            Event::Calendar(CalendarEvent::Refresh) => {
                if self.board.is_some() {
                    self.maybe_sync();
                } else {
                    self.refresh_copy();
                }
            }
            Event::Calendar(CalendarEvent::Wrote) => self.maybe_sync(),
            Event::Map(event) => sjel_gpui_ui::nsh_travel::map_event(event),
            Event::Settings(action) => self.on_settings(action),
            Event::Interior(event) => {
                use sjel_gpui_ui::nsh_interior::InteriorEvent;
                match event {
                    InteriorEvent::Capture => present_roomplan(),
                    InteriorEvent::Sync => load_roomplan_upload(),
                    InteriorEvent::Import => {
                        #[cfg(target_os = "ios")]
                        import_legacy_roomplan();
                    }
                    InteriorEvent::Preview(id) => {
                        #[cfg(target_os = "ios")]
                        preview_roomplan(&id);
                        #[cfg(not(target_os = "ios"))]
                        let _ = id;
                    }
                    InteriorEvent::CachePreview { id, draft, bytes } => {
                        #[cfg(target_os = "ios")]
                        cache_preview_roomplan(&id, &draft, bytes);
                        #[cfg(not(target_os = "ios"))]
                        let _ = (id, draft, bytes);
                    }
                }
            }
        }
    }
}
