//! Keyboard selection: J/K move a cursor over the rows a page draws, Enter runs the row's
//! primary action, ⌘⌫ its costly one. The selected row reveals the actions a mouse gets on hover.

use sjel_gpui_core::context::{needs_attention, ATTENTION_SHOWN};
pub use sjel_gpui_ui::selection::{Item, Selection};

use crate::route::Page;
use crate::shell::GlobalTask;
use crate::workspace::Workspace;

impl Workspace {
    /// The rows of the current page, in drawing order. A page highlights a row by comparing its
    /// `Item` with `selected`, never by position, so Enter acts on the highlighted row even when
    /// this order and the drawing order differ.
    pub fn items(&self) -> Vec<Item> {
        match self.page {
            Page::Home => {
                let mut items: Vec<Item> = self
                    .agent
                    .value
                    .iter()
                    .flat_map(|agent| agent.pending.iter().map(|a| Item::Approval(a.id.clone())))
                    .collect();
                if let Some(health) = &self.health.value {
                    items.extend(health.down().into_iter().map(Item::Down));
                }
                items.extend(
                    self.urgent_tasks()
                        .into_iter()
                        .map(|t| Item::FocusTask(t.id)),
                );
                items
            }
            Page::Capabilities => self
                .health
                .value
                .iter()
                .flat_map(|health| health.capabilities.iter())
                .map(|(name, state)| Item::Capability {
                    name: name.clone(),
                    up: state.up,
                })
                .collect(),
            Page::Projects
            | Page::Context
            | Page::Travel
            | Page::Feed
            | Page::Inbox
            | Page::Calendar
            | Page::Documents
            | Page::Backup
            | Page::Finance
            | Page::People
            | Page::Inventory
            | Page::Software
            | Page::Links
            | Page::Import
            | Page::Systems
            | Page::Packs
            | Page::SelfModel
            | Page::Cv
            | Page::Media
            | Page::Research
            | Page::Setup
            | Page::Docs
            | Page::Record
            | Page::Settings => Vec::new(),
        }
    }

    /// The row under the cursor.
    pub fn selected(&self) -> Option<Item> {
        self.items().get(self.selection.cursor?).cloned()
    }

    /// The urgent rows the Today page draws: the first [`ATTENTION_SHOWN`] of
    /// [`Self::urgent_task_count`], so a row the reader can select is a row that is on screen.
    pub fn urgent_tasks(&self) -> Vec<GlobalTask> {
        self.urgent_task_count_all()
            .into_iter()
            .take(ATTENTION_SHOWN)
            .collect()
    }

    /// Every open task that is P1 or due by today, including the ones the list does not draw.
    /// The TASKS card counts these, and the list says how many it left out.
    pub fn urgent_task_count(&self) -> usize {
        self.urgent_task_count_all().len()
    }

    fn urgent_task_count_all(&self) -> Vec<GlobalTask> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.focus_tasks
            .value
            .as_ref()
            .map(|tasks| needs_attention(tasks, &today))
            .unwrap_or_default()
    }
}
