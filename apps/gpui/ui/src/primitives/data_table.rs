//! Data tables on the kit's `DataTable`: one column spec per table drives the header, the cells,
//! sorting and optional grouping, so a header and its rows cannot drift apart.
//!
//! The kit owns column resize (drag the header edge), column moves, keyboard row selection and
//! row virtualization. A page keeps one [`RowTable`] in a `TableState` entity, pushes its filtered
//! rows with [`RowTable::set_rows`], and listens for `TableEvent`s.

use std::cmp::Ordering;
use std::rc::Rc;

use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::{Sizable, Size};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FontWeight, Pixels,
    SharedString, Stateful, Window,
};

use super::tables::SortOrder;
use crate::theme::{tint, Density, Palette, ThemeContext};

/// Draws one cell of a row.
pub type CellRender<R> = Rc<dyn Fn(&R, Palette, &mut Window, &mut App) -> AnyElement>;

/// One column of a [`RowTable`].
pub struct TableColumn<R> {
    key: &'static str,
    label: &'static str,
    width: f32,
    min_width: Option<f32>,
    max_width: Option<f32>,
    right: bool,
    compare: Option<fn(&R, &R) -> Ordering>,
    text: Option<fn(&R) -> String>,
    render: CellRender<R>,
}

impl<R> TableColumn<R> {
    /// A column `width` pixels wide that draws each cell with `render`.
    pub fn new(
        key: &'static str,
        label: &'static str,
        width: f32,
        render: impl Fn(&R, Palette, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            key,
            label,
            width,
            min_width: None,
            max_width: None,
            right: false,
            compare: None,
            text: None,
            render: Rc::new(render),
        }
    }

    /// Set minimum allowed column width.
    pub fn min_width(mut self, min: f32) -> Self {
        self.min_width = Some(min);
        self
    }

    /// Set maximum allowed column width.
    pub fn max_width(mut self, max: f32) -> Self {
        self.max_width = Some(max);
        self
    }

    /// A column that draws `text(row)` as plain text, and reads it to screen readers.
    pub fn text(key: &'static str, label: &'static str, width: f32, text: fn(&R) -> String) -> Self
    where
        R: 'static,
    {
        Self::new(key, label, width, move |row, palette, _, _| {
            div()
                .truncate()
                .text_color(rgb(palette.text))
                .child(text(row))
                .into_any_element()
        })
        .label_text(text)
    }

    /// Right-align the header and the cells, for amounts and counts.
    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }

    /// Make the column sortable by `compare`, ascending.
    pub fn sort_by(mut self, compare: fn(&R, &R) -> Ordering) -> Self {
        self.compare = Some(compare);
        self
    }

    /// The text a screen reader reads, and an export writes, for a cell.
    pub fn label_text(mut self, text: fn(&R) -> String) -> Self {
        self.text = Some(text);
        self
    }
}

enum Slot {
    Group(SharedString, usize),
    Row(usize),
}

/// The delegate behind every Sjel data table.
pub struct RowTable<R> {
    columns: Vec<TableColumn<R>>,
    rows: Vec<R>,
    slots: Vec<Slot>,
    sort: Option<(&'static str, SortOrder)>,
    tie_break: Option<fn(&R, &R) -> Ordering>,
    group: Option<fn(&R) -> SharedString>,
    empty: SharedString,
    palette: Option<Palette>,
}

impl<R: 'static> RowTable<R> {
    pub fn new(columns: Vec<TableColumn<R>>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            slots: Vec::new(),
            sort: None,
            tie_break: None,
            group: None,
            empty: "Nothing to show.".into(),
            palette: None,
        }
    }

    /// Sort by the column `key` in `order` until the user picks another column.
    pub fn sorted(mut self, key: &'static str, order: SortOrder) -> Self {
        self.sort = Some((key, order));
        self
    }

    /// Order rows that compare equal on the sort column.
    pub fn tie_break(mut self, compare: fn(&R, &R) -> Ordering) -> Self {
        self.tie_break = Some(compare);
        self
    }

    /// Insert a header row before each run of rows with the same `group(row)`. Groups keep the
    /// order in which they first appear in [`Self::set_rows`]; rows sort inside their group.
    pub fn grouped_by(mut self, group: Option<fn(&R) -> SharedString>) -> Self {
        self.group = group;
        self.rebuild();
        self
    }

    /// The text shown when there are no rows.
    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty = text.into();
        self
    }

    /// Draw with `palette` instead of the window's theme.
    pub fn set_palette(&mut self, palette: Option<Palette>) {
        self.palette = palette;
    }

    /// Replace the rows, keeping the current sort.
    pub fn set_rows(&mut self, rows: Vec<R>) {
        self.rows = rows;
        self.rebuild();
    }

    /// Change grouping, keeping rows and sort.
    pub fn set_group(&mut self, group: Option<fn(&R) -> SharedString>) {
        self.group = group;
        self.rebuild();
    }

    /// All rows, in the order they were given.
    pub fn rows(&self) -> &[R] {
        &self.rows
    }

    /// The row drawn at table row `ix`, or `None` for a group header.
    pub fn row(&self, ix: usize) -> Option<&R> {
        match self.slots.get(ix)? {
            Slot::Row(row) => self.rows.get(*row),
            Slot::Group(..) => None,
        }
    }

    /// The table row that draws the first row matching `matches`.
    pub fn position(&self, matches: impl Fn(&R) -> bool) -> Option<usize> {
        self.slots.iter().position(|slot| match slot {
            Slot::Row(row) => self.rows.get(*row).is_some_and(&matches),
            Slot::Group(..) => false,
        })
    }

    /// The column the table sorts by and its order.
    pub fn sort(&self) -> Option<(&'static str, SortOrder)> {
        self.sort
    }

    /// Sort by the column `key`; `None` restores the given order.
    pub fn set_sort(&mut self, sort: Option<(&'static str, SortOrder)>) {
        self.sort = sort;
        self.rebuild();
    }

    fn compare(&self, left: usize, right: usize) -> Ordering {
        let (left, right) = (&self.rows[left], &self.rows[right]);
        let primary = self
            .sort
            .and_then(|(key, order)| {
                let compare = self.columns.iter().find(|c| c.key == key)?.compare?;
                let ordering = compare(left, right);
                Some(if order.is_ascending() {
                    ordering
                } else {
                    ordering.reverse()
                })
            })
            .unwrap_or(Ordering::Equal);
        match (primary, self.sort.and(self.tie_break)) {
            (Ordering::Equal, Some(tie_break)) => tie_break(left, right),
            (ordering, _) => ordering,
        }
    }

    fn rebuild(&mut self) {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        if self.sort.is_some() {
            order.sort_by(|a, b| self.compare(*a, *b));
        }
        self.slots = match self.group {
            None => order.into_iter().map(Slot::Row).collect(),
            Some(group) => {
                let mut groups: Vec<(SharedString, Vec<usize>)> = Vec::new();
                let mut first_seen: Vec<SharedString> = Vec::new();
                for row in &self.rows {
                    let label = group(row);
                    if !first_seen.contains(&label) {
                        first_seen.push(label);
                    }
                }
                for label in first_seen {
                    groups.push((label, Vec::new()));
                }
                for ix in order {
                    let label = group(&self.rows[ix]);
                    if let Some((_, members)) = groups.iter_mut().find(|(l, _)| *l == label) {
                        members.push(ix);
                    }
                }
                groups
                    .into_iter()
                    .flat_map(|(label, members)| {
                        std::iter::once(Slot::Group(label, members.len()))
                            .chain(members.into_iter().map(Slot::Row))
                    })
                    .collect()
            }
        };
    }

    fn palette(&self, window: &Window, cx: &App) -> Palette {
        self.palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette)
    }
}

impl<R: 'static> TableDelegate for RowTable<R> {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.slots.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let spec = &self.columns[col_ix];
        let mut column = Column::new(spec.key, spec.label).width(px(spec.width));
        if let Some(min) = spec.min_width {
            column = column.min_width(px(min));
        }
        if let Some(max) = spec.max_width {
            column = column.max_width(px(max));
        }
        if spec.right {
            column = column.text_right();
        }
        if spec.compare.is_some() {
            column = match self.sort {
                Some((key, SortOrder::Ascending)) if key == spec.key => column.ascending(),
                Some((key, SortOrder::Descending)) if key == spec.key => column.descending(),
                _ => column.sortable(),
            };
        }
        column
    }

    fn fit_column(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Option<Pixels> {
        let spec = self.columns.get(col_ix)?;
        let mut max_chars = spec.label.chars().count();
        if let Some(text_fn) = spec.text {
            for row in &self.rows {
                let len = text_fn(row).chars().count();
                if len > max_chars {
                    max_chars = len;
                }
            }
        }
        // Approximate character width (~8.5px) plus cell padding & icon gutter (24px)
        let estimated = (max_chars as f32 * 8.5 + 24.0).clamp(
            spec.min_width.unwrap_or(40.),
            spec.max_width.unwrap_or(800.),
        );
        Some(px(estimated))
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let key = self.columns[col_ix].key;
        self.sort = match sort {
            ColumnSort::Ascending => Some((key, SortOrder::Ascending)),
            ColumnSort::Descending => Some((key, SortOrder::Descending)),
            ColumnSort::Default => None,
        };
        self.rebuild();
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        if col_ix < self.columns.len() && to_ix < self.columns.len() {
            let column = self.columns.remove(col_ix);
            self.columns.insert(to_ix, column);
        }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let palette = self.palette(window, cx);
        let spec = &self.columns[col_ix];
        div()
            .size_full()
            .flex()
            .items_center()
            .when(spec.right, |th| th.justify_end())
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.muted))
            .truncate()
            .child(spec.label)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui_kit::Div> {
        let tr = div().id(("row", row_ix));
        if matches!(self.slots.get(row_ix), Some(Slot::Group(..))) {
            let palette = self.palette(window, cx);
            tr.bg(rgba(tint(palette.raised, 0x60)))
        } else {
            tr
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let palette = self.palette(window, cx);
        match self.slots.get(row_ix) {
            Some(Slot::Group(label, count)) if col_ix == 0 => div()
                .truncate()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(format!("{label} · {count}"))
                .into_any_element(),
            Some(Slot::Row(row)) => match (self.rows.get(*row), self.columns.get(col_ix)) {
                (Some(row), Some(column)) => {
                    let render = column.render.clone();
                    render(row, palette, window, cx)
                }
                _ => div().into_any_element(),
            },
            _ => div().into_any_element(),
        }
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        match self.slots.get(row_ix)? {
            Slot::Group(label, _) => (col_ix == 0).then(|| label.clone()),
            Slot::Row(row) => {
                let text = self.columns.get(col_ix)?.text?;
                Some(text(self.rows.get(*row)?).into())
            }
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String {
        self.cell_label(row_ix, col_ix, cx)
            .map(|label| label.to_string())
            .unwrap_or_default()
    }

    fn render_empty(
        &mut self,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let palette = self.palette(window, cx);
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(rgb(palette.muted))
            .child(self.empty.clone())
    }
}

/// A `TableState` for `table`, with the options every Sjel table shares.
pub fn new_table_state<R: 'static>(
    table: RowTable<R>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TableState<RowTable<R>>> {
    cx.new(|cx| {
        TableState::new(table, window, cx)
            .row_selectable(true)
            .col_selectable(false)
            .sortable(true)
            .col_resizable(true)
            .col_movable(true)
    })
}

/// Replace a table's rows and redraw it, keeping column widths, order and sort.
pub fn set_table_rows<R: 'static>(
    state: &Entity<TableState<RowTable<R>>>,
    rows: Vec<R>,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        state.delegate_mut().set_rows(rows);
        cx.notify();
    });
}

/// The table element, with row height from `density`.
pub fn data_table<R: 'static>(
    state: &Entity<TableState<RowTable<R>>>,
    label: &'static str,
    density: Density,
) -> AnyElement {
    DataTable::new(state)
        .with_size(Size::Size(density.row_height()))
        .accessibility_label(label)
        .bordered(true)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui;

    #[derive(Clone, Debug, PartialEq)]
    struct Item {
        name: &'static str,
        group: &'static str,
        amount: i64,
    }

    fn table() -> RowTable<Item> {
        RowTable::new(vec![
            TableColumn::text("name", "NAME", 120., |i: &Item| i.name.to_owned())
                .sort_by(|a, b| a.name.cmp(b.name)),
            TableColumn::text("amount", "AMOUNT", 80., |i: &Item| i.amount.to_string())
                .right()
                .sort_by(|a, b| a.amount.cmp(&b.amount)),
        ])
        .tie_break(|a, b| a.name.cmp(b.name))
    }

    fn items() -> Vec<Item> {
        vec![
            Item {
                name: "c",
                group: "x",
                amount: 2,
            },
            Item {
                name: "a",
                group: "y",
                amount: 2,
            },
            Item {
                name: "b",
                group: "x",
                amount: 1,
            },
        ]
    }

    fn names(table: &RowTable<Item>) -> Vec<Option<&'static str>> {
        (0..table.slots.len())
            .map(|ix| table.row(ix).map(|i| i.name))
            .collect()
    }

    #[test]
    fn unsorted_rows_keep_their_order() {
        let mut table = table();
        table.set_rows(items());
        assert_eq!(names(&table), [Some("c"), Some("a"), Some("b")]);
    }

    #[test]
    fn sort_survives_new_rows_and_ties_break_by_name() {
        let mut table = table().sorted("amount", SortOrder::Descending);
        table.set_rows(items());
        assert_eq!(names(&table), [Some("a"), Some("c"), Some("b")]);
        table.set_sort(Some(("amount", SortOrder::Ascending)));
        table.set_rows(items());
        assert_eq!(names(&table), [Some("b"), Some("a"), Some("c")]);
        table.set_sort(None);
        assert_eq!(names(&table), [Some("c"), Some("a"), Some("b")]);
    }

    #[test]
    fn groups_keep_first_seen_order_and_sort_inside() {
        let mut table = table()
            .sorted("name", SortOrder::Ascending)
            .grouped_by(Some(|i: &Item| SharedString::from(i.group)));
        table.set_rows(items());
        assert_eq!(names(&table), [None, Some("b"), Some("c"), None, Some("a")]);
        assert_eq!(table.position(|i| i.name == "a"), Some(4));
    }

    #[gpui::test]
    fn column_reports_the_active_sort(cx: &mut gpui::TestAppContext) {
        let table = table().sorted("amount", SortOrder::Descending);
        cx.update(|cx| {
            assert_eq!(table.column(0, cx).sort, Some(ColumnSort::Default));
            assert_eq!(table.column(1, cx).sort, Some(ColumnSort::Descending));
        });
    }

    #[gpui::test]
    fn fit_column_measures_header_and_content_width(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
        });
        cx.skip_drawing();
        let mut table = table();
        table.set_rows(vec![Item {
            name: "A very long item name exceeding normal width",
            group: "x",
            amount: 100,
        }]);
        let window = cx.add_window(|window, cx| TableState::new(table, window, cx));
        window
            .update(cx, |state, window, cx| {
                let fitted = state.delegate_mut().fit_column(0, window, cx);
                assert!(fitted.is_some());
                let width: f32 = fitted.unwrap().into();
                // Estimated from ~44 chars * 8.5 + 24
                assert!(width > 300.0);
            })
            .unwrap();
    }
}
