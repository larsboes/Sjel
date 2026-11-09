//! Systems: what this Mac is doing now, and what it is carrying.
//!
//! Six panels, each one another owner's answer with the page's own wording on top:
//!
//! - **Runtime profile** — which profile is in force and what it admits (`sjel_runtime`).
//! - **Mac** — temperature, power, CPU and memory from macmon, every three seconds.
//! - **Agent** — the agent's reach on this machine: one mode per capability, the writes
//!   waiting for the owner, and the latest calls.
//! - **Storage** — what fills the disk, from `tools/storage report --json`, plus the disk's own
//!   hourly history so a slope reads where a snapshot cannot.
//! - **Updates** — installed software outside this checkout and who moves it, with one button
//!   per class. The one long-running write on this page.
//! - **Host watch** — the conditions the hourly watch has now, and since when.
//!
//! Storage, updates and host watch are read from sjel-status rather than from the tools, because
//! the tools measure and exit; the shell is the surface that stays up. macmon is a capability, so
//! `/macmon/json` reaches it through the same proxy every other capability uses.
//!
//! Nothing here is on the ten-second workspace beat: temperature wants three seconds and the
//! update report asks three registries, so this page runs its own clocks while it is visible and
//! stops them when it is not.

mod agent;
mod explorer;
mod overview;
mod storage;
mod types;

use gpui_kit::{
    div, prelude::*, rgb, AnyElement, Context, IntoElement, Render, SharedString, Window,
};

use crate::data::{Busy, Remote, Repo};
use crate::primitives::sub_tabs;
use crate::theme::{Palette, ThemeContext};
use sjel_gpui_core::shell::AgentView;
use sjel_gpui_core::systems::{
    Finding, MacmonSample, RuntimeStatus, StorageHistory, StorageReport, UpdatesReport,
};

pub use types::SystemsTab;
use types::*;

pub struct SystemsPage {
    pub(super) repo: Repo,
    pub(super) tab: SystemsTab,
    pub(super) macmon: Remote<MacmonSample>,
    pub(super) storage: Remote<StorageReport>,
    pub(super) history: Remote<StorageHistory>,
    explorer: explorer::ExplorerState,
    pub(super) updates: Remote<UpdatesReport>,
    pub(super) findings: Remote<Vec<Finding>>,
    pub(super) agent: Remote<AgentView>,
    pub(super) runtime: Remote<RuntimeStatus>,
    derived: SystemsDerived,
    pub(super) busy: Busy,
    /// Bumped on every activation. A loop that sees a different generation stops, which is how
    /// leaving the page stops three clocks without a handle to cancel.
    pub(super) generation: u64,
}

impl SystemsPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            tab: SystemsTab::Overview,
            macmon: Remote::default(),
            storage: Remote::default(),
            history: Remote::default(),
            explorer: explorer::ExplorerState::default(),
            updates: Remote::default(),
            findings: Remote::default(),
            agent: Remote::default(),
            runtime: Remote::default(),
            derived: SystemsDerived::default(),
            busy: Busy::default(),
            generation: 0,
        }
    }

    pub fn set_tab(&mut self, tab: SystemsTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.explorer.stop();
            self.tab = tab;
            if tab == SystemsTab::Storage {
                self.read_explorer(false, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn settle_derived(&mut self) {
        if self.derived.storage_at != self.storage.loaded_at {
            self.derived.storage_at = self.storage.loaded_at;
            self.derived.storage_view = self.storage.value.as_ref().map(StorageReport::view);
        }
        if self.derived.history_at != self.history.loaded_at {
            self.derived.history_at = self.history.loaded_at;
            self.derived.storage_trend =
                self.history.value.as_ref().and_then(StorageHistory::trend);
            self.derived.class_deltas = self
                .history
                .value
                .as_ref()
                .map(StorageHistory::class_deltas)
                .unwrap_or_default();
        }
        if self.derived.updates_at != self.updates.loaded_at {
            self.derived.updates_at = self.updates.loaded_at;
            self.derived.updates_view = self.updates.value.as_ref().map(UpdatesReport::view);
        }
    }

    /// Reads everything once and starts the page's clocks.
    ///
    /// One generation for all three, taken here rather than per clock: a clock stops as soon as
    /// it sees a different number, so bumping it once per `start_clock` would leave only the
    /// last one alive.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.load(cx);
        self.start_clock(generation, MACMON_BEAT, |page, cx| page.read_macmon(cx), cx);
        self.start_clock(generation, AGENT_BEAT, |page, cx| page.read_agent(cx), cx);
        self.start_clock(
            generation,
            RUNTIME_BEAT,
            |page, cx| page.read_runtime(cx),
            cx,
        );
    }

    /// Stops the clocks. A page that is no longer drawn must not keep asking the machine.
    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.explorer.stop();
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub(super) fn load(&mut self, cx: &mut Context<Self>) {
        self.read_macmon(cx);
        if self.tab == SystemsTab::Storage {
            self.read_explorer(false, cx);
        }
        self.read_history(cx);
        self.read_updates(cx);
        self.read_findings(cx);
        self.read_agent(cx);
        self.read_runtime(cx);
    }

    /// One clock: `beat` apart, calling `tick` until the page is left.
    ///
    /// A `tick` that finds the read already in flight does nothing — `Remote::begin` is the
    /// guard — so a slow answer never stacks requests.
    pub(super) fn start_clock(
        &mut self,
        generation: u64,
        beat: std::time::Duration,
        tick: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor().timer(beat).await;
            let live = weak
                .update(cx, |page, cx| {
                    if page.generation != generation {
                        return false;
                    }
                    tick(page, cx);
                    true
                })
                .unwrap_or(false);
            if !live {
                return;
            }
        })
        .detach();
    }
}

impl Render for SystemsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        self.settle_derived();

        let pending_approvals = self
            .agent
            .value
            .as_ref()
            .map_or(0, |agent| agent.pending.len());
        let stale_updates = self
            .derived
            .updates_view
            .as_ref()
            .map_or(0, |view| view.stale);
        let findings_count = self.findings.value.as_ref().map_or(0, Vec::len);

        let entity = cx.entity().clone();
        let tabs = sub_tabs(
            "systems",
            vec![
                (
                    "Overview",
                    (findings_count > 0).then_some(findings_count),
                    self.tab == SystemsTab::Overview,
                ),
                (
                    "Storage & Updates",
                    (stale_updates > 0).then_some(stale_updates),
                    self.tab == SystemsTab::Storage,
                ),
                (
                    "Agent Gate",
                    (pending_approvals > 0).then_some(pending_approvals),
                    self.tab == SystemsTab::Agent,
                ),
            ],
            palette,
            move |idx, _, app| {
                entity.update(app, |page, cx| {
                    let tab = match idx {
                        0 => SystemsTab::Overview,
                        1 => SystemsTab::Storage,
                        _ => SystemsTab::Agent,
                    };
                    page.set_tab(tab, cx);
                });
            },
        );

        let mut root = div().flex().flex_col().gap_6().child(tabs);
        match self.tab {
            SystemsTab::Overview => {
                let runtime = self.runtime_panel(palette, cx);
                let mac = self.mac_panel(palette, cx);
                let findings = self.findings_panel(palette);
                root = root.child(runtime).child(mac).child(findings);
            }
            SystemsTab::Storage => {
                let storage = self.storage_panel(palette, cx);
                let updates = self.updates_panel(palette, cx);
                root = root.child(storage).child(updates);
            }
            SystemsTab::Agent => {
                let agent = self.agent_panel(palette, cx);
                root = root.child(agent);
            }
        }
        root.into_any_element()
    }
}

pub(super) fn heading(title: impl Into<SharedString>, palette: Palette) -> AnyElement {
    div()
        .pt_2()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(title.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_slot_is_not_read_twice_at_once() {
        let mut page = SystemsPage::new(Repo::new(Err(
            sjel_gpui_core::shell::ShellError::NotLoopback,
        )));
        assert!(page.macmon.begin(), "the first read starts");
        assert!(!page.macmon.begin(), "a second read waits for the first");
        assert_eq!(page.generation, 0);
    }
}
