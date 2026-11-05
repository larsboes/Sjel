//! Google client JSON drag-and-drop, OAuth consent flow, and credential card.

use gpui::ExternalPaths;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString,
};
use sjel_gpui_core::accounts::CredentialRow;

use super::Accounts;
use crate::primitives::note;
use crate::theme::Palette;

impl Accounts {
    pub(crate) fn drop_file(&mut self, id: &str, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let Some(path) = paths.paths().iter().find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        }) else {
            self.outcome.insert(
                id.to_owned(),
                (false, "Drop the client JSON file (.json).".into()),
            );
            cx.notify();
            return;
        };
        self.dropped.insert(id.to_owned(), path.clone());
        self.outcome.remove(id);
        cx.notify();
    }

    pub(crate) fn connect(&mut self, row: CredentialRow, cx: &mut Context<Self>) {
        let Some(file) = self.dropped.get(&row.id).cloned() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(row.id.clone());
        self.outcome.insert(
            row.id.clone(),
            (true, "Waiting for Google in your browser…".into()),
        );
        cx.notify();
        let id = row.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sjel_gpui_core::accounts::connect_google(&row, &file) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = None;
                let line = match &result {
                    Ok(()) => (
                        true,
                        "Connected. The grant is in the OS keychain.".to_owned(),
                    ),
                    Err(error) => (false, error.clone()),
                };
                if result.is_ok() {
                    view.dropped.remove(&id);
                }
                view.outcome.insert(id, line);
                view.refresh(cx);
            });
        })
        .detach();
    }

    pub(crate) fn move_to_keychain(&mut self, row: CredentialRow, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(row.id.clone());
        self.outcome.insert(
            row.id.clone(),
            (true, "Moving the grant to the keychain…".into()),
        );
        cx.notify();
        let id = row.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sjel_gpui_core::accounts::move_to_keychain(&row) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = None;
                let line = match result {
                    Ok(()) => (
                        true,
                        "Moved. The grant is in the keychain and the env file no longer holds it."
                            .to_owned(),
                    ),
                    Err(error) => (false, error),
                };
                view.outcome.insert(id, line);
                view.refresh(cx);
            });
        })
        .detach();
    }

    pub(crate) fn card(&self, row: &CredentialRow, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = &row.error {
            return div()
                .px_4()
                .py_3()
                .child(note(format!("{}: {error}", row.capability), palette.red))
                .into_any_element();
        }
        let id = row.id.clone();
        let busy = self.busy.as_deref() == Some(row.id.as_str());
        let dropped = self.dropped.get(&row.id);
        let state_color = if row.in_keychain {
            palette.green
        } else {
            palette.amber
        };
        let drop_label: SharedString = match dropped {
            Some(path) => path
                .file_name()
                .map(|n| format!("Ready: {}", n.to_string_lossy()))
                .unwrap_or_else(|| "Ready".into())
                .into(),
            None => "Drop the client JSON file here".into(),
        };
        let drop_zone = div()
            .id(SharedString::from(format!("drop-{}", row.id)))
            .flex_1()
            .min_w(px(220.))
            .px_3()
            .py_3()
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(rgba(palette.border))
            .text_sm()
            .text_color(rgb(if dropped.is_some() {
                palette.text
            } else {
                palette.muted
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(rgba(palette.raised)))
            .on_drop(cx.listener({
                let id = id.clone();
                move |view, paths: &ExternalPaths, _, cx| view.drop_file(&id, paths, cx)
            }))
            .child(drop_label);
        let connect = Button::new(SharedString::from(format!("connect-{}", row.id)))
            .primary()
            .small()
            .label(if busy { "Connecting…" } else { "Connect" })
            .disabled(dropped.is_none() || self.busy.is_some())
            .on_click(cx.listener({
                let row = row.clone();
                move |view, _, _, cx| view.connect(row.clone(), cx)
            }));
        // Offered only when the old env file holds the grant and the keychain does not.
        let move_button = (row.env_file && !row.in_keychain).then(|| {
            Button::new(SharedString::from(format!("move-{}", row.id)))
                .outline()
                .small()
                .label("Move to keychain")
                .disabled(self.busy.is_some())
                .on_click(cx.listener({
                    let row = row.clone();
                    move |view, _, _, cx| view.move_to_keychain(row.clone(), cx)
                }))
        });
        let steps = row.steps.iter().enumerate().map(|(i, step)| {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(SharedString::from(format!("{}. {step}", i + 1)))
        });
        div()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_sm().child(SharedString::from(row.label.clone())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(state_color))
                            .child(row.state()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(SharedString::from(format!(
                        "{} · {} · {}",
                        row.capability,
                        row.id,
                        row.scopes.join(", ")
                    ))),
            )
            .children(steps)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(drop_zone)
                    .child(connect)
                    .children(move_button),
            )
            .children(self.outcome.get(&row.id).map(|(ok, line)| {
                note(line.clone(), if *ok { palette.muted } else { palette.red })
            }))
            .into_any_element()
    }
}
