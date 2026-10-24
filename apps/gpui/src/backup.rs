//! Deep backup inspection, capability contracts, target cadence policy, rehearsal verification,
//! and durable run attempt history.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, rgb, rgba, AnyElement, App, Context, FocusHandle, KeyBinding, Render,
    Window, WindowAppearance,
};

use crate::data::{Remote, Repo};
use crate::shell::{self, BackupStatus, Backups, ShellError};
use crate::theme::{tint, Palette};
use crate::ui::dot;
use sjel_gpui_core::backup::{
    format_age_seconds, format_bytes, format_duration, run_outcome, BackupRunAttemptRecord,
    BackupTargetRecord, VerifyTargetResult,
};

/// The `Busy` key the button marks while its requests are in flight.
pub const BACK_UP_ALL: &str = "backup:all";

actions!(
    backup,
    [ShowContracts, ShowTargets, ShowHistory, RefreshBackup,]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ShowContracts, Some("BackupView")),
        KeyBinding::new("2", ShowTargets, Some("BackupView")),
        KeyBinding::new("3", ShowHistory, Some("BackupView")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackupTab {
    #[default]
    Contracts,
    Targets,
    History,
}

pub struct BackupPage {
    repo: Repo,
    palette: Option<Palette>,
    pub tab: BackupTab,
    pub backups: Remote<Backups>,
    pub targets: Remote<Vec<BackupTargetRecord>>,
    pub runs: Remote<(Vec<BackupRunAttemptRecord>, usize)>,
    pub verifying: HashMap<String, bool>,
    pub verify_verdicts: HashMap<String, VerifyTargetResult>,
    pub confirming_cold_copy: Option<String>,
    pub selected_target_override: HashMap<String, String>,
    pub action_in_flight: HashMap<String, bool>,
    focus: FocusHandle,
}

impl BackupPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            tab: BackupTab::Contracts,
            backups: Remote::default(),
            targets: Remote::default(),
            runs: Remote::default(),
            verifying: HashMap::new(),
            verify_verdicts: HashMap::new(),
            confirming_cold_copy: None,
            selected_target_override: HashMap::new(),
            action_in_flight: HashMap::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn set_tab(&mut self, tab: BackupTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };

        self.backups.loading = true;
        self.targets.loading = true;
        self.runs.loading = true;
        cx.notify();

        // 1. Fetch Backups
        let s_b = shell.clone();
        let b_work = shell::run(async move { s_b.backups().await });
        cx.spawn(async move |this, cx| {
            let res = b_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.backups.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 2. Fetch Targets
        let s_t = shell.clone();
        let t_work = shell::run(async move { s_t.backup_targets().await });
        cx.spawn(async move |this, cx| {
            let res = t_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.targets.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 3. Fetch Runs Ledger
        let s_r = shell.clone();
        let r_work = shell::run(async move { s_r.backup_runs(50).await });
        cx.spawn(async move |this, cx| {
            let res = r_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.runs.settle(res);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn verify_target(&mut self, target_id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.verifying.insert(target_id.clone(), true);
        cx.notify();

        let tid = target_id.clone();
        let work = shell::run(async move { shell.verify_backup_target(&tid).await });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.verifying.remove(&target_id);
                if let Ok(verdict) = res {
                    page.verify_verdicts.insert(target_id, verdict);
                }
                page.load(cx);
            });
        })
        .detach();
    }

    pub fn set_target_policy(
        &mut self,
        target_id: String,
        interval: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.action_in_flight
            .insert(format!("policy:{}", target_id), true);
        cx.notify();

        let tid = target_id.clone();
        let work = shell::run(async move { shell.set_backup_policy(&tid, interval).await });
        cx.spawn(async move |this, cx| {
            let _ = work.await;
            let _ = this.update(cx, |page, cx| {
                page.action_in_flight
                    .remove(&format!("policy:{}", target_id));
                page.load(cx);
            });
        })
        .detach();
    }

    pub fn trigger_backup(
        &mut self,
        capability: String,
        target: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.confirming_cold_copy = None;
        self.action_in_flight
            .insert(format!("backup:{}", capability), true);
        cx.notify();

        let cap = capability.clone();
        let work =
            shell::run(async move { shell.backup_capability(&cap, target.as_deref()).await });
        cx.spawn(async move |this, cx| {
            let _ = work.await;
            let _ = this.update(cx, |page, cx| {
                page.action_in_flight
                    .remove(&format!("backup:{}", capability));
                page.load(cx);
            });
        })
        .detach();
    }

    pub fn trigger_backup_all(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let caps: Vec<String> = self
            .backups
            .value
            .as_ref()
            .map(|b| b.backups.iter().map(|s| s.capability.clone()).collect())
            .unwrap_or_default();

        self.action_in_flight.insert(BACK_UP_ALL.to_string(), true);
        cx.notify();

        let work = shell::run(async move {
            for cap in caps {
                let _ = shell.backup(&cap).await;
            }
            Ok::<(), ShellError>(())
        });

        cx.spawn(async move |this, cx| {
            let _ = work.await;
            let _ = this.update(cx, |page, cx| {
                page.action_in_flight.remove(BACK_UP_ALL);
                page.load(cx);
            });
        })
        .detach();
    }

    fn render_header_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let backups_count = self
            .backups
            .value
            .as_ref()
            .map(|b| b.backups.len())
            .unwrap_or(0);
        let targets_count = self.targets.value.as_ref().map(|t| t.len()).unwrap_or(0);
        let (runs_count, running_count) = self
            .runs
            .value
            .as_ref()
            .map(|(r, run)| (r.len(), *run))
            .unwrap_or((0, 0));

        let is_backing_up_all = self.action_in_flight.contains_key(BACK_UP_ALL)
            || self.backups.value.as_ref().is_some_and(|b| b.running());

        div()
            .flex()
            .items_center()
            .justify_between()
            .pb_3()
            .border_b_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.render_tab_button(
                        "Capability Contracts",
                        BackupTab::Contracts,
                        backups_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Targets & Cadence",
                        BackupTab::Targets,
                        targets_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Attempt Ledger",
                        BackupTab::History,
                        runs_count,
                        palette,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .children(if running_count > 0 {
                        Some(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child(dot(palette.accent))
                                .child(format!("{running_count} active")),
                        )
                    } else {
                        None
                    })
                    .child(
                        Button::new("backup-all-header")
                            .small()
                            .label("Back up all now")
                            .loading(is_backing_up_all)
                            .on_click(cx.listener(|page, _, _, cx| page.trigger_backup_all(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_tab_button(
        &self,
        label: &str,
        tab: BackupTab,
        count: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_active = self.tab == tab;
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .font_weight(if is_active {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .bg(if is_active {
                rgba(tint(palette.accent, 0x20))
            } else {
                rgba(0)
            })
            .text_color(if is_active {
                rgb(palette.accent)
            } else {
                rgb(palette.muted)
            })
            .hover(|s| s.bg(rgba(tint(palette.border, 0x30))))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, cx| page.set_tab(tab, cx)),
            )
            .child(format!("{label} ({count})"))
            .into_any_element()
    }

    fn render_contracts_view(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(backups) = &self.backups.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading capability backup contracts…")
                .into_any_element();
        };

        if backups.backups.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No capabilities declare a backup contract on this machine.")
                .into_any_element();
        }

        let mut cards = Vec::new();
        for b in &backups.backups {
            cards.push(self.render_contract_card(b, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    fn render_contract_card(
        &self,
        b: &BackupStatus,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_running = self
            .action_in_flight
            .get(&format!("backup:{}", b.capability))
            .copied()
            .unwrap_or(false)
            || b.run.as_ref().is_some_and(|r| r.state == "running");
        let is_confirming = self.confirming_cold_copy.as_deref() == Some(&b.capability);

        let attempt_failed = b
            .attempt
            .as_ref()
            .is_some_and(|att| att.exit_code.is_some_and(|code| code != 0));

        let (badge_bg, badge_fg, badge_text) = match b.state.as_str() {
            "ok" | "fresh" => (
                rgba(tint(palette.green, 0x20)),
                palette.green,
                "ok".to_string(),
            ),
            "due" => (
                rgba(tint(palette.amber, 0x20)),
                palette.amber,
                "due".to_string(),
            ),
            "overdue" => (
                rgba(tint(palette.red, 0x20)),
                palette.red,
                "overdue".to_string(),
            ),
            "never" => (
                rgba(tint(palette.red, 0x20)),
                palette.red,
                "never".to_string(),
            ),
            _ => (
                rgba(tint(palette.muted, 0x20)),
                palette.muted,
                b.state.clone(),
            ),
        };

        let cap_name = b.capability.clone();
        let holds_service = b.holds_service;

        div()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .bg(rgba(tint(palette.raised, 0x40)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(b.capability.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(badge_bg)
                                            .text_color(rgb(badge_fg))
                                            .child(badge_text),
                                    )
                                    .children(if b.holds_service {
                                        Some(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded_full()
                                                .text_xs()
                                                .bg(rgba(tint(palette.amber, 0x20)))
                                                .text_color(rgb(palette.amber))
                                                .child("holds-service"),
                                        )
                                    } else {
                                        None
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(if b.last_success.is_some() {
                                        format!(
                                            "Backed up {} ({})",
                                            format_age_seconds(b.age_seconds),
                                            format_bytes(b.bytes)
                                        )
                                    } else {
                                        "Never backed up".to_string()
                                    })
                                    .child(format!(
                                        "[advise: {}d, stale: {}d]",
                                        b.advise_days.map(|d| d.to_string()).unwrap_or_else(|| "—".to_string()),
                                        b.stale_days.map(|d| d.to_string()).unwrap_or_else(|| "—".to_string())
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(if is_running {
                                Some(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.accent))
                                        .child("Backing up…")
                                        .into_any_element(),
                                )
                            } else if is_confirming {
                                Some(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Button::new(format!("confirm-{}", cap_name))
                                                .small()
                                                .label("Stop & back up")
                                                .on_click(cx.listener(move |page, _, _, cx| {
                                                    page.trigger_backup(cap_name.clone(), None, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("cancel-{}", b.capability))
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(|page, _, _, cx| {
                                                    page.confirming_cold_copy = None;
                                                    cx.notify();
                                                })),
                                        )
                                        .into_any_element(),
                                )
                            } else {
                                let cap_to_backup = cap_name.clone();
                                Some(
                                    Button::new(format!("backup-btn-{}", cap_name))
                                        .small()
                                        .label("Back up now")
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            if holds_service {
                                                page.confirming_cold_copy = Some(cap_to_backup.clone());
                                                cx.notify();
                                            } else {
                                                page.trigger_backup(cap_to_backup.clone(), None, cx);
                                            }
                                        }))
                                        .into_any_element(),
                                )
                            }),
                    ),
            )
            .children(if let Some(att) = &b.attempt {
                if attempt_failed {
                    Some(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgba(tint(palette.red, 0x15)))
                            .border_1()
                            .border_color(rgba(tint(palette.red, 0x30)))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_xs()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.red))
                                    .child(format!(
                                        "Last attempt FAILED (exit {}) · target: {}",
                                        att.exit_code.unwrap_or(-1),
                                        att.target
                                    )),
                            )
                            .children(if !att.detail.is_empty() {
                                Some(
                                    div()
                                        .text_color(rgb(palette.text))
                                        .font_family(".AppleSystemUIFontMonospaced")
                                        .child(att.detail.clone()),
                                )
                            } else {
                                None
                            })
                            .children(if !att.log_path.is_empty() {
                                Some(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Log: {}", att.log_path)),
                                )
                            } else {
                                None
                            }),
                    )
                } else {
                    None
                }
            } else {
                None
            })
            .children(if is_confirming {
                Some(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgba(tint(palette.amber, 0x15)))
                        .border_1()
                        .border_color(rgba(tint(palette.amber, 0x30)))
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .child(format!(
                            "A cold copy backup requires temporarily stopping '{}'. It will automatically restart once the archive is written.",
                            b.capability
                        )),
                )
            } else {
                None
            })
            .into_any_element()
    }

    fn render_targets_view(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(targets) = &self.targets.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading declared backup targets…")
                .into_any_element();
        };

        if targets.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No backup targets are declared on this machine.")
                .into_any_element();
        }

        let mut cards = Vec::new();
        for t in targets {
            cards.push(self.render_target_card(t, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(cards)
            .into_any_element()
    }

    fn render_target_card(
        &self,
        t: &BackupTargetRecord,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_verifying = self.verifying.get(&t.id).copied().unwrap_or(false);
        let tid = t.id.clone();

        let present_color = match t.present.as_str() {
            "true" => palette.green,
            "false" => palette.red,
            _ => palette.muted,
        };

        let verdict = self.verify_verdicts.get(&t.id).cloned().or_else(|| {
            t.verified_verdict.as_ref().map(|v| VerifyTargetResult {
                target: t.id.clone(),
                verdict: v.clone(),
                detail: t.verified_detail.clone().unwrap_or_default(),
            })
        });

        div()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .bg(rgba(tint(palette.raised, 0x40)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(t.id.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(rgba(tint(palette.border, 0x30)))
                                            .text_color(rgb(palette.muted))
                                            .child(t.kind.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(rgba(tint(present_color, 0x20)))
                                            .text_color(rgb(present_color))
                                            .child(match t.present.as_str() {
                                                "true" => "present".to_string(),
                                                "false" => "missing".to_string(),
                                                other => other.to_string(),
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .font_family(".AppleSystemUIFontMonospaced")
                                    .child(if !t.host.is_empty() {
                                        format!("{}:{}", t.host, t.path)
                                    } else {
                                        t.path.clone()
                                    }),
                            ),
                    )
                    .child(
                        Button::new(format!("verify-{}", t.id))
                            .small()
                            .label(if is_verifying {
                                "Verifying…"
                            } else {
                                "Verify target"
                            })
                            .loading(is_verifying)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.verify_target(tid.clone(), cx);
                            })),
                    ),
            )
            .children(verdict.map(|v| {
                let is_ok = v.verdict == "verified";
                let tone = if is_ok { palette.green } else { palette.red };
                div()
                    .p_2p5()
                    .rounded_md()
                    .bg(rgba(tint(tone, 0x15)))
                    .border_1()
                    .border_color(rgba(tint(tone, 0x30)))
                    .text_xs()
                    .text_color(rgb(tone))
                    .child(format!(
                        "Verification {}: {}",
                        v.verdict,
                        if v.detail.is_empty() {
                            "rehearsal pass"
                        } else {
                            &v.detail
                        }
                    ))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgba(tint(palette.border, 0x20)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Scheduled Cadence:"),
                    )
                    .child(self.render_cadence_selector(t, palette, cx)),
            )
            .into_any_element()
    }

    fn render_cadence_selector(
        &self,
        t: &BackupTargetRecord,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let presets = [
            ("Off", None),
            ("12h", Some(12)),
            ("Daily (24h)", Some(24)),
            ("48h", Some(48)),
            ("Weekly (168h)", Some(168)),
        ];

        let target_id = t.id.clone();
        let current_interval = t.interval_hours;

        div()
            .flex()
            .items_center()
            .gap_1()
            .children(presets.into_iter().map(|(label, hours)| {
                let tid = target_id.clone();
                let is_selected = current_interval == hours;
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .cursor_pointer()
                    .bg(if is_selected {
                        rgba(tint(palette.accent, 0x25))
                    } else {
                        rgba(tint(palette.raised, 0x20))
                    })
                    .text_color(if is_selected {
                        rgb(palette.accent)
                    } else {
                        rgb(palette.muted)
                    })
                    .border_1()
                    .border_color(if is_selected {
                        rgb(palette.accent)
                    } else {
                        rgba(tint(palette.border, 0x30))
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |page, _, _, cx| {
                            page.set_target_policy(tid.clone(), hours, cx);
                        }),
                    )
                    .child(label)
            }))
            .into_any_element()
    }

    fn render_history_view(&self, palette: Palette, _cx: &mut Context<Self>) -> AnyElement {
        let Some((runs, _running)) = &self.runs.value else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading attempt history ledger…")
                .into_any_element();
        };

        if runs.is_empty() {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No recent backup runs recorded.")
                .into_any_element();
        }

        let mut rows = Vec::new();
        for r in runs {
            rows.push(self.render_run_row(r, palette));
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(rows)
            .into_any_element()
    }

    fn render_run_row(&self, r: &BackupRunAttemptRecord, palette: Palette) -> AnyElement {
        let outcome = run_outcome(r.exit_code, r.finished_epoch);
        let (outcome_bg, outcome_fg, outcome_label) = match outcome {
            "ok" => (rgba(tint(palette.green, 0x20)), palette.green, "exit 0"),
            "running" => (rgba(tint(palette.accent, 0x20)), palette.accent, "running"),
            _ => (rgba(tint(palette.red, 0x20)), palette.red, "failed"),
        };

        let duration_label = format_duration(r.started_epoch, r.finished_epoch);

        div()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgba(tint(palette.border, 0x20)))
            .bg(rgba(tint(palette.raised, 0x30)))
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(r.capability.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("→ {}", r.target)),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(outcome_bg)
                                    .text_color(rgb(outcome_fg))
                                    .child(outcome_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("duration: {duration_label}"))
                            .child(r.started_at.clone()),
                    ),
            )
            .children(r.archive.as_ref().map(|arc| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .child(format!(
                        "Archive: {} ({})",
                        arc.name,
                        format_bytes(Some(arc.bytes as u64))
                    ))
                    .child(format!("SHA: {}", &arc.sha256[..12.min(arc.sha256.len())]))
            }))
            .children(if !r.detail.is_empty() {
                Some(
                    div()
                        .text_xs()
                        .text_color(if outcome == "failed" {
                            rgb(palette.red)
                        } else {
                            rgb(palette.muted)
                        })
                        .font_family(".AppleSystemUIFontMonospaced")
                        .child(r.detail.clone()),
                )
            } else {
                None
            })
            .into_any_element()
    }
}

impl Render for BackupPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(WindowAppearance::Dark));

        div()
            .id("backup-page")
            .key_context("BackupView")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|page, _: &ShowContracts, _, cx| {
                    page.set_tab(BackupTab::Contracts, cx)
                }),
            )
            .on_action(
                cx.listener(|page, _: &ShowTargets, _, cx| page.set_tab(BackupTab::Targets, cx)),
            )
            .on_action(
                cx.listener(|page, _: &ShowHistory, _, cx| page.set_tab(BackupTab::History, cx)),
            )
            .on_action(cx.listener(|page, _: &RefreshBackup, _, cx| page.refresh(cx)))
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .child(self.render_header_bar(palette, cx))
            .child(
                div()
                    .id("backup-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(match self.tab {
                        BackupTab::Contracts => self.render_contracts_view(palette, cx),
                        BackupTab::Targets => self.render_targets_view(palette, cx),
                        BackupTab::History => self.render_history_view(palette, cx),
                    }),
            )
    }
}

/// `never`, `just now`, `5h ago`. A test-only shorthand beside [`format_age_seconds`], which the
/// page calls directly with the row it is drawing.
#[cfg(test)]
pub fn age(seconds: Option<u64>) -> String {
    format_age_seconds(seconds)
}

pub fn size(bytes: u64) -> String {
    format_bytes(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_and_sizes_read_at_a_glance() {
        assert_eq!(age(None), "never");
        assert_eq!(age(Some(30)), "just now");
        assert_eq!(age(Some(5 * 3_600)), "5h ago");
        assert_eq!(age(Some(3 * 86_400)), "3d ago");
        assert_eq!(size(63 * 1_048_576), "63.0 MB");
        assert_eq!(size(4_294_967_296), "4.0 GB");
    }

    #[gpui::test]
    fn test_backup_tab_switching(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|cx| {
            let repo = Repo::new(Err(ShellError::NotSetUp));
            BackupPage::new(repo, cx)
        });

        page.update(cx, |page, cx| {
            assert_eq!(page.tab, BackupTab::Contracts);
            page.set_tab(BackupTab::Targets, cx);
            assert_eq!(page.tab, BackupTab::Targets);
            page.set_tab(BackupTab::History, cx);
            assert_eq!(page.tab, BackupTab::History);
        });
    }
}
