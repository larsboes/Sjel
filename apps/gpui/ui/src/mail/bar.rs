//! Action bar for mail triage bulk operations and confirmations.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgba, AnyElement, Context, FontWeight, SharedString,
};
use sjel_gpui_core::mail::{MailAction, CATEGORIES};

use crate::theme::Palette;

use super::actions::{Cancel, Confirm};
use super::Mail;

impl Mail {
    pub(super) fn action_bar(
        &self,
        targets: &[String],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some((action, ids)) = &self.pending {
            let question = action.confirmation(ids.len()).unwrap_or_default();
            return div()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .child(div().flex_1().text_sm().child(question))
                .child(
                    Button::new("mail-cancel")
                        .small()
                        .ghost()
                        .label("Cancel · Esc")
                        .on_click(
                            cx.listener(|view, _, window, cx| view.on_cancel(&Cancel, window, cx)),
                        ),
                )
                .child(
                    Button::new("mail-confirm")
                        .small()
                        .primary()
                        .label("Confirm · ↩")
                        .on_click(
                            cx.listener(|view, _, window, cx| {
                                view.on_confirm(&Confirm, window, cx)
                            }),
                        ),
                )
                .into_any_element();
        }
        let none = targets.is_empty() || self.busy;
        let action =
            |id: &'static str, label: &'static str, action: MailAction, cx: &mut Context<Self>| {
                Button::new(id)
                    .small()
                    .ghost()
                    .label(label)
                    .disabled(none)
                    .on_click(cx.listener(move |view, _, _, cx| view.request(action.clone(), cx)))
            };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .bg(rgba(palette.raised))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .mr_2()
                    .child(format!("{} selected", self.selected.len())),
            )
            .children(CATEGORIES.iter().map(|(key, label)| {
                Button::new(SharedString::from(format!("mail-set-{key}")))
                    .xsmall()
                    .ghost()
                    .label(format!("→ {label}"))
                    .disabled(none)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.request(MailAction::Categorize((*key).to_owned()), cx)
                    }))
            }))
            .child(action(
                "mail-bulk-dismiss",
                "Dismiss",
                MailAction::Dismiss,
                cx,
            ))
            .child(action(
                "mail-bulk-waiting",
                "Waiting",
                MailAction::Waiting,
                cx,
            ))
            .child(action(
                "mail-bulk-archive",
                "Archive",
                MailAction::Archive,
                cx,
            ))
            .child(action("mail-bulk-trash", "Trash", MailAction::Trash, cx))
            .child(action(
                "mail-bulk-secret",
                "Mark Secret",
                MailAction::SetDataClass("c3".into()),
                cx,
            ))
            .into_any_element()
    }
}
