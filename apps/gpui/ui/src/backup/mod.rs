//! Deep backup inspection, capability contracts, target cadence policy, rehearsal verification,
//! and durable run attempt history.

pub mod actions;
pub mod contracts;
pub mod history;
pub mod targets;
pub mod types;

pub use actions::*;
pub use types::*;

use std::collections::HashMap;

use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, rgb, rgba, AnyElement, Context, FocusHandle, Render, ScrollHandle, Window,
};

use crate::data::{Remote, Repo};
use crate::primitives::{dot, key_bar, sub_tabs};
use crate::theme::{tint, Palette, ThemeContext};
use sjel_gpui_core::backup::{
    BackupRunAttemptRecord, BackupTargetRecord, VerifyTargetResult,
};
use sjel_gpui_core::shell::{self, Backups, ShellError};

pub struct BackupPage {
    pub repo: Repo,
    pub palette: Option<Palette>,
    pub tab: BackupTab,
    pub backups: Remote<Backups>,
    pub targets: Remote<Vec<BackupTargetRecord>>,
    pub runs: Remote<(Vec<BackupRunAttemptRecord>, usize)>,
    pub verifying: HashMap<String, bool>,
    pub verify_verdicts: HashMap<String, VerifyTargetResult>,
    pub confirming_cold_copy: Option<String>,
    pub selected_target_override: HashMap<String, String>,
    pub action_in_flight: HashMap<String, bool>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) focus: FocusHandle,
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
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.clone()
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
        let entity = cx.entity();

        div()
            .flex()
            .items_center()
            .justify_between()
            .pb_3()
            .border_b_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .child(sub_tabs(
                "backup",
                vec![
                    (
                        "Capability Contracts",
                        Some(backups_count),
                        self.tab == BackupTab::Contracts,
                    ),
                    (
                        "Targets & Cadence",
                        Some(targets_count),
                        self.tab == BackupTab::Targets,
                    ),
                    (
                        "Attempt Ledger",
                        Some(runs_count),
                        self.tab == BackupTab::History,
                    ),
                ],
                palette,
                move |idx, _window, cx| {
                    entity.update(cx, |page, cx| {
                        let next = match idx {
                            0 => BackupTab::Contracts,
                            1 => BackupTab::Targets,
                            _ => BackupTab::History,
                        };
                        page.set_tab(next, cx);
                    });
                },
            ))
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
}

impl Render for BackupPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);

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
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.scroll)
                    .child(match self.tab {
                        BackupTab::Contracts => self.render_contracts_view(palette, cx),
                        BackupTab::Targets => self.render_targets_view(palette, cx),
                        BackupTab::History => self.render_history_view(palette, cx),
                    }),
            )
            .child(key_bar(&[("1 / 2 / 3", "switch tab")], palette))
    }
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
