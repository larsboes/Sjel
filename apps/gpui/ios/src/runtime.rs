//! Offline synchronization and notification rebuilding without a rendering executor.
use super::*;
impl AppState {
    pub(crate) fn maybe_sync(&mut self) {
        if self.syncing {
            return;
        }
        let (Some(client), Some(board)) = (self.client.clone(), self.board.clone()) else {
            return;
        };
        self.syncing = true;
        self.synced = Some("Syncing with the Mac…".into());
        let events = self.events.clone();
        shell::spawn(async move {
            let Some(store) = board.store().cloned() else {
                return;
            };
            let outcome = sync::run(&store, &client).await;
            let mut board = board;
            let _ = board.reload().await;
            events.post(Event::Synced(outcome, board));
        });
    }

    /// Rebuilds the lock screen from what this phone holds: the calendar's next weeks and the open
    /// tasks, through the same store their pages read.
    ///
    /// Called when the replica opens, after a write, after a pull and when the app comes back into
    /// view. A phone with reminders off takes every request away here rather than leaving stale ones
    /// to fire at a time nobody asked about.
    pub(crate) fn rebuild_reminders(&mut self) {
        if notify::disabled_reason().is_some() {
            // This build may not touch the notification centre at all, so the card says why and
            // nothing is scheduled. The preference is left as the person set it: the switch is
            // about their intent, not this launch's build.
            self.notify_plan = reminders::Plan::default();
            self.notify_state = notify::State::default();
            notify::clear();

            return;
        }
        let reminders_enabled = self.notify_prefs.enabled;
        if !reminders_enabled {
            notify::clear();
            self.notify_plan = reminders::Plan::default();

            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let lead = self.notify_prefs.lead();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let check_fare_alerts = self.notify_prefs.begin_fare_alert_check(now_ms);
        if check_fare_alerts {
            let _ = self.notify_prefs.save();
        }
        let seen: std::collections::HashSet<String> = self
            .notify_prefs
            .seen_fare_alert_ids
            .iter()
            .cloned()
            .collect();
        let authorization = self.notify_state.authorization;
        let source = self
            .client
            .clone()
            .map(sjel_gpui_core::travel::Source::Device);
        let events = self.events.clone();
        shell::spawn(async move {
            let today = chrono::Local::now().date_naive();
            let from = format!("{today}T00:00:00");
            let to = format!("{}T00:00:00", today + chrono::Duration::days(31));
            let entries = if reminders_enabled {
                repo.fetch_calendar(&from, &to).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let tasks = if reminders_enabled {
                repo.fetch_tasks(Some("open")).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            let plan = reminders::plan(&entries, &tasks, lead, chrono::Local::now());
            if reminders_enabled {
                notify::apply(&plan);
            }

            if check_fare_alerts {
                let today = today.format("%Y-%m-%d").to_string();
                let saved = sjel_gpui_core::travel::saved_plans().await.ok().flatten();
                let plans = if let Some(source) = source.as_ref() {
                    match source.plans().await {
                        Ok(plans) => {
                            let _ = sjel_gpui_core::travel::save_plans(&plans).await;
                            plans
                        }
                        Err(_) => saved.map(|snapshot| snapshot.plans).unwrap_or_default(),
                    }
                } else {
                    saved.map(|snapshot| snapshot.plans).unwrap_or_default()
                };
                let mut details = Vec::new();
                for plan in plans
                    .iter()
                    .filter(|plan| sjel_gpui_core::travel::upcoming(plan, &today))
                {
                    let fresh = if let Some(source) = source.as_ref() {
                        source.details(&plan.id).await.ok()
                    } else {
                        None
                    };
                    if let Some(fresh) = fresh {
                        let _ = sjel_gpui_core::travel::save_details(fresh.clone()).await;
                        details.push(fresh);
                    } else if let Ok(Some(saved)) =
                        sjel_gpui_core::travel::saved_details(&plan.id).await
                    {
                        details.push(saved.details);
                    }
                }
                let alerts = sjel_gpui_core::travel::unseen_fare_lows(&details, &seen, &today);
                if !alerts.is_empty() {
                    match authorization {
                        notify::Authorization::Allowed => {
                            notify::deliver_fare_alerts(&alerts);
                            let ids = alerts.into_iter().map(|alert| alert.id).collect();
                            events.post(Event::FareAlertsSeen(ids));
                        }
                        notify::Authorization::NotAsked | notify::Authorization::Denied => {}
                    }
                }
            }
            events.post(Event::Reminded(plan));
        });
    }

    /// The switch on the Settings page. On: the permission is asked for (once in the app's life) and
    /// the requests are laid down — iOS accepts them before they may be shown, so a person who grants
    /// the prompt a moment later still gets the reminders already scheduled. Off: every request goes.
    pub(crate) fn toggle_reminders(&mut self) {
        self.notify_prefs.enabled = !self.notify_prefs.enabled;
        let _ = self.notify_prefs.save();
        if self.notify_prefs.enabled {
            notify::authorize();
            self.rebuild_reminders();
        } else {
            notify::clear();
            self.notify_plan = reminders::Plan::default();
        }
    }

    /// Fetches the Mac's newest copy for the Calendar page.
    ///
    /// Called when the pairing goes live and when the person taps Refresh. A launch that never
    /// reaches the Mac keeps the copy it already has: [`open_copy`] reads that one at startup,
    /// and a failed fetch reports itself beside rows that are still readable.
    pub(crate) fn refresh_copy(&mut self) {
        if self.copying {
            return;
        }
        let Some(client) = self.client.clone() else {
            // A refresh needs the Mac. Saying so is better than a button that does nothing.
            nsh_calendar::set_note(Some(
                "Pair with your Mac in Settings to fetch a copy.".to_owned(),
            ));
            return;
        };
        self.copying = true;
        let events = self.events.clone();
        shell::spawn(async move {
            events.post(Event::Copy(fetch_copy(client).await));
        });
    }

    pub(crate) fn repo_for(&self, store: Arc<sjel_gpui_core::store::TursoStore>) -> Repo {
        let events = self.events.clone();
        Repo::replica(store, move || {
            events.post(Event::Wrote);
        })
    }
}
