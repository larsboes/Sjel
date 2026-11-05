use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, ListOffset, ListState, Pixels,
    SharedString, Window,
};
use sjel_gpui_core::shell::FeedEntry;

use crate::primitives::{filter_pill, virtual_list};
use crate::theme::Palette;

use super::reading::{muted, title};
use super::types::Filter;
use super::Feed;

impl Feed {
    /// The archive as one table, newest first. A row opens the reader; its buttons decide.
    pub(crate) fn library_list(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = self.library_field(window, cx);
        self.sync_library();
        let total = self.library.value.as_ref().map_or(0, Vec::len);
        let count = |status: Option<Filter>| match status {
            None => self.library_counts[0],
            Some(Filter::New) => self.library_counts[1],
            Some(Filter::Kept) => self.library_counts[2],
            Some(Filter::Dismissed) => self.library_counts[3],
        };
        let pill = |label: &'static str, status: Option<Filter>, cx: &mut Context<Self>| {
            filter_pill(
                label,
                count(status),
                self.library_status == status,
                palette,
                cx.listener(move |view, _, _, cx| {
                    view.library_status = status;
                    view.library_rev = view.library_rev.wrapping_add(1);
                    cx.notify();
                }),
            )
        };
        let kind_button = |label: String, kind: Option<String>, cx: &mut Context<Self>| {
            let active = self.library_kind == kind;
            Button::new(SharedString::from(format!("feed-kind-{label}")))
                .small()
                .label(label)
                .when(active, |b| b.primary())
                .when(!active, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.library_kind = kind.clone();
                    view.library_rev = view.library_rev.wrapping_add(1);
                    cx.notify();
                }))
        };
        let mut kind_row =
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(kind_button(format!("All {total}"), None, cx));
        for (kind, n) in &self.library_kinds {
            kind_row = kind_row.child(kind_button(format!("{kind} {n}"), Some(kind.clone()), cx));
        }
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(pill("All", None, cx))
            .child(pill("New", Some(Filter::New), cx))
            .child(pill("Kept", Some(Filter::Kept), cx))
            .child(pill("Dismissed", Some(Filter::Dismissed), cx))
            .child(div().flex_1())
            .child(div().w(px(280.)).child(Input::new(&input).small()));

        let mut column = div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(self.title_row(cx))
            .child(toolbar)
            .child(kind_row);
        if let Some(error) = &self.library.error {
            column = column.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        } else if self.library.value.is_none() && self.library.loading {
            column = column.child(muted("Loading the archive…", palette));
        } else if self.library_order.is_empty() && !self.library.loading {
            column = column.child(muted("No entries match these filters.", palette));
        }
        if !self.library_order.is_empty() {
            // The archive is the whole history; only the rows in the window are built.
            let entity = cx.entity();
            column = column.child(virtual_list(
                "feed-library",
                &self.library_list,
                move |ix, _window, cx| {
                    entity.update(cx, |feed, cx| feed.library_row_at(ix, palette, cx))
                },
            ));
        }
        column.into_any_element()
    }

    /// One Library row, drawn from its place in the archive.
    pub(crate) fn library_row_at(&self, ix: usize, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(rows) = self.library.value.as_ref() else {
            return div().into_any_element();
        };
        let Some(entry) = self
            .library_order
            .get(ix)
            .and_then(|index| rows.get(*index))
        else {
            return div().into_any_element();
        };
        library_row(ix, entry, palette, cx)
    }
}

pub(crate) fn library_row(
    i: usize,
    entry: &FeedEntry,
    palette: Palette,
    cx: &mut Context<Feed>,
) -> AnyElement {
    let cell = |width: f32, text: String| {
        div()
            .w(px(width))
            .flex_none()
            .truncate()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(text)
    };
    // Author and lens give way first, so a narrow window still shows the title.
    let shrinking = |width: f32, text: String| cell(width, text).flex_shrink(1.).min_w_0();
    let action = |label: &'static str, status: &'static str, cx: &mut Context<Feed>| {
        let id = entry.id.clone();
        Button::new(SharedString::from(format!("feed-library-{label}-{i}")))
            .small()
            .ghost()
            .label(label)
            .on_click(cx.listener(move |view, _, _, cx| view.decide(id.clone(), status, cx)))
    };
    let actions = match entry.status.as_str() {
        "new" => div()
            .flex()
            .child(action("Keep", "keeper", cx))
            .child(action("Dismiss", "dismissed", cx)),
        _ => div().flex().child(action("Restore", "new", cx)),
    };
    let score = entry.score();
    let lens = entry
        .relevance
        .as_ref()
        .map_or("Unassigned".to_owned(), |r| r.profile_label.clone());
    let status = match entry.status.as_str() {
        "keeper" => "Kept",
        "dismissed" => "Dismissed",
        _ => "New",
    };
    let id = entry.id.clone();
    div()
        .id(("feed-library-row", i))
        .flex()
        .items_center()
        .gap_3()
        .px_2()
        .py_1()
        .rounded(px(6.))
        .when(entry.status == "dismissed", |el| el.opacity(0.5))
        .hover(move |style| style.bg(rgba(palette.raised)))
        .cursor_pointer()
        .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx)))
        .child(cell(80., entry.kind.clone()))
        .child(
            div()
                .flex_1()
                .min_w(px(180.))
                .truncate()
                .text_sm()
                .child(title(entry.title.as_deref(), &entry.url)),
        )
        .child(shrinking(120., entry.author.clone().unwrap_or_default()))
        .child(shrinking(110., lens))
        .child(cell(
            40.,
            if score > 0.0 {
                format!("{:.0}%", score * 100.0)
            } else {
                String::new()
            },
        ))
        .child(cell(70., status.to_owned()))
        .child(cell(80., entry.day.clone()))
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .flex()
                .justify_end()
                .child(actions),
        )
        .into_any_element()
}

/// The library's rows for one status (`None`: all), kind and lowercase query, newest day first
/// and best score first within a day.
#[cfg(test)]
pub(crate) fn in_library<'a>(
    rows: impl Iterator<Item = &'a FeedEntry>,
    status: Option<Filter>,
    kind: Option<&str>,
    query: &str,
) -> Vec<&'a FeedEntry> {
    let mut rows: Vec<&FeedEntry> = rows
        .filter(|e| library_matches(e, status, kind, query))
        .collect();
    rows.sort_by(|left, right| feed_order(left, right));
    rows
}

/// Whether one archive entry survives the Library tab's three filters.
pub(crate) fn library_matches(
    entry: &FeedEntry,
    status: Option<Filter>,
    kind: Option<&str>,
    query: &str,
) -> bool {
    status.is_none_or(|wanted| entry.status == wanted.status())
        && kind.is_none_or(|wanted| entry.kind == wanted)
        && (query.is_empty()
            || [
                entry.title.as_deref(),
                entry.author.as_deref(),
                Some(entry.url.as_str()),
            ]
            .into_iter()
            .flatten()
            .any(|text| text.to_lowercase().contains(query)))
}

/// Newest day first, best score first within a day.
pub(crate) fn feed_order(left: &FeedEntry, right: &FeedEntry) -> std::cmp::Ordering {
    right
        .day
        .cmp(&left.day)
        .then(right.score().total_cmp(&left.score()))
}

/// The entry at the top of a virtual list, so a rebuild can leave the reader where it was.
///
/// The anchor is the entry, not its index: a decision removes a row, and an index would slide the
/// whole list up by one under the reader.
pub(crate) fn list_anchor(
    list: &ListState,
    order: &[usize],
    rows: Option<&Vec<FeedEntry>>,
) -> Option<(String, usize, Pixels)> {
    let top = list.logical_scroll_top();
    let row = rows?.get(*order.get(top.item_ix)?)?;
    Some((row.id.clone(), top.item_ix, top.offset_in_item))
}

/// Puts a list back where [`list_anchor`] found it, once its order has been rebuilt.
pub(crate) fn restore_anchor(
    list: &ListState,
    anchor: Option<(String, usize, Pixels)>,
    order: &[usize],
    rows: Option<&Vec<FeedEntry>>,
) {
    let Some((id, anchored_at, offset)) = anchor else {
        return;
    };
    let found = order.iter().position(|index| {
        rows.and_then(|rows| rows.get(*index))
            .is_some_and(|row| row.id == id)
    });
    // The anchor can be the row a decision removed; then the same index, one row shorter.
    let item_ix = found.unwrap_or_else(|| anchored_at.min(order.len().saturating_sub(1)));
    list.scroll_to(ListOffset {
        item_ix,
        offset_in_item: offset,
    });
}

/// Each kind in the archive with its count, most common first.
#[cfg(test)]
pub(crate) fn kinds(rows: &[&FeedEntry]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for row in rows {
        match counts.iter_mut().find(|(kind, _)| *kind == row.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((row.kind.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}
