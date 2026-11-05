//! Inventory mail review and match decision workflow.

use gpui::{div, prelude::*, rgb, rgba, AnyElement, Context, Entity, Subscription, Window};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::inventory::{MailMatchSnapshot, MailProposal};
use sjel_gpui_core::shell::{self, ShellError};

use crate::theme::{tint, Palette};

use super::InventoryPage;

pub(crate) struct MailReviewForm {
    pub(super) proposal: MailProposal,
    pub(super) title: Entity<InputState>,
    pub(super) item_id: Entity<InputState>,
    pub(super) snapshot: Option<MailMatchSnapshot>,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl InventoryPage {
    pub(super) fn load_mail(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn open_mail(
        &mut self,
        proposal: MailProposal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn mail_snapshot(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn decide_mail(&mut self, action: &'static str, cx: &mut Context<Self>) {
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

    pub(super) fn render_mail_review(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
}
