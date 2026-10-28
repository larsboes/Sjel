//! Household inventory, asset catalog, wishlist budget, and interior spatial layout.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    KeyBinding, Render, Subscription, Window, WindowAppearance,
};

use crate::data::{Remote, Repo};
use crate::interior::RoomPlanHistory;
use crate::theme::{tint, Palette};
use sjel_gpui_core::interior::RoomPlanSource;
use sjel_gpui_core::inventory::{
    budget_of, filter_inventory, format_cents, format_dimensions, group_of, months_to_pay,
    wishlist_view, InteriorLayoutSummary, InventoryItem, InventoryRow, InventoryState,
    InventoryWishlist, MailMatchSnapshot, MailProposal, NewInventoryItem, WishGroup,
};
use sjel_gpui_core::shell::{self, ShellError};

#[cfg(target_os = "macos")]
fn cache_preview_roomplan(revision_id: &str, draft_json: &str, asset_bytes: Vec<u8>) {
    let revision_id = revision_id.to_owned();
    let draft_json = draft_json.to_owned();
    std::thread::spawn(move || {
        if revision_id.is_empty()
            || !revision_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return;
        }
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let root =
            std::path::PathBuf::from(home).join("Library/Application Support/dev.sjel/roomplan");
        let directory = root.join(&revision_id);
        let asset = directory.join("parametric.usdz");
        if directory.exists() {
            let existing_draft = std::fs::read(directory.join("draft.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if existing_draft
                .as_ref()
                .and_then(|draft| draft.get("draft_id"))
                .and_then(serde_json::Value::as_str)
                != Some(revision_id.as_str())
                || std::fs::read(&asset).ok().as_deref() != Some(asset_bytes.as_slice())
            {
                return;
            }
            let draft_path = directory.join("draft.json");
            let draft_temp = directory.join(".draft.json.tmp");
            if std::fs::write(&draft_temp, draft_json).is_err()
                || std::fs::rename(&draft_temp, &draft_path).is_err()
            {
                let _ = std::fs::remove_file(draft_temp);
                return;
            }
        } else {
            if serde_json::from_str::<serde_json::Value>(&draft_json)
                .ok()
                .and_then(|draft| {
                    draft
                        .get("draft_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                != Some(revision_id.as_str())
            {
                return;
            }
            let staging = root.join(format!(".{}.{}.tmp", revision_id, std::process::id()));
            if std::fs::create_dir_all(&root).is_err()
                || std::fs::create_dir(&staging).is_err()
                || std::fs::write(staging.join("draft.json"), draft_json).is_err()
                || std::fs::write(staging.join("parametric.usdz"), asset_bytes).is_err()
            {
                let _ = std::fs::remove_dir_all(&staging);
                return;
            }
            if std::fs::rename(&staging, &directory).is_err() {
                let _ = std::fs::remove_dir_all(&staging);
                return;
            }
        }
        let _ = std::process::Command::new("/usr/bin/qlmanage")
            .args(["-p"])
            .arg(asset)
            .spawn();
    });
}

actions!(
    inventory,
    [
        /// Show the owned assets tab.
        ShowOwned,
        /// Show the wishlist budget tab.
        ShowWishlist,
        /// Show the interior layout summaries tab.
        ShowInterior,
        /// Show mail proposals awaiting review.
        ShowMailReview,
        /// Reload inventory data from the shell.
        RefreshInventory,
        /// Focus the search input field.
        FocusSearch,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ShowOwned, Some("InventoryView")),
        KeyBinding::new("2", ShowWishlist, Some("InventoryView")),
        KeyBinding::new("3", ShowInterior, Some("InventoryView")),
        KeyBinding::new("4", ShowMailReview, Some("InventoryView")),
        KeyBinding::new("/", FocusSearch, Some("InventoryView")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InventoryTab {
    #[default]
    Owned,
    Wishlist,
    Interior,
    MailReview,
}

struct MailReviewForm {
    proposal: MailProposal,
    title: Entity<InputState>,
    item_id: Entity<InputState>,
    snapshot: Option<MailMatchSnapshot>,
    _subscriptions: Vec<Subscription>,
}

struct WishForm {
    fields: Vec<(String, Entity<InputState>)>,
    kind: String,
    state: InventoryState,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl WishForm {
    fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _)| field == key)
            .map(|(_, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

struct InventoryEditForm {
    item: InventoryItem,
    fields: Vec<(String, String, Entity<InputState>)>,
    error: Option<String>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

/// The two fields a wish group is made of. Its members are chosen on the wishes themselves.
struct GroupForm {
    fields: Vec<(String, String, Entity<InputState>)>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl GroupForm {
    fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

impl InventoryEditForm {
    fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

pub struct InventoryPage {
    repo: Repo,
    palette: Option<Palette>,
    pub tab: InventoryTab,
    pub rows: Remote<Vec<InventoryRow>>,
    pub wishlist: Remote<InventoryWishlist>,
    pub wish_groups: Remote<Vec<WishGroup>>,
    pub layouts: Remote<Vec<InteriorLayoutSummary>>,
    mail_proposals: Remote<Vec<MailProposal>>,
    mail_form: Option<MailReviewForm>,
    mail_busy: bool,
    mail_message: Option<String>,
    roomplan_history: Entity<RoomPlanHistory>,
    pub search_query: String,
    pub selected_category: Option<String>,
    /// The wish group the wishlist is narrowed to, by id. None shows every open wish.
    pub selected_group: Option<String>,
    search_input: Option<(Entity<InputState>, Subscription)>,
    wish_form: Option<WishForm>,
    group_form: Option<GroupForm>,
    /// The wish whose group picker is open, by id.
    group_menu: Option<String>,
    /// The group a write is in flight for, so one press cannot make two.
    group_busy: Option<String>,
    confirm_remove_group: Option<String>,
    edit_form: Option<InventoryEditForm>,
    wish_busy: Option<String>,
    wish_notice: Option<String>,
    wish_error: Option<String>,
    confirm_remove_wish: Option<String>,
    focus: FocusHandle,
}

impl InventoryPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        let source = repo.shell().ok().map(RoomPlanSource::Shell);
        let roomplan_history = cx.new(|cx| {
            let mut history = RoomPlanHistory::new(source, cx);
            #[cfg(target_os = "macos")]
            history.set_cache_preview_handler(Some(cache_preview_roomplan));
            history
        });
        Self {
            repo,
            palette: None,
            tab: InventoryTab::Owned,
            rows: Remote::default(),
            wishlist: Remote::default(),
            wish_groups: Remote::default(),
            layouts: Remote::default(),
            mail_proposals: Remote::default(),
            mail_form: None,
            mail_busy: false,
            mail_message: None,
            roomplan_history,
            search_query: String::new(),
            selected_category: None,
            selected_group: None,
            search_input: None,
            wish_form: None,
            group_form: None,
            group_menu: None,
            group_busy: None,
            confirm_remove_group: None,
            edit_form: None,
            wish_busy: None,
            wish_notice: None,
            wish_error: None,
            confirm_remove_wish: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
        self.roomplan_history
            .update(cx, |history, cx| history.set_palette(palette, cx));
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn set_tab(&mut self, tab: InventoryTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            if tab == InventoryTab::Interior {
                self.roomplan_history
                    .update(cx, |history, cx| history.refresh(cx));
            }
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_proposals.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.load_mail(cx);

        self.rows.loading = true;
        self.wishlist.loading = true;
        self.wish_groups.loading = true;
        self.layouts.loading = true;
        cx.notify();

        // 1. Fetch Inventory Rows
        let s_i = shell.clone();
        let i_work = shell::run(async move { s_i.inventory_rows().await });
        cx.spawn(async move |this, cx| {
            let res = i_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.rows.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 2. Fetch Wishlist
        let s_w = shell.clone();
        let w_work = shell::run(async move { s_w.inventory_wishlist().await });
        cx.spawn(async move |this, cx| {
            let res = w_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wishlist.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 3. Fetch Wish Groups
        let s_g = shell.clone();
        let g_work = shell::run(async move { s_g.inventory_wish_groups().await });
        cx.spawn(async move |this, cx| {
            let res = g_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_groups.settle(res);
                // A group that is no longer on the list would narrow the wishlist to nothing.
                let known = page.wish_groups.value.as_ref().is_some_and(|groups| {
                    page.selected_group
                        .as_deref()
                        .is_some_and(|id| groups.iter().any(|group| group.id == id))
                });
                if !known {
                    page.selected_group = None;
                }
                cx.notify();
            });
        })
        .detach();

        // 4. Fetch Interior Layouts
        let s_l = shell.clone();
        let l_work = shell::run(async move { s_l.interior_layouts().await });
        cx.spawn(async move |this, cx| {
            let res = l_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.layouts.settle(res);
                cx.notify();
            });
        })
        .detach();

        self.roomplan_history
            .update(cx, |history, cx| history.refresh(cx));
    }

    fn load_mail(&mut self, cx: &mut Context<Self>) {
        if self.mail_proposals.loading {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_proposals.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.mail_proposals.loading = true;
        let work = shell::run(async move { shell.inventory_mail_proposals().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.mail_proposals.settle(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_mail(&mut self, proposal: MailProposal, window: &mut Window, cx: &mut Context<Self>) {
        let title = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Item title");
            input.set_value(&proposal.title, window, cx);
            input
        });
        let item_id = cx.new(|cx| InputState::new(window, cx).placeholder("Inventory item ID"));
        let subscription = cx.subscribe_in(&item_id, window, |page, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(form) = page.mail_form.as_mut() {
                    form.snapshot = None;
                }
                cx.notify();
            }
        });
        self.mail_form = Some(MailReviewForm {
            proposal,
            title,
            item_id,
            snapshot: None,
            _subscriptions: vec![subscription],
        });
        self.mail_message = None;
        cx.notify();
    }

    fn mail_snapshot(&mut self, cx: &mut Context<Self>) {
        if self.mail_busy {
            return;
        }
        let Some(form) = &self.mail_form else {
            return;
        };
        let id = form.item_id.read(cx).value().trim().to_owned();
        if id.is_empty() {
            self.mail_message = Some("Enter an owned item ID first.".into());
            cx.notify();
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.mail_busy = true;
        self.mail_message = None;
        let proposal_id = form.proposal.id;
        let work = shell::run(async move { shell.inventory_mail_snapshot(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.mail_busy = false;
                match result {
                    Ok(snapshot)
                        if snapshot.state.as_deref() == Some("owned")
                            && snapshot.state_id.is_some() =>
                    {
                        if let Some(form) = page.mail_form.as_mut() {
                            if form.proposal.id == proposal_id
                                && form.item_id.read(cx).value().trim() == snapshot.item_id
                            {
                                form.snapshot = Some(snapshot);
                            }
                        }
                    }
                    Ok(_) => {
                        page.mail_message = Some(
                            "The target is not a currently owned item with a state revision."
                                .into(),
                        )
                    }
                    Err(error) => {
                        page.mail_message = Some(format!("Could not load the target: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn decide_mail(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.mail_busy {
            return;
        }
        if self.mail_proposals.loading || self.mail_proposals.error.is_some() {
            self.mail_message = Some("Refresh proposals successfully before deciding.".into());
            cx.notify();
            return;
        }
        let Some(form) = &self.mail_form else {
            return;
        };
        let id = form.proposal.id;
        let version = form.proposal.version;
        let target = form.item_id.read(cx).value().trim().to_owned();
        let title = form.title.read(cx).value().trim().to_owned();
        let body = match action {
            "reject" => serde_json::json!({"version": version, "action": "reject"}),
            "approve"
                if form.proposal.event == "delivered"
                    && form.proposal.data_class != "c2"
                    && !target.is_empty()
                    && !title.is_empty() =>
            {
                serde_json::json!({"version": version, "action": "approve", "item_id": target, "title": title})
            }
            "match" => match &form.snapshot {
                Some(snapshot) if snapshot.item_id == target && snapshot.state_id.is_some() => {
                    serde_json::json!({"version": version, "action": "match", "item_id": target, "expected_item_revision": snapshot.item_revision, "expected_state_id": snapshot.state_id})
                }
                _ => {
                    self.mail_message =
                        Some("Load and review the owned target before matching.".into());
                    cx.notify();
                    return;
                }
            },
            _ => {
                self.mail_message = Some(
                    "Creation needs a delivered, non-c2 proposal, a title and a new item ID."
                        .into(),
                );
                cx.notify();
                return;
            }
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.mail_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.mail_busy = true;
        self.mail_message = None;
        let work = shell::run(async move { shell.inventory_mail_decision(id, body).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.mail_busy = false;
                match result {
                    Ok(()) => {
                        page.mail_form = None;
                        page.mail_message = Some("Decision saved.".into());
                        page.load(cx);
                    }
                    Err(error @ ShellError::Refused(409, _)) => {
                        page.mail_form = None;
                        page.mail_message = Some(format!(
                            "The proposal or target changed. Refresh and review again. {error}"
                        ));
                        page.load_mail(cx);
                    }
                    Err(error) => page.mail_message = Some(format!("Decision failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_mail_review(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child("Mail proposals"));
        if self.mail_proposals.loading {
            content = content.child("Loading mail proposals…");
        }
        if let Some(error) = &self.mail_proposals.error {
            content = content
                .child(
                    div()
                        .text_color(rgb(palette.red))
                        .child(format!("Could not load mail proposals: {error}")),
                )
                .child(
                    Button::new("mail-retry")
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|page, _, _, cx| page.load_mail(cx))),
                );
        }
        if self.mail_proposals.error.is_some() && self.mail_proposals.value.is_some() {
            content = content.child("Showing the previous result. Refresh before deciding.");
        }
        if let Some(message) = &self.mail_message {
            content = content.child(div().text_sm().child(message.clone()));
        }
        if let Some(form) = &self.mail_form {
            let p = &form.proposal;
            let mut detail = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(rgba(tint(palette.border, 0x55)))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(p.title.clone()),
                )
                .child(format!(
                    "Event: {} · Classification: {} · Version: {}",
                    p.event, p.data_class, p.version
                ))
                .child(format!("Source: {}", p.source_id))
                .child(format!("Line: {}", p.line_id))
                .child(format!(
                    "Order reference: {}",
                    p.order_ref.as_deref().unwrap_or("—")
                ))
                .child("Evidence (exact)")
                .child(p.evidence.clone())
                .child("Title for new item")
                .child(Input::new(&form.title).small())
                .child("New item ID or owned target ID")
                .child(Input::new(&form.item_id).small());
            if let Some(url) = p.gmail_url() {
                detail = detail.child(
                    Button::new("mail-open-source")
                        .small()
                        .label("Open source in Gmail")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            if p.data_class == "c2" {
                detail = detail.child(
                    "Creation is unavailable for c2 evidence. Match an owned item or reject.",
                );
            }
            if p.event != "delivered" {
                detail = detail.child("Only a delivery can create an item.");
            }
            if let Some(snapshot) = &form.snapshot {
                detail = detail.child(format!(
                    "Owned target: {} · {} · item revision {} · state revision {}",
                    snapshot.title,
                    snapshot.item_id,
                    snapshot.item_revision,
                    snapshot.state_id.unwrap_or_default()
                ));
            }
            detail = detail.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("mail-create")
                            .small()
                            .primary()
                            .label("Approve and create")
                            .disabled(
                                self.mail_busy || p.event != "delivered" || p.data_class == "c2",
                            )
                            .on_click(
                                cx.listener(|page, _, _, cx| page.decide_mail("approve", cx)),
                            ),
                    )
                    .child(
                        Button::new("mail-load-target")
                            .small()
                            .label("Review target")
                            .disabled(self.mail_busy)
                            .on_click(cx.listener(|page, _, _, cx| page.mail_snapshot(cx))),
                    )
                    .child(
                        Button::new("mail-match")
                            .small()
                            .label("Approve match")
                            .disabled(self.mail_busy || form.snapshot.is_none())
                            .on_click(cx.listener(|page, _, _, cx| page.decide_mail("match", cx))),
                    )
                    .child(
                        Button::new("mail-reject")
                            .small()
                            .label("Reject")
                            .disabled(self.mail_busy)
                            .on_click(cx.listener(|page, _, _, cx| page.decide_mail("reject", cx))),
                    )
                    .child(
                        Button::new("mail-close")
                            .small()
                            .ghost()
                            .label("Close")
                            .disabled(self.mail_busy)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.mail_form = None;
                                cx.notify();
                            })),
                    ),
            );
            content = content.child(detail);
        }
        if let Some(proposals) = &self.mail_proposals.value {
            let pending: Vec<_> = proposals.iter().filter(|p| p.decision.is_none()).collect();
            if pending.is_empty() {
                content = content.child("No mail proposals need review.");
            }
            for p in pending {
                let proposal = p.clone();
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(format!("{} · {} · {}", p.title, p.event, p.data_class))
                        .child(
                            Button::new(format!("mail-open-{}", p.id))
                                .small()
                                .label("Review")
                                .disabled(self.mail_busy)
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.open_mail(proposal.clone(), window, cx)
                                })),
                        ),
                );
            }
        }
        content.into_any_element()
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search inventory…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The Interior tab draws no search field, so a focus there would land nowhere.
        if matches!(self.tab, InventoryTab::Interior | InventoryTab::MailReview) {
            self.set_tab(InventoryTab::Owned, cx);
        }
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    fn open_group_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.group_form.is_some() {
            self.group_form = None;
            cx.notify();
            return;
        }
        let definitions = [
            ("title", "Title", "Bad", "What the group is called"),
            (
                "constraint",
                "Constraint",
                "",
                "What bounds it, e.g. one month's surplus",
            ),
        ];
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label, value, placeholder) in definitions {
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            });
            subscriptions.push(
                cx.subscribe_in(&input, window, |page, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        page.create_group(window, cx);
                    }
                }),
            );
            fields.push((key.to_owned(), label.to_owned(), input));
        }
        if let Some((_, _, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.group_form = Some(GroupForm {
            fields,
            error: None,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        cx.notify();
    }

    fn create_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.group_form.as_ref()) else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        let title = form.value("title", cx).trim().to_owned();
        if title.is_empty() {
            if let Some(form) = self.group_form.as_mut() {
                form.error = Some("Give the group a name.".to_owned());
            }
            cx.notify();
            return;
        }
        let constraint_text = form.value("constraint", cx).trim().to_owned();
        let id = sjel_gpui_core::capture::slug(&title);
        let write_id = id.clone();
        let write_title = title.clone();
        let write_constraint = constraint_text.clone();
        self.group_busy = Some(id.clone());
        self.wish_error = None;
        let work = shell::run(async move {
            shell
                .inventory_save_wish_group(&write_id, &write_title, &write_constraint)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        page.group_form = None;
                        page.selected_group = Some(id.clone());
                        page.wish_notice = Some(format!("Group \"{title}\" created."));
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.group_form.as_mut() {
                            form.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let _ = window;
        cx.notify();
    }

    /// Adds a wish to a group, or takes it out again. The capability replaces the whole membership,
    /// so the list is rebuilt from the group as it was last read.
    fn toggle_group_member(&mut self, group: WishGroup, item_id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        let joining = !group.contains(&item_id);
        let mut ids = group.item_ids();
        if joining {
            ids.push(item_id.clone());
        } else {
            ids.retain(|id| id != &item_id);
        }
        self.group_busy = Some(group.id.clone());
        self.wish_error = None;
        let group_id = group.id.clone();
        let group_title = group.title.clone();
        let work =
            shell::run(async move { shell.inventory_set_wish_group_items(&group_id, &ids).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        page.wish_notice = Some(if joining {
                            format!("Added to \"{group_title}\".")
                        } else {
                            format!("Removed from \"{group_title}\".")
                        });
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Takes a group off the list. Its wishes stay in the wishlist.
    fn remove_group(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        if self.group_busy.is_some() {
            return;
        }
        self.group_busy = Some(id.clone());
        self.confirm_remove_group = None;
        self.wish_error = None;
        let write_id = id.clone();
        let work = shell::run(async move { shell.inventory_delete_wish_group(&write_id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => {
                        if page.selected_group.as_deref() == Some(id.as_str()) {
                            page.selected_group = None;
                        }
                        page.wish_notice =
                            Some("Group removed. Its wishes stay on the list.".to_owned());
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_wish_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wish_form.is_some() {
            self.wish_form = None;
            cx.notify();
            return;
        }
        self.edit_form = None;
        let definitions = [
            ("label", "What do you want?", "Item or need"),
            ("price", "Estimated price", "39,90"),
            ("category", "Category", "Furniture, clothes…"),
            ("link", "Product link", "https://…"),
        ];
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label, placeholder) in definitions {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            subscriptions.push(cx.subscribe_in(&input, window, |page, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    page.create_wish(cx);
                }
            }));
            fields.push((format!("{key} · {label}"), input));
        }
        if let Some((_, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.wish_form = Some(WishForm {
            fields,
            kind: "piece".to_owned(),
            state: if self.tab == InventoryTab::Wishlist {
                InventoryState::Wanted
            } else {
                InventoryState::Owned
            },
            error: None,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        cx.notify();
    }

    fn create_wish(&mut self, cx: &mut Context<Self>) {
        let (Some(shell), Some(form)) = (self.repo.shell().ok(), self.wish_form.as_ref()) else {
            return;
        };
        if self.wish_busy.is_some() {
            return;
        }
        let label = form
            .value("label · What do you want?", cx)
            .trim()
            .to_owned();
        if label.is_empty() {
            if let Some(form) = self.wish_form.as_mut() {
                form.error = Some("Give the wish a name.".to_owned());
            }
            cx.notify();
            return;
        }
        let price_text = form.value("price · Estimated price", cx);
        let price_cents = if price_text.trim().is_empty() {
            None
        } else {
            match sjel_gpui_core::capture::cents_from(&price_text) {
                Some(cents) if cents >= 0 => Some(cents),
                _ => {
                    if let Some(form) = self.wish_form.as_mut() {
                        form.error = Some("Enter a price like 39,90.".to_owned());
                    }
                    cx.notify();
                    return;
                }
            }
        };
        let category = nonempty(form.value("category · Category", cx));
        let link = nonempty(form.value("link · Product link", cx));
        let slug = sjel_gpui_core::capture::slug(&label);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let wish = NewInventoryItem {
            id: format!("{slug}-{timestamp}"),
            kind: form.kind.clone(),
            label,
            state: form.state,
            note: "Created in GPUI Inventory".to_owned(),
            price_cents,
            category,
            link,
        };
        self.wish_busy = Some("new".to_owned());
        self.wish_notice = None;
        self.wish_error = None;
        if let Some(form) = self.wish_form.as_mut() {
            form.error = None;
        }
        let created_state = wish.state;
        let work = shell::run(async move { shell.inventory_create_item(&wish).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_busy = None;
                match result {
                    Ok(()) => {
                        page.wish_form = None;
                        page.wish_notice = Some(match created_state {
                            InventoryState::Owned => "Added to inventory.".to_owned(),
                            InventoryState::Wanted => "Added to the wishlist.".to_owned(),
                            InventoryState::Gone => "Added as a gone item.".to_owned(),
                        });
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.wish_form.as_mut() {
                            form.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_wish_state(&mut self, id: String, state: InventoryState, cx: &mut Context<Self>) {
        if self.wish_busy.is_some() {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.wish_busy = Some(id.clone());
        self.wish_notice = None;
        self.wish_error = None;
        self.confirm_remove_wish = None;
        let write_id = id.clone();
        let note = match state {
            InventoryState::Owned => "Marked bought in GPUI Wishlist",
            InventoryState::Gone => "Removed from GPUI Wishlist",
            InventoryState::Wanted => "Restored from GPUI Wishlist",
        };
        let work = shell::run(async move {
            shell
                .inventory_set_state(&write_id, state, Some(note))
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.wish_busy = None;
                match result {
                    Ok(_) => {
                        page.wish_notice = Some(match state {
                            InventoryState::Owned => "Marked as bought.".to_owned(),
                            InventoryState::Gone => "Removed from the open wishlist.".to_owned(),
                            InventoryState::Wanted => "Restored to the wishlist.".to_owned(),
                        });
                        page.load(cx);
                    }
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_item_edit(&mut self, item: InventoryItem, window: &mut Window, cx: &mut Context<Self>) {
        self.wish_form = None;
        let definitions = [
            ("label", "Name", item.label.clone(), "Item name".to_owned()),
            (
                "price_cents",
                "Price",
                item.price_cents
                    .map(|cents| format!("{:.2}", cents as f64 / 100.))
                    .unwrap_or_default(),
                "39,90".to_owned(),
            ),
            (
                "category",
                "Category",
                item.category.clone().unwrap_or_default(),
                "Category".to_owned(),
            ),
            (
                "priority",
                "Priority",
                item.priority.clone().unwrap_or_default(),
                "required, recommended…".to_owned(),
            ),
            (
                "width",
                "Width cm",
                item.width
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "depth",
                "Depth cm",
                item.depth
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "height",
                "Height cm",
                item.height
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "cm".to_owned(),
            ),
            (
                "size",
                "Size",
                item.size.clone().unwrap_or_default(),
                "M".to_owned(),
            ),
            (
                "color",
                "Colour",
                item.color.clone().unwrap_or_default(),
                "Colour".to_owned(),
            ),
            (
                "link",
                "Product link",
                item.link.clone().unwrap_or_default(),
                "https://…".to_owned(),
            ),
            (
                "notes",
                "Note",
                item.notes.clone().unwrap_or_default(),
                "Optional note".to_owned(),
            ),
        ];
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label, value, placeholder) in definitions {
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder);
                input.set_value(value, window, cx);
                input
            });
            subscriptions.push(
                cx.subscribe_in(&input, window, |page, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        page.save_item_edit(window, cx);
                    }
                }),
            );
            fields.push((key.to_owned(), label.to_owned(), input));
        }
        if let Some((_, _, input)) = fields.first() {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.edit_form = Some(InventoryEditForm {
            item,
            fields,
            error: None,
            saving: false,
            _subscriptions: subscriptions,
        });
        self.wish_notice = None;
        self.wish_error = None;
        cx.notify();
    }

    fn save_item_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(shell), Some(form)) = (self.repo.shell(), self.edit_form.as_ref()) else {
            return;
        };
        if form.saving {
            return;
        }
        let id = form.item.id.clone();
        let revision = form.item.revision;
        let label = form.value("label", cx).trim().to_owned();
        if label.is_empty() {
            if let Some(form) = self.edit_form.as_mut() {
                form.error = Some("Give the entry a name.".to_owned());
            }
            cx.notify();
            return;
        }
        let price_text = form.value("price_cents", cx);
        let price = if price_text.trim().is_empty() {
            serde_json::Value::Null
        } else {
            match sjel_gpui_core::capture::cents_from(&price_text) {
                Some(cents) if cents >= 0 => serde_json::json!(cents),
                _ => {
                    if let Some(form) = self.edit_form.as_mut() {
                        form.error = Some("Enter a price like 39,90 or leave it blank.".to_owned());
                    }
                    cx.notify();
                    return;
                }
            }
        };
        let dimension = |key: &str| -> Result<serde_json::Value, String> {
            let text = form.value(key, cx);
            if text.trim().is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                text.trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .map_or_else(
                        || {
                            Err("Dimensions must be positive whole centimeters, or blank."
                                .to_owned())
                        },
                        |value| Ok(serde_json::json!(value)),
                    )
            }
        };
        let (width, depth, height) =
            match (dimension("width"), dimension("depth"), dimension("height")) {
                (Ok(width), Ok(depth), Ok(height)) => (width, depth, height),
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                    if let Some(form) = self.edit_form.as_mut() {
                        form.error = Some(error);
                    }
                    cx.notify();
                    return;
                }
            };
        let optional = |key: &str| {
            nonempty(form.value(key, cx))
                .map_or(serde_json::Value::Null, |value| serde_json::json!(value))
        };
        let patch = serde_json::json!({
            "label": label,
            "price_cents": price,
            "category": optional("category"),
            "priority": optional("priority"),
            "width": width,
            "depth": depth,
            "height": height,
            "size": optional("size"),
            "color": optional("color"),
            "link": optional("link"),
            "notes": optional("notes"),
        });
        if let Some(form) = self.edit_form.as_mut() {
            form.saving = true;
            form.error = None;
        }
        self.wish_error = None;
        let work =
            shell::run(async move { shell.inventory_patch_item(&id, revision, patch).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(_) => {
                        page.edit_form = None;
                        page.wish_notice = Some("Saved inventory changes.".to_owned());
                        page.load(cx);
                    }
                    Err(error @ ShellError::Refused(409, _)) => {
                        page.edit_form = None;
                        page.wish_error = Some(format!(
                            "This entry changed elsewhere. Fresh values were loaded; reopen it before saving again. {error}"
                        ));
                        page.load(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.edit_form.as_mut() {
                            form.saving = false;
                            form.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let _ = window;
        cx.notify();
    }

    fn render_header_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let owned_count = self
            .rows
            .value
            .as_ref()
            .map(|r| {
                r.iter()
                    .filter(|row| row.state.as_deref() == Some("owned"))
                    .count()
            })
            .unwrap_or(0);
        let wishlist_count = self
            .wishlist
            .value
            .as_ref()
            .map(|w| w.items.len())
            .unwrap_or(0);
        let layouts_count = self.layouts.value.as_ref().map(|l| l.len()).unwrap_or(0);

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
                        "Owned Items",
                        InventoryTab::Owned,
                        owned_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Wishlist",
                        InventoryTab::Wishlist,
                        wishlist_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Interior Layouts",
                        InventoryTab::Interior,
                        layouts_count,
                        palette,
                        cx,
                    ))
                    .child(self.render_tab_button(
                        "Mail review",
                        InventoryTab::MailReview,
                        self.mail_proposals.value.as_ref().map_or(0, |items| {
                            items.iter().filter(|p| p.decision.is_none()).count()
                        }),
                        palette,
                        cx,
                    )),
            )
            .child(
                div().flex().items_center().gap_3().child(
                    Button::new("refresh-inventory")
                        .small()
                        .ghost()
                        .label("Refresh")
                        .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                ),
            )
            .into_any_element()
    }

    fn render_tab_button(
        &self,
        label: &str,
        tab: InventoryTab,
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

    fn render_owned_view(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(all_rows) = self.rows.value.clone() else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading inventory…")
                .into_any_element();
        };

        let owned_rows: Vec<_> = all_rows
            .iter()
            .filter(|r| r.state.as_deref() == Some("owned"))
            .cloned()
            .collect();

        // Compute categories
        let mut categories = BTreeSet::new();
        let mut total_cents: i64 = 0;
        for r in &owned_rows {
            categories.insert(group_of(&r.item));
            if let Some(cents) = r.item.price_cents {
                total_cents += cents;
            }
        }

        let filtered = filter_inventory(
            &owned_rows,
            &self.search_query,
            self.selected_category.as_deref(),
            Some("owned"),
        );

        let search_field = self.search_field(window, cx);

        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Inventory"),
                    )
                    .child(
                        Button::new("inventory-new-entry")
                            .small()
                            .primary()
                            .label(if self.wish_form.is_some() {
                                "Cancel"
                            } else {
                                "+ New entry"
                            })
                            .disabled(self.wish_busy.as_deref() == Some("new"))
                            .on_click(
                                cx.listener(|page, _, window, cx| page.open_wish_form(window, cx)),
                            ),
                    ),
            )
            .child(
                // Stat Cards
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.render_stat_card(
                        "Owned Assets",
                        &owned_rows.len().to_string(),
                        palette,
                    ))
                    .child(self.render_stat_card(
                        "Estimated Value",
                        &format_cents(Some(total_cents)),
                        palette,
                    ))
                    .child(self.render_stat_card(
                        "Categories",
                        &categories.len().to_string(),
                        palette,
                    )),
            )
            .child(
                // Filter Bar
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().w(px(280.)).child(Input::new(&search_field).small()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("cat-all")
                                    .small()
                                    .label("All")
                                    .when(self.selected_category.is_none(), |b| b.primary())
                                    .when(self.selected_category.is_some(), |b| b.ghost())
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.selected_category = None;
                                        cx.notify();
                                    })),
                            )
                            .children(categories.into_iter().map(|cat| {
                                let active = self.selected_category.as_deref() == Some(&cat);
                                let cat_name = cat.clone();
                                Button::new(format!("cat-{}", cat))
                                    .small()
                                    .label(cat.clone())
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.selected_category =
                                            if active { None } else { Some(cat_name.clone()) };
                                        cx.notify();
                                    }))
                            })),
                    ),
            );
        if let Some(form) = self.render_wish_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(form) = self.render_inventory_edit_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(error) = &self.wish_error {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        content
            .child(if filtered.is_empty() {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No inventory items match the current filters.")
                    .into_any_element()
            } else {
                let mut cards = Vec::new();
                for r in &filtered {
                    cards.push(self.render_item_card(&r.item, palette, cx));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .children(cards)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_stat_card(&self, label: &str, value: &str, palette: Palette) -> AnyElement {
        div()
            .flex_1()
            .p_3()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x40)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(value.to_string()),
            )
            .into_any_element()
    }

    fn render_item_card(
        &self,
        item: &InventoryItem,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = format_dimensions(item.width, item.depth, item.height);
        let price = format_cents(item.price_cents);
        let item = item.clone();

        div()
            .p_3p5()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x30)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x25)))
            .flex()
            .items_center()
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
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(rgba(tint(palette.accent, 0x20)))
                                    .text_color(rgb(palette.accent))
                                    .child(group_of(&item)),
                            )
                            .children(if !item.kind.is_empty() && item.kind != "piece" {
                                Some(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_xs()
                                        .bg(rgba(tint(palette.border, 0x25)))
                                        .text_color(rgb(palette.muted))
                                        .child(item.kind.clone()),
                                )
                            } else {
                                None
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Dim: {dim}"))
                            .children(item.size.as_ref().map(|s| format!("Size: {s}")))
                            .children(item.color.as_ref().map(|c| format!("Color: {c}")))
                            .children(item.notes.as_ref().map(|n| format!("Note: {n}"))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .child(price),
                    )
                    .children(if let Some(count) = item.count {
                        if count > 1 {
                            Some(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("qty: {count}")),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    })
                    .child(
                        Button::new(format!("inventory-edit-{}", item.id))
                            .small()
                            .ghost()
                            .label("Edit")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.open_item_edit(item.clone(), window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_wish_form(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.wish_form.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, input) in &form.fields {
            let (field, label) = key.split_once(" · ").unwrap_or((key, key));
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.to_owned()),
                    )
                    .child(Input::new(input).small())
                    .id(format!("wishlist-new-{field}")),
            );
        }
        let kind = form.kind.clone();
        let state = form.state;
        let saving = self.wish_busy.as_deref() == Some("new");
        let mut card = div()
            .p_4()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x30)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x25)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(if self.tab == InventoryTab::Wishlist {
                        "New wishlist entry"
                    } else {
                        "New inventory entry"
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("wishlist-kind-piece")
                            .small()
                            .label("Product")
                            .when(kind == "piece", |button| button.primary())
                            .when(kind != "piece", |button| button.ghost())
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(form) = page.wish_form.as_mut() {
                                    form.kind = "piece".to_owned();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("wishlist-kind-slot")
                            .small()
                            .label("Flat need")
                            .when(kind == "slot", |button| button.primary())
                            .when(kind != "slot", |button| button.ghost())
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(form) = page.wish_form.as_mut() {
                                    form.kind = "slot".to_owned();
                                }
                                cx.notify();
                            })),
                    )
                    .when(self.tab == InventoryTab::Owned, |row| {
                        row.child(
                            Button::new("inventory-new-owned")
                                .small()
                                .label("Owned")
                                .when(state == InventoryState::Owned, |button| button.primary())
                                .when(state != InventoryState::Owned, |button| button.ghost())
                                .on_click(cx.listener(|page, _, _, cx| {
                                    if let Some(form) = page.wish_form.as_mut() {
                                        form.state = InventoryState::Owned;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("inventory-new-wanted")
                                .small()
                                .label("Wanted")
                                .when(state == InventoryState::Wanted, |button| button.primary())
                                .when(state != InventoryState::Wanted, |button| button.ghost())
                                .on_click(cx.listener(|page, _, _, cx| {
                                    if let Some(form) = page.wish_form.as_mut() {
                                        form.state = InventoryState::Wanted;
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(fields);
        if let Some(error) = &form.error {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        Some(
            card.child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("wishlist-create-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .disabled(saving)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.wish_form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("wishlist-create-save")
                            .small()
                            .primary()
                            .label(if saving {
                                "Adding…"
                            } else {
                                "Add to wishlist"
                            })
                            .disabled(saving)
                            .on_click(cx.listener(|page, _, _, cx| page.create_wish(cx))),
                    ),
            )
            .into_any_element(),
        )
    }

    fn render_wishlist_view(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(wishlist) = self.wishlist.value.clone() else {
            return div()
                .p_6()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading wishlist budget…")
                .into_any_element();
        };

        let groups = self.wish_groups.value.clone().unwrap_or_default();
        // A group that has just been deleted would narrow the list to nothing, so the selection is
        // resolved against the groups that exist.
        let group = self
            .selected_group
            .as_deref()
            .and_then(|id| groups.iter().find(|group| group.id == id))
            .cloned();
        let shown = wishlist_view(&wishlist.items, group.as_ref(), &self.search_query);
        let filtered = group.is_some() || !self.search_query.trim().is_empty();

        // The banner describes what the list shows. When a group or a search narrows it, the range
        // of the whole wishlist stays visible underneath instead of disappearing.
        let budget = budget_of(shown.iter().copied());
        let whole = budget_of(wishlist.items.iter());
        let range = format!(
            "{} – {}",
            format_cents(Some(budget.low_cents)),
            format_cents(Some(budget.high_cents))
        );
        let label = match &group {
            Some(group) => format!("{} · {} open", group.title, shown.len()),
            None if filtered => format!(
                "Matching wishes · {} of {}",
                shown.len(),
                wishlist.items.len()
            ),
            None => "Estimated Total Budget Range".to_owned(),
        };
        let whole_line = filtered.then(|| {
            format!(
                "All {} open: {} – {}",
                wishlist.items.len(),
                format_cents(Some(whole.low_cents)),
                format_cents(Some(whole.high_cents))
            )
        });
        let months = wishlist
            .monthly_balance
            .as_ref()
            .and_then(|balance| months_to_pay(budget.low_cents, balance));
        let search_field = self.search_field(window, cx);
        let group_busy = self.group_busy.is_some();

        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Wishlist"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("wishlist-new-group")
                                    .small()
                                    .ghost()
                                    .label(if self.group_form.is_some() {
                                        "Cancel"
                                    } else {
                                        "+ New group"
                                    })
                                    .disabled(group_busy)
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.open_group_form(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("wishlist-new")
                                    .small()
                                    .primary()
                                    .label(if self.wish_form.is_some() {
                                        "Cancel"
                                    } else {
                                        "+ Add wish"
                                    })
                                    .disabled(self.wish_busy.as_deref() == Some("new"))
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.open_wish_form(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().w(px(280.)).child(Input::new(&search_field).small()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("group-all")
                                    .small()
                                    .label(format!("All ({})", wishlist.items.len()))
                                    .when(group.is_none(), |b| b.primary())
                                    .when(group.is_some(), |b| b.ghost())
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.selected_group = None;
                                        cx.notify();
                                    })),
                            )
                            .children(groups.iter().map(|candidate| {
                                let active =
                                    group.as_ref().is_some_and(|group| group.id == candidate.id);
                                let id = candidate.id.clone();
                                Button::new(format!("group-{}", candidate.id))
                                    .small()
                                    .label(format!(
                                        "{} ({})",
                                        candidate.title, candidate.items_count
                                    ))
                                    .when(active, |b| b.primary())
                                    .when(!active, |b| b.ghost())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.selected_group =
                                            if active { None } else { Some(id.clone()) };
                                        cx.notify();
                                    }))
                            })),
                    ),
            );
        if let Some(form) = self.render_group_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(group) = &group {
            let mut row = div().flex().items_center().justify_between().gap_3().child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(if group.constraint_text.trim().is_empty() {
                        "No constraint set.".to_owned()
                    } else {
                        group.constraint_text.clone()
                    }),
            );
            let id = group.id.clone();
            let confirming = self.confirm_remove_group.as_deref() == Some(id.as_str());
            row = row.child(
                Button::new(format!("group-remove-{id}"))
                    .small()
                    .ghost()
                    .label(if confirming {
                        "Confirm removal"
                    } else {
                        "Remove group"
                    })
                    .disabled(group_busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if page.confirm_remove_group.as_deref() == Some(id.as_str()) {
                            page.remove_group(id.clone(), cx);
                        } else {
                            page.confirm_remove_group = Some(id.clone());
                            cx.notify();
                        }
                    })),
            );
            content = content.child(row);
        }
        if let Some(form) = self.render_wish_form(palette, cx) {
            content = content.child(form);
        }
        if let Some(notice) = &self.wish_notice {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(notice.clone()),
            );
        }
        if let Some(error) = &self.wish_error {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        content
            .when_some(
                self.render_inventory_edit_form(palette, cx),
                |content, form| content.child(form),
            )
            .child(
                // Budget banner
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgba(tint(palette.raised, 0x40)))
                    .border_1()
                    .border_color(rgba(tint(palette.border, 0x30)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.accent))
                                    .child(range),
                            )
                            .children((budget.unpriced > 0).then(|| {
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.amber))
                                    .child(format!(
                                        "{} unpriced items (budget is a floor)",
                                        budget.unpriced
                                    ))
                            }))
                            .children(whole_line.map(|line| {
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(line)
                            })),
                    )
                    .children(wishlist.monthly_balance.as_ref().map(|b| {
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "Median Surplus: {}/mo",
                                        format_cents(Some(b.median_cents))
                                    )),
                            )
                            .children(months.map(|m| {
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.green))
                                    .child(format!("{:.1} months to fund", m))
                            }))
                    })),
            )
            .child(if shown.is_empty() {
                div()
                    .p_8()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(match (&group, filtered) {
                        (Some(group), _) if wishlist.items.is_empty() => {
                            "No open wishlist items.".to_owned()
                        }
                        (Some(group), _) => format!(
                            "Nothing from the open wishlist is in \"{}\" yet. Pick Group on a wish to add it.",
                            group.title
                        ),
                        (None, true) => "No wish matches the search.".to_owned(),
                        (None, false) => "No open wishlist items.".to_owned(),
                    })
                    .into_any_element()
            } else {
                let mut cards = Vec::new();
                for item in &shown {
                    cards.push(self.render_wish_item(item, &groups, palette, cx));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .children(cards)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_group_form(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.group_form.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, label, input) in &form.fields {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(Input::new(input).small())
                    .id(format!("group-{key}")),
            );
        }
        Some(
            div()
                .p_4()
                .rounded_lg()
                .bg(rgba(tint(palette.raised, 0x30)))
                .border_1()
                .border_color(rgba(tint(palette.border, 0x25)))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("New wish group"),
                )
                .child(fields)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("group-create-save")
                                .small()
                                .primary()
                                .label("Create group")
                                .on_click(
                                    cx.listener(|page, _, window, cx| {
                                        page.create_group(window, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("group-create-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.group_form = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_inventory_edit_form(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.edit_form.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, label, input) in &form.fields {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(Input::new(input).small())
                    .id(format!("inventory-edit-{key}")),
            );
        }
        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let saving = form.saving;
        Some(
            div()
                .p_4()
                .rounded_lg()
                .bg(rgba(tint(palette.raised, 0x30)))
                .border_1()
                .border_color(rgba(tint(palette.border, 0x25)))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(format!("Edit {}", form.item.label)),
                        )
                        .child(
                            Button::new("inventory-edit-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(saving)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.edit_form = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(fields)
                .child(
                    div().flex().justify_end().child(
                        Button::new("inventory-edit-save")
                            .small()
                            .primary()
                            .label(if saving { "Saving…" } else { "Save changes" })
                            .disabled(saving)
                            .on_click(
                                cx.listener(|page, _, window, cx| page.save_item_edit(window, cx)),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_wish_item(
        &self,
        item: &InventoryItem,
        groups: &[WishGroup],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cost_range = if let Some(price) = item.price_cents {
            format_cents(Some(price))
        } else if item.cost_min_cents.is_some() || item.cost_max_cents.is_some() {
            format!(
                "{} – {}",
                format_cents(item.cost_min_cents),
                format_cents(item.cost_max_cents)
            )
        } else {
            "unpriced".to_string()
        };

        let dim = format_dimensions(item.width, item.depth, item.height);

        let id = item.id.clone();
        let group_id = id.clone();
        let bought_id = id.clone();
        let edit_item = item.clone();
        let remove_id = id.clone();
        let busy = self.wish_busy.as_deref() == Some(id.as_str());
        let confirming_remove = self.confirm_remove_wish.as_deref() == Some(id.as_str());
        let picker_open = self.group_menu.as_deref() == Some(id.as_str());
        let mut card = div()
            .p_3p5()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x30)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x25)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
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
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(item.label.clone()),
                                    )
                                    .children(item.priority.as_ref().map(|pri| {
                                        let color = match pri.as_str() {
                                            "required" | "decided" => palette.red,
                                            "recommended" => palette.amber,
                                            _ => palette.accent,
                                        };
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .text_xs()
                                            .bg(rgba(tint(color, 0x20)))
                                            .text_color(rgb(color))
                                            .child(pri.clone())
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("Target Dim: {dim}"))
                                    .children(if !item.replaces.is_empty() {
                                        Some(format!("Replaces: {}", item.replaces.join(", ")))
                                    } else {
                                        None
                                    })
                                    .children(item.notes.as_ref().map(|n| format!("Note: {n}"))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(cost_range),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new(format!("wishlist-group-{id}"))
                                            .small()
                                            .ghost()
                                            .label("Group")
                                            .disabled(groups.is_empty())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.group_menu = if page.group_menu.as_deref()
                                                    == Some(group_id.as_str())
                                                {
                                                    None
                                                } else {
                                                    Some(group_id.clone())
                                                };
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-edit-{id}"))
                                            .small()
                                            .ghost()
                                            .label("Edit")
                                            .on_click(cx.listener(move |page, _, window, cx| {
                                                page.open_item_edit(edit_item.clone(), window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-bought-{id}"))
                                            .small()
                                            .primary()
                                            .label(if busy { "Saving…" } else { "Mark bought" })
                                            .disabled(self.wish_busy.is_some())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.set_wish_state(
                                                    bought_id.clone(),
                                                    InventoryState::Owned,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        Button::new(format!("wishlist-remove-{id}"))
                                            .small()
                                            .ghost()
                                            .label(if confirming_remove {
                                                "Confirm removal"
                                            } else {
                                                "No longer wanted"
                                            })
                                            .disabled(self.wish_busy.is_some())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                if page.confirm_remove_wish.as_deref()
                                                    == Some(remove_id.as_str())
                                                {
                                                    page.set_wish_state(
                                                        remove_id.clone(),
                                                        InventoryState::Gone,
                                                        cx,
                                                    );
                                                } else {
                                                    page.confirm_remove_wish =
                                                        Some(remove_id.clone());
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .when(confirming_remove, |row| {
                                        row.child(
                                            Button::new(format!("wishlist-remove-cancel-{id}"))
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(move |page, _, _, cx| {
                                                    page.confirm_remove_wish = None;
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                    ),
            );
        if picker_open {
            card = card.child(self.render_group_picker(item, groups, palette, cx));
        }
        card.into_any_element()
    }

    /// The picker under one wish: one chip per group, filled where this wish is already a member.
    fn render_group_picker(
        &self,
        item: &InventoryItem,
        groups: &[WishGroup],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.group_busy.is_some();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(rgba(tint(palette.border, 0x22)))
            .children(groups.iter().map(|candidate| {
                let member = candidate.contains(&item.id);
                let group = candidate.clone();
                let item_id = item.id.clone();
                Button::new(format!("wish-group-{}-{}", candidate.id, item.id))
                    .small()
                    .label(if member {
                        format!("✓ {}", candidate.title)
                    } else {
                        candidate.title.clone()
                    })
                    .when(member, |button| button.primary())
                    .when(!member, |button| button.ghost())
                    .disabled(busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.toggle_group_member(group.clone(), item_id.clone(), cx);
                    }))
            }))
            .into_any_element()
    }

    fn render_interior_view(&self, palette: Palette) -> AnyElement {
        let layouts = match self.layouts.value.as_ref() {
            None if self.layouts.loading => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Loading interior layouts…")
                .into_any_element(),
            None => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Interior layouts are unavailable.")
                .into_any_element(),
            Some(layouts) if layouts.is_empty() => div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("No interior layouts found yet.")
                .into_any_element(),
            Some(layouts) => {
                let cards = layouts
                    .iter()
                    .map(|layout| self.render_layout_card(layout, palette))
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(cards)
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.roomplan_history.clone())
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Layouts"),
            )
            .child(layouts)
            .into_any_element()
    }

    fn render_layout_card(&self, l: &InteriorLayoutSummary, palette: Palette) -> AnyElement {
        let status_color = if l.pass {
            palette.green
        } else if l.hard > 0 {
            palette.red
        } else {
            palette.amber
        };

        let status_text = if l.pass {
            "Pass".to_string()
        } else if l.hard > 0 {
            format!("{} hard violations", l.hard)
        } else {
            format!("{} soft warnings", l.soft)
        };

        div()
            .p_4()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x40)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .flex()
            .items_center()
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
                                    .child(l.name.clone()),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .text_xs()
                                    .bg(rgba(tint(status_color, 0x20)))
                                    .text_color(rgb(status_color))
                                    .child(status_text),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Layout ID: {}", l.id)),
                    ),
            )
            .children(l.occupied_m2.map(|area| {
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(format!("{:.1} m² occupied", area))
            }))
            .into_any_element()
    }
}

impl Render for InventoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(WindowAppearance::Dark));
        self.roomplan_history
            .update(cx, |history, cx| history.set_palette(palette, cx));

        div()
            .id("inventory-page")
            .key_context("InventoryView")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|page, _: &ShowOwned, _, cx| page.set_tab(InventoryTab::Owned, cx)),
            )
            .on_action(
                cx.listener(|page, _: &ShowWishlist, _, cx| {
                    page.set_tab(InventoryTab::Wishlist, cx)
                }),
            )
            .on_action(
                cx.listener(|page, _: &ShowInterior, _, cx| {
                    page.set_tab(InventoryTab::Interior, cx)
                }),
            )
            .on_action(cx.listener(|page, _: &ShowMailReview, _, cx| {
                page.set_tab(InventoryTab::MailReview, cx)
            }))
            .on_action(cx.listener(|page, _: &RefreshInventory, _, cx| page.refresh(cx)))
            .on_action(
                cx.listener(|page, _: &FocusSearch, window, cx| page.focus_search(window, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .child(self.render_header_bar(palette, cx))
            .child(
                div()
                    .id("inventory-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(match self.tab {
                        InventoryTab::Owned => self.render_owned_view(palette, window, cx),
                        InventoryTab::Wishlist => self.render_wishlist_view(palette, window, cx),
                        InventoryTab::Interior => self.render_interior_view(palette),
                        InventoryTab::MailReview => self.render_mail_review(palette, cx),
                    }),
            )
    }
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn test_inventory_tab_switching(cx: &mut gpui::TestAppContext) {
        let page = cx.new(|cx| {
            let repo = Repo::new(Err(ShellError::NotSetUp));
            InventoryPage::new(repo, cx)
        });

        page.update(cx, |page, cx| {
            assert_eq!(page.tab, InventoryTab::Owned);
            page.set_tab(InventoryTab::Wishlist, cx);
            assert_eq!(page.tab, InventoryTab::Wishlist);
            page.set_tab(InventoryTab::Interior, cx);
            assert_eq!(page.tab, InventoryTab::Interior);
            page.set_tab(InventoryTab::MailReview, cx);
            assert_eq!(page.tab, InventoryTab::MailReview);
        });
    }
}
