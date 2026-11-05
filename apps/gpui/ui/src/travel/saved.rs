//! Saved items, activities, bookings, and stay candidates board.
use super::forms::{FormKind, Remove};
use super::{urlencode, Travel};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context};
use gpui_kit::component::button::ButtonVariants;
use sjel_gpui_core::travel::Item;

impl Travel {
    pub(super) fn saved_items(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(saved) = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
        else {
            return page
                .child("Saved activities, stays and bookings appear when trip details have loaded.")
                .into_any_element();
        };
        page = page
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(self.form_button(
                        "add-booking",
                        "Add booking",
                        FormKind::Booking(None),
                        cx,
                    ))
                    .children(self.form_button(
                        "add-stay",
                        "Add stay candidate",
                        FormKind::Stay(None),
                        cx,
                    )),
            )
            .children(self.form_card(&FormKind::Booking(None), cx))
            .children(self.form_card(&FormKind::Stay(None), cx));
        let mut stays: Vec<_> = saved
            .details
            .items
            .iter()
            .filter(|item| item.item_type == "stay")
            .collect();
        sjel_gpui_core::travel::sort_stay_candidates(&mut stays);
        if !stays.is_empty() {
            let mut board = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .border_1()
                .border_color(rgba(self.palette.border))
                .bg(rgba(self.palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("Stay ranking board"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(self.palette.muted))
                                .child(format!("{} candidate(s)", stays.len())),
                        ),
                );
            for item in &stays {
                board = board.child(self.item_row(item, cx));
            }
            page = page.child(board);
        }
        let mut items: Vec<_> = saved
            .details
            .items
            .iter()
            .filter(|item| item.item_type != "stay")
            .collect();
        items.sort_by(|a, b| {
            a.day
                .is_none()
                .cmp(&b.day.is_none())
                .then(a.day.cmp(&b.day))
                .then(a.title.cmp(&b.title))
        });
        if items.is_empty() && stays.is_empty() {
            return page
                .child("No saved activities, stays or bookings for this trip.")
                .into_any_element();
        }
        let mut day = None;
        for item in items {
            if day != Some(&item.day) {
                day = Some(&item.day);
                page = page.child(
                    div()
                        .text_sm()
                        .text_color(rgb(self.palette.muted))
                        .child(item.day.clone().unwrap_or("No date assigned".into())),
                );
            }
            page = page.child(self.item_row(item, cx));
        }
        page.into_any_element()
    }

    pub(super) fn item_row(&self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let editing = if item.item_type == "stay" {
            FormKind::Stay(Some(item.id.clone()))
        } else {
            FormKind::Booking(Some(item.id.clone()))
        };
        if let Some(card) = self.form_card(&editing, cx) {
            return card;
        }
        let mut badges = div().flex().flex_wrap().items_center().gap_2().child(
            div()
                .px_2()
                .py(px(1.5))
                .rounded(px(4.))
                .bg(rgba(self.palette.raised))
                .border_1()
                .border_color(rgba(self.palette.border))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(self.palette.muted))
                .child(item.item_type.replace('_', " ").to_uppercase()),
        );
        if item.item_type == "stay" {
            let rank_label = item
                .stay_rank()
                .map_or_else(|| "Unranked".to_owned(), |r| format!("#{r}"));
            badges = badges.child(
                div()
                    .px_2()
                    .py(px(1.5))
                    .rounded(px(4.))
                    .bg(rgba(crate::theme::tint(self.palette.accent, 0x18)))
                    .border_1()
                    .border_color(rgba(crate::theme::tint(self.palette.accent, 0x35)))
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(self.palette.accent))
                    .child(rank_label),
            );
            if item.is_stay_booked() {
                badges = badges.child(
                    div()
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(rgba(crate::theme::tint(self.palette.green, 0x18)))
                        .border_1()
                        .border_color(rgba(crate::theme::tint(self.palette.green, 0x35)))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(self.palette.green))
                        .child("Booked"),
                );
            }
        }
        if let Some(price) = item.price_label() {
            badges = badges.child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(self.palette.text))
                    .child(price),
            );
        }
        if let Some(rating) = item.rating_label() {
            badges = badges.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(format!("Rating {rating}")),
            );
        }
        if let Some(code) = item.booking_code() {
            badges = badges.child(
                div()
                    .px_2()
                    .py(px(1.5))
                    .rounded(px(4.))
                    .bg(rgba(crate::theme::tint(self.palette.accent, 0x18)))
                    .border_1()
                    .border_color(rgba(crate::theme::tint(self.palette.accent, 0x35)))
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(self.palette.accent))
                    .child(format!("Code: {code}")),
            );
        }
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(rgba(self.palette.border))
            .child(badges)
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(item.title.clone()),
            );
        for (label, value) in item.fields() {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(format!("{label}: {value}")),
            );
        }
        let mut actions = div().flex().flex_wrap().gap_2();
        if item.item_type == "stay" && self.writer().is_some() {
            let current_rank = item.stay_rank();
            for r in [1_i64, 2, 3] {
                let item_id = item.id.clone();
                let active = current_rank == Some(r);
                actions = actions.child(
                    self.button(gpui::SharedString::from(format!(
                        "stay-rank-{}-{r}",
                        item.id
                    )))
                    .when(active, |b| b.primary())
                    .when(!active, |b| b.ghost())
                    .label(format!("#{r}"))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let next = if active { None } else { Some(r) };
                        view.set_stay_rank(item_id.clone(), next, window, cx);
                    })),
                );
            }
            if let Some(r) = current_rank {
                if r > 1 {
                    let item_id = item.id.clone();
                    actions = actions.child(
                        self.button(gpui::SharedString::from(format!("stay-up-{}", item.id)))
                            .ghost()
                            .label("↑")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.set_stay_rank(item_id.clone(), Some(r - 1), window, cx);
                            })),
                    );
                }
                let item_id_down = item.id.clone();
                actions = actions.child(
                    self.button(gpui::SharedString::from(format!("stay-down-{}", item.id)))
                        .ghost()
                        .label("↓")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.set_stay_rank(item_id_down.clone(), Some(r + 1), window, cx);
                        })),
                );
                let item_id_clear = item.id.clone();
                actions = actions.child(
                    self.button(gpui::SharedString::from(format!("stay-unrank-{}", item.id)))
                        .ghost()
                        .label("Unrank")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.set_stay_rank(item_id_clear.clone(), None, window, cx);
                        })),
                );
            }
        }
        if let Some(code) = item.booking_code() {
            let code_str = code.to_owned();
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("copy-code-{}", item.id)))
                    .primary()
                    .label(format!("Copy {code_str}"))
                    .on_click(cx.listener(move |_view, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(code_str.clone()));
                    })),
            );
        }
        if let Some(address) = item
            .payload
            .get("address")
            .and_then(serde_json::Value::as_str)
        {
            let addr = address.to_owned();
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("map-addr-{}", item.id)))
                    .ghost()
                    .label("Apple Maps")
                    .on_click(move |_, _, cx| {
                        cx.open_url(&format!("https://maps.apple.com/?q={}", urlencode(&addr)));
                    }),
            );
        }
        if let Some(url) = item.source_url() {
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("open-item-{}", item.id)))
                    .ghost()
                    .label("Source")
                    .on_click(move |_, _, cx| cx.open_url(url.as_str())),
            );
        }
        if item.item_type == "booking" || item.item_type == "stay" {
            actions = actions.children(self.form_button(
                gpui::SharedString::from(format!("edit-item-{}", item.id)),
                "Edit",
                editing,
                cx,
            ));
        }
        row.child(actions.children(self.remove_button(Remove::Item(item.id.clone()), cx)))
            .into_any_element()
    }
}
