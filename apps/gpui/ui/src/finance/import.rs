//! CSV bank import, column mapping, staging, candidate matching, and reconciliation.

use std::rc::Rc;
use std::time::Instant;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Input, InputEvent, TextInput};
use crate::ely::theme::ControlSize;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, ClickEvent, Context, Entity, FontWeight,
    PathPromptOptions, SharedString, Subscription, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    parse_personal_cents, prioritized_expense_queue, reviewable_expenses, selected_csv_mapping,
    ExpenseReview, ReviewQueueMode, ReviewScope,
};
use sjel_gpui_core::shell::{self, FinanceCandidate, ShellError};

use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction, SortKey};
use crate::ely::theme::Density;
use crate::primitives::tabular_cell;
use crate::theme::Palette;

use super::FinancePage;

fn candidate_columns() -> [Column; 5] {
    [
        Column::new("cand_date", "DATE").width(rems(6.)).muted(),
        Column::new("cand_description", "DESCRIPTION"),
        Column::new("cand_account", "PROPOSED ACCOUNT")
            .width(rems(11.))
            .muted(),
        Column::new("cand_amount", "AMOUNT").width(rems(7.)).end(),
        Column::new("cand_actions", "").width(rems(12.)).actions([
            RowAction::new("transfer", "Reconcile"),
            RowAction::new("confirm", "Confirm").primary(),
            RowAction::new("confirm-mail", "Confirm").primary(),
            RowAction::new("reject", "Reject"),
        ]),
    ]
}

/// The one transfer a candidate matches exactly, if it matches one.
fn exact_transfer(c: &FinanceCandidate) -> Option<&String> {
    match c.transfer_match_ids.as_slice() {
        [counterpart] => Some(counterpart),
        _ => None,
    }
}

fn is_mail(c: &FinanceCandidate) -> bool {
    c.source_reference
        .as_deref()
        .is_some_and(|reference| reference.starts_with("gmail:"))
}

/// The actions a candidate offers: an exact transfer reconciles, a bank row confirms its proposal, and a mail row confirms only from a known source account.
fn offered(c: &FinanceCandidate, has_mail_source: bool) -> &'static [&'static str] {
    match (exact_transfer(c).is_some(), is_mail(c)) {
        (true, _) => &["transfer", "reject"],
        (false, false) => &["confirm", "reject"],
        (false, true) if has_mail_source => &["confirm-mail", "reject"],
        (false, true) => &["reject"],
    }
}

/// A candidate's row. A mail row that confirms from `mail_source` names that account under its proposed one.
fn candidate_row(c: &FinanceCandidate, mail_source: Option<&str>) -> Row {
    let offered = offered(c, mail_source.is_some())
        .iter()
        .copied()
        .map(SharedString::from);
    let account = match mail_source.filter(|_| is_mail(c) && exact_transfer(c).is_none()) {
        Some(source) => Cell::Lines(
            c.proposed_account.clone().into(),
            format!("from {source}").into(),
        ),
        None => Cell::Text(c.proposed_account.clone().into()),
    };
    let tone = if c.amount_cents < 0 {
        Tone::Danger
    } else {
        Tone::Success
    };
    Row::new(
        c.id.clone(),
        [
            Cell::Text(c.booked_at.clone().into()),
            Cell::Text(c.description.clone().into()),
            account,
            Cell::Figure(
                format_money(c.amount_cents, &c.currency).into(),
                c.amount_cents as f64,
                tone,
            ),
            Cell::Actions(offered.collect()),
        ],
    )
}

pub(crate) struct CsvMappingField {
    pub(super) label: &'static str,
    pub(super) input: Entity<TextInput>,
    pub(super) _subscription: Subscription,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportDerivedKey {
    candidates_loaded_at: Option<Instant>,
    dashboard_loaded_at: Option<Instant>,
    trip_plans_loaded_at: Option<Instant>,
    review_revision: u64,
}

#[derive(Default)]
pub(crate) struct ImportDerived {
    pub(super) key: Option<ImportDerivedKey>,
    pub(super) pending_indices: Vec<usize>,
    pub(super) uncategorized: Vec<FinanceCandidate>,
    pub(super) review_queue: Vec<ExpenseReview>,
    pub(super) inflows: Vec<FinanceCandidate>,
}

pub(crate) fn default_csv_mapping() -> serde_json::Value {
    serde_json::json!({
        "delimiter": ";",
        "decimal_separator": ",",
        "date_column": "Buchung",
        "amount_column": "Betrag",
        "description_column": "Verwendungszweck",
        "categorization_columns": [],
        "reference_column": null,
        "currency_column": "Währung",
        "default_currency": "EUR",
        "source_account": "assets:bank:checking",
        "default_outflow_account": "expenses:uncategorized",
        "default_inflow_account": "income:uncategorized",
        "categorization_rules": [],
        "row_filter": null,
        "amount_sign": "as_provided",
        "amount_rounding": "reject",
        "date_formats": ["day_month_year_dots"],
        "row_policy": "strict",
        "location_columns": null,
    })
}

impl FinancePage {
    /// The account a mail candidate is confirmed from: the selected CSV mapping's source account, when it is an asset or liability.
    fn mail_source_account(&self) -> Option<String> {
        self.csv_mappings
            .value
            .as_ref()
            .and_then(|profiles| profiles.get(self.selected_mapping))
            .and_then(|profile| profile.mapping.get("source_account"))
            .and_then(serde_json::Value::as_str)
            .filter(|account| account.starts_with("assets:") || account.starts_with("liabilities:"))
            .map(str::to_owned)
    }

    /// The pending candidates shown, newest first, built once per change.
    pub(super) fn import_rows(&mut self) -> Rc<Vec<Row>> {
        let mail_source = self.mail_source_account();
        let key = (
            self.candidates.loaded_at,
            self.review_rev,
            self.pending_limit,
            mail_source.clone(),
        );
        if self.import_rows_key.as_ref() != Some(&key) {
            self.import_rows_key = Some(key);
            self.sync_import_derived();
            let candidates = self.candidates.value.as_deref().unwrap_or_default();
            let mut pending: Vec<&FinanceCandidate> = self
                .import_derived
                .pending_indices
                .iter()
                .take(self.pending_limit)
                .filter_map(|&ix| candidates.get(ix))
                .collect();
            pending.sort_by(|a, b| b.booked_at.cmp(&a.booked_at).then_with(|| a.id.cmp(&b.id)));
            self.import_rows = Rc::new(
                pending
                    .into_iter()
                    .map(|c| candidate_row(c, mail_source.as_deref()))
                    .collect(),
            );
        }
        self.import_rows.clone()
    }

    /// Runs row action `action` on the candidate with id `id`.
    fn candidate_action(&mut self, id: &str, action: &str, cx: &mut Context<Self>) {
        let Some(candidate) = (self.candidates.value.iter().flatten()).find(|c| c.id == id) else {
            return;
        };
        let (id, counterpart) = (candidate.id.clone(), exact_transfer(candidate).cloned());
        match (action, counterpart) {
            ("transfer", Some(counterpart)) => self.reconcile_candidate(id, counterpart, cx),
            ("confirm", _) => self.review_candidate(id, "confirm", None, cx),
            ("confirm-mail", _) => {
                if let Some(account) = self.mail_source_account() {
                    self.review_candidate(id, "confirm", Some(account), cx);
                }
            }
            ("reject", _) => self.review_candidate(id, "reject", None, cx),
            _ => {}
        }
    }

    pub(super) fn sync_import_derived(&mut self) {
        let key = ImportDerivedKey {
            candidates_loaded_at: self.candidates.loaded_at,
            dashboard_loaded_at: self.dashboard.loaded_at,
            trip_plans_loaded_at: self.trip_plans.loaded_at,
            review_revision: self.review_rev,
        };
        if self.import_derived.key == Some(key) {
            return;
        }
        let candidates = self.candidates.value.as_deref().unwrap_or_default();
        let pending_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(ix, c)| (c.state == "pending").then_some(ix))
            .collect();
        let mut uncategorized: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.state == "confirmed"
                    && candidate.amount_cents < 0
                    && candidate
                        .proposed_account
                        .split(':')
                        .any(|part| part == "uncategorized")
            })
            .cloned()
            .collect();
        uncategorized.sort_by(|a, b| b.booked_at.cmp(&a.booked_at).then_with(|| a.id.cmp(&b.id)));
        let transactions = self
            .dashboard
            .value
            .as_ref()
            .map_or(&[][..], |d| d.transactions.as_slice());
        let eligible = reviewable_expenses(
            candidates,
            transactions,
            &ReviewScope {
                start: String::new(),
                end: "9999-12-31".into(),
                account: String::new(),
                category: String::new(),
            },
        );
        let focused_plan = self
            .trip_plans
            .value
            .as_ref()
            .and_then(|plans| plans.iter().find(|plan| plan.id == self.review_trip_id));
        let review_queue = prioritized_expense_queue(
            &eligible,
            self.queue_mode,
            focused_plan.map(|plan| (plan.date_start.as_str(), plan.date_end.as_str())),
        );
        let inflows: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidate.state == "confirmed"
                    && candidate.amount_cents > 0
                    && candidate.proposed_account != "assets:receivable:shared"
            })
            .cloned()
            .collect();
        self.import_derived = ImportDerived {
            key: Some(key),
            pending_indices,
            uncategorized,
            review_queue,
            inflows,
        };
    }

    pub(super) fn active_csv_mapping(&self) -> serde_json::Value {
        let profiles = self
            .csv_mappings
            .value
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        profiles
            .get(self.selected_mapping)
            .map(|profile| profile.mapping.clone())
            .unwrap_or_else(|| self.manual_csv_mapping.clone())
    }

    pub(super) fn ensure_csv_mapping_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.csv_mapping_fields.is_some() {
            return;
        }
        let fields = [
            ("delimiter", "Delimiter"),
            ("decimal_separator", "Decimal separator"),
            ("date_column", "Date column"),
            ("amount_column", "Amount column"),
            ("description_column", "Description column"),
            ("reference_column", "Reference column (optional)"),
            ("currency_column", "Currency column (optional)"),
            ("default_currency", "Default currency"),
            ("source_account", "Source account"),
            ("default_outflow_account", "Default outflow account"),
            ("default_inflow_account", "Default inflow account"),
        ];
        self.csv_mapping_fields = Some(
            fields
                .into_iter()
                .map(|(key, label)| {
                    let initial = self
                        .manual_csv_mapping
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let input = cx.new(|cx| {
                        let mut state = TextInput::new(window, cx).placeholder(label);
                        state.set_text(initial, cx);
                        state
                    });
                    let subscription =
                        cx.subscribe_in(&input, window, move |page, input, event, _, cx| {
                            if matches!(event, InputEvent::Changed) {
                                let value = input.read(cx).text().to_string();
                                if let Some(object) = page.manual_csv_mapping.as_object_mut() {
                                    object.insert(
                                        key.to_owned(),
                                        if matches!(key, "currency_column" | "reference_column")
                                            && value.trim().is_empty()
                                        {
                                            serde_json::Value::Null
                                        } else {
                                            serde_json::Value::String(value)
                                        },
                                    );
                                }
                                page.csv_preview = None;
                                cx.notify();
                            }
                        });
                    CsvMappingField {
                        label,
                        input,
                        _subscription: subscription,
                    }
                })
                .collect(),
        );
    }

    pub(super) fn review_candidate(
        &mut self,
        id: String,
        decision: &'static str,
        source_account: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let account = self
            .candidates
            .value
            .as_ref()
            .and_then(|items| items.iter().find(|candidate| candidate.id == id))
            .map(|candidate| candidate.proposed_account.clone());
        let work = shell::run(async move {
            shell
                .finance_review_candidate(
                    &id,
                    decision,
                    account.as_deref(),
                    source_account.as_deref(),
                )
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn reconcile_candidate(
        &mut self,
        id: String,
        counterpart: String,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work =
            shell::run(async move { shell.finance_reconcile_transfer(&id, &counterpart).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn choose_csv(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a CSV export".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("CSV")
                .to_owned();
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(std::fs::read_to_string(path));
            });
            let loaded = rx.await.ok().and_then(Result::ok);
            let _ = this.update(cx, |page, cx| {
                if let Some(content) = loaded {
                    page.csv_file = Some((name, content));
                    page.csv_preview = None;
                    page.notice = None;
                } else {
                    page.notice = Some("Could not read the selected file as UTF-8 text.".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn preview_csv(&mut self, cx: &mut Context<Self>) {
        let Some((_, content)) = self.csv_file.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let mapping = selected_csv_mapping(&self.active_csv_mapping());
        let work = shell::run(async move { shell.finance_preview_csv(content, mapping).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(preview) => {
                        page.csv_preview = Some(preview);
                        page.notice = None;
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn stage_csv(&mut self, cx: &mut Context<Self>) {
        let (Some((_, content)), Some(preview)) = (self.csv_file.clone(), self.csv_preview.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let mapping = selected_csv_mapping(&self.active_csv_mapping());
        let work = shell::run(async move {
            shell
                .finance_import_csv(content, mapping, preview.preview_id)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        page.csv_file = None;
                        page.csv_preview = None;
                        page.notice = Some(format!(
                            "{} new candidates staged.",
                            value["created"].as_u64().unwrap_or_default()
                        ));
                        page.load(cx);
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl FinancePage {
    pub(super) fn render_import_review(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.import_rows();
        let page = cx.entity();
        let pending_count = self.import_derived.pending_indices.len();
        let queue = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                DataTable::new("finance-candidates", candidate_columns())
                    .label("Import candidates")
                    .rows(rows)
                    .density(Density::Comfortable)
                    .sorts([SortKey {
                        column: "cand_date".into(),
                        rising: false,
                    }])
                    .on_action(move |id, action, _, cx| {
                        page.update(cx, |page, cx| page.candidate_action(id, action, cx))
                    })
                    .empty("No pending import candidates to review."),
            )
            .children((pending_count > self.pending_limit).then(|| {
                Button::new("finance-pending-more", "Show 24 more candidates")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.pending_limit += 24;
                        cx.notify();
                    }))
            }));

        let uncategorized_len = self.import_derived.uncategorized.len();
        let uncategorized_slice: Vec<FinanceCandidate> = self
            .import_derived
            .uncategorized
            .iter()
            .take(self.category_limit)
            .cloned()
            .collect();
        let mut category_rows = div().flex().flex_col().gap_1();
        for candidate in &uncategorized_slice {
            let candidate_id = candidate.id.clone();
            category_rows = category_rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_2()
                    .rounded(px(5.))
                    .child(tabular_cell(
                        candidate.booked_at.clone(),
                        px(96.),
                        palette.muted,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(candidate.description.clone()),
                    )
                    .child(tabular_cell(
                        format_money(candidate.amount_cents, &candidate.currency),
                        px(110.),
                        palette.red,
                    ))
                    .child(
                        Button::new(
                            SharedString::from(format!("finance-select-category-{}", candidate.id)),
                            if self.selected_reclass_candidate.as_deref() == Some(&candidate.id) {
                                "Selected"
                            } else {
                                "Choose"
                            },
                        )
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                page.select_reclass_candidate(candidate_id.clone(), window, cx)
                            },
                        )),
                    ),
            );
        }
        let category_input = div()
            .flex_1()
            .capture_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "up" => page.move_reclass_highlight(-1, cx),
                    "down" => page.move_reclass_highlight(1, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(Input::new(&self.reclass_field(window, cx)).size(ControlSize::Sm));
        let suggestions = self.reclass_suggestions();
        let suggestion_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(suggestions.into_iter().enumerate().map(|(index, account)| {
                let chosen = account.clone();
                Button::new(
                    SharedString::from(format!("finance-category-suggestion-{index}")),
                    account.trim_start_matches("expenses:").to_string(),
                )
                .size(ControlSize::Sm)
                .when(index == self.reclass_highlight, |button| button.primary())
                .when(index != self.reclass_highlight, |button| {
                    button.variant(ButtonVariant::Ghost)
                })
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.apply_reclass(Some(chosen.clone()), window, cx)
                }))
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("↑↓ choose · ⏎ apply and go to the next"),
            );
        let categorize_panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Recategorize confirmed expenses"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{} confirmed expenses need a category",
                        uncategorized_len
                    )),
            )
            .child(category_rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(category_input)
                    .child(
                        Button::new("finance-reclassify-selected", "Reclassify selected")
                            .size(ControlSize::Sm)
                            .primary()
                            .when(
                                self.selected_reclass_candidate.is_none()
                                    || self.reclass_account.trim().is_empty(),
                                |button| button.disabled(true),
                            )
                            .on_click(cx.listener(|page, _, _, cx| page.reclassify_selected(cx))),
                    ),
            )
            .child(suggestion_row);
        let categorize_panel = if uncategorized_len > self.category_limit {
            categorize_panel.child(
                Button::new("finance-category-more", "Show more uncategorized")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.category_limit += 16;
                        cx.notify();
                    })),
            )
        } else {
            categorize_panel
        };

        let review_queue_len = self.import_derived.review_queue.len();
        let review_slice: Vec<ExpenseReview> = self
            .import_derived
            .review_queue
            .iter()
            .take(self.expense_limit)
            .cloned()
            .collect();
        let mut expense_rows = div().flex().flex_col().gap_1();
        for entry in &review_slice {
            let selected = self.selected_expense.as_deref() == Some(&entry.candidate.id);
            let selected_entry = entry.clone();
            expense_rows = expense_rows.child(
                Button::new(
                    SharedString::from(format!("finance-expense-{}", entry.candidate.id)),
                    format!(
                        "{} · {} · {}",
                        entry.candidate.booked_at,
                        entry.candidate.description,
                        format_money(entry.total_cents, &entry.candidate.currency)
                    ),
                )
                .size(ControlSize::Sm)
                .when(selected, |button| button.primary())
                .when(!selected, |button| button.variant(ButtonVariant::Ghost))
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.select_expense(&selected_entry, window, cx)
                })),
            );
        }
        let personal_input = Input::new(&self.personal_field(window, cx)).size(ControlSize::Sm);
        let share_is_valid = self
            .selected_expense
            .as_ref()
            .and_then(|id| {
                self.candidates
                    .value
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .find(|candidate| &candidate.id == id)
            })
            .and_then(|candidate| {
                parse_personal_cents(&self.personal_amount)
                    .map(|cents| cents <= candidate.amount_cents.saturating_abs())
            })
            .unwrap_or(false);
        let mut allocation = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Purpose and shared-cost review"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("finance-mode-unreviewed", "Needs purpose")
                            .size(ControlSize::Sm)
                            .when(self.queue_mode == ReviewQueueMode::Unreviewed, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Unreviewed, |button| {
                                button.variant(ButtonVariant::Ghost)
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Unreviewed;
                                page.review_rev = page.review_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-shared", "Shared costs")
                            .size(ControlSize::Sm)
                            .when(self.queue_mode == ReviewQueueMode::Shared, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Shared, |button| {
                                button.variant(ButtonVariant::Ghost)
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Shared;
                                page.review_rev = page.review_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-all", "All")
                            .size(ControlSize::Sm)
                            .when(self.queue_mode == ReviewQueueMode::All, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::All, |button| {
                                button.variant(ButtonVariant::Ghost)
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::All;
                                page.review_rev = page.review_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    ),
            )
            .child(expense_rows);
        let plans = self.trip_plans.value.as_deref().unwrap_or_default();
        let mut trip_focus = div().flex().items_center().gap_1().child(
            Button::new("finance-trip-focus-all", "Any trip")
                .size(ControlSize::Sm)
                .when(self.review_trip_id.is_empty(), |button| button.primary())
                .when(!self.review_trip_id.is_empty(), |button| {
                    button.variant(ButtonVariant::Ghost)
                })
                .on_click(cx.listener(|page, _, _, cx| {
                    page.review_trip_id.clear();
                    page.review_rev = page.review_rev.wrapping_add(1);
                    cx.notify();
                })),
        );
        for plan in plans.iter().filter(|plan| plan.status != "archived") {
            let id = plan.id.clone();
            trip_focus = trip_focus.child(
                Button::new(
                    SharedString::from(format!("finance-trip-focus-{}", plan.id)),
                    plan.title.clone(),
                )
                .size(ControlSize::Sm)
                .when(self.review_trip_id == plan.id, |button| button.primary())
                .when(self.review_trip_id != plan.id, |button| {
                    button.variant(ButtonVariant::Ghost)
                })
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.review_trip_id = id.clone();
                    page.review_rev = page.review_rev.wrapping_add(1);
                    cx.notify();
                })),
            );
        }
        allocation = allocation.child(trip_focus);
        if review_queue_len > self.expense_limit {
            allocation = allocation.child(
                Button::new("finance-expenses-more", "Show more expenses")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.expense_limit += 16;
                        cx.notify();
                    })),
            );
        }
        if self.selected_expense.is_some() {
            let purposes = [
                ("day_to_day", "Day to day"),
                ("trip", "Trip"),
                ("work", "Work"),
                ("housing", "Housing"),
                ("other", "Other"),
            ];
            let mut purpose_buttons = div().flex().items_center().gap_1();
            for (value, label) in purposes {
                let value_for_click = value.to_owned();
                purpose_buttons = purpose_buttons.child(
                    Button::new(
                        SharedString::from(format!("finance-purpose-{value}")),
                        label,
                    )
                    .size(ControlSize::Sm)
                    .when(self.allocation_purpose == value, |button| button.primary())
                    .when(self.allocation_purpose != value, |button| {
                        button.variant(ButtonVariant::Ghost)
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.allocation_purpose = value_for_click.clone();
                        if page.allocation_purpose != "trip" {
                            page.selected_trip_id.clear();
                        }
                        cx.notify();
                    })),
                );
            }
            allocation = allocation
                .child(purpose_buttons)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Your final cost"),
                        )
                        .child(personal_input),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("finance-share-100", "All mine")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(100, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-50", "Half")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(50, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-25", "Quarter")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(25, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-0", "Fronted all")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(0, window, cx)
                                })),
                        ),
                )
                .child(
                    Button::new("finance-save-allocation", "Save purpose and share")
                        .size(ControlSize::Sm)
                        .primary()
                        .when(
                            !share_is_valid
                                || (self.allocation_purpose == "trip"
                                    && self.selected_trip_id.is_empty()),
                            |button| button.disabled(true),
                        )
                        .on_click(cx.listener(|page, _, _, cx| page.save_allocation(cx))),
                );
            if self.allocation_purpose == "trip" {
                let plans = self.trip_plans.value.as_deref().unwrap_or_default();
                let mut plan_buttons = div().flex().items_center().gap_1().child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Trip:"),
                );
                for plan in plans.iter().filter(|plan| plan.status != "archived") {
                    let trip_id = plan.id.clone();
                    plan_buttons = plan_buttons.child(
                        Button::new(
                            SharedString::from(format!("finance-trip-{}", plan.id)),
                            plan.title.clone(),
                        )
                        .size(ControlSize::Sm)
                        .when(self.selected_trip_id == plan.id, |button| button.primary())
                        .when(self.selected_trip_id != plan.id, |button| {
                            button.variant(ButtonVariant::Ghost)
                        })
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.selected_trip_id = trip_id.clone();
                            cx.notify();
                        })),
                    );
                }
                allocation = allocation.child(plan_buttons);
            }
        }

        let open_receivables: Vec<_> = self
            .dashboard
            .value
            .as_ref()
            .map(|dashboard| {
                dashboard
                    .shared_expenses
                    .iter()
                    .filter(|expense| expense.outstanding_cents > 0)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let inflows_len = self.import_derived.inflows.len();
        let inflows_slice: Vec<FinanceCandidate> = self
            .import_derived
            .inflows
            .iter()
            .take(self.inflow_limit)
            .cloned()
            .collect();
        let mut reimbursement = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Link a repayment"),
            );
        for inflow in &inflows_slice {
            let id = inflow.id.clone();
            reimbursement = reimbursement.child(
                Button::new(
                    SharedString::from(format!("finance-inflow-{}", inflow.id)),
                    format!(
                        "Inflow · {} · {}",
                        inflow.description,
                        format_money(inflow.amount_cents, &inflow.currency)
                    ),
                )
                .size(ControlSize::Sm)
                .when(
                    self.selected_inflow.as_deref() == Some(&inflow.id),
                    |button| button.primary(),
                )
                .when(
                    self.selected_inflow.as_deref() != Some(&inflow.id),
                    |button| button.variant(ButtonVariant::Ghost),
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.selected_inflow = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        if inflows_len > self.inflow_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-inflows-more", "Show more inflows")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.inflow_limit += 8;
                        cx.notify();
                    })),
            );
        }
        for expense in open_receivables.iter().take(self.receivable_limit) {
            let id = expense.candidate_id.clone();
            reimbursement = reimbursement.child(
                Button::new(
                    SharedString::from(format!("finance-receivable-{}", expense.candidate_id)),
                    format!(
                        "Expense · {} · {} open",
                        expense.description,
                        format_money(expense.outstanding_cents, &expense.currency)
                    ),
                )
                .size(ControlSize::Sm)
                .when(
                    self.reimbursement_target.as_deref() == Some(&expense.candidate_id),
                    |button| button.primary(),
                )
                .when(
                    self.reimbursement_target.as_deref() != Some(&expense.candidate_id),
                    |button| button.variant(ButtonVariant::Ghost),
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.reimbursement_target = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        if open_receivables.len() > self.receivable_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-receivables-more", "Show more receivables")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.receivable_limit += 8;
                        cx.notify();
                    })),
            );
        }
        reimbursement = reimbursement.child(
            Button::new("finance-link-reimbursement", "Link selected inflow")
                .size(ControlSize::Sm)
                .primary()
                .when(
                    self.selected_inflow.is_none() || self.reimbursement_target.is_none(),
                    |button| button.disabled(true),
                )
                .on_click(cx.listener(|page, _, _, cx| page.link_reimbursement(cx))),
        );
        let profiles = self
            .csv_mappings
            .value
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        let manual_selected = self.selected_mapping >= profiles.len();
        if manual_selected {
            self.ensure_csv_mapping_fields(window, cx);
        }
        let profile_name = if manual_selected {
            "Manual entry".to_owned()
        } else {
            profiles[self.selected_mapping].label.clone()
        };
        let mapping = self.active_csv_mapping();
        let mapping_detail = format!(
            "Date: {} · Amount: {} · Description: {} · Source: {}",
            mapping["date_column"].as_str().unwrap_or("—"),
            mapping["amount_column"].as_str().unwrap_or("—"),
            mapping["description_column"].as_str().unwrap_or("—"),
            mapping["source_account"].as_str().unwrap_or("—"),
        );
        let mut import_card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(7.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("CSV import"),
            )
            .child(
                div().text_xs().text_color(rgb(palette.muted)).child(
                    self.csv_file
                        .as_ref()
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| {
                            "Choose a CSV file. It is read locally and sent for preview.".into()
                        }),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(mapping_detail),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("finance-choose-csv", "Choose CSV")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(Self::choose_csv)),
                    )
                    .child(
                        Button::new(
                            "finance-mapping-profile",
                            format!("Mapping: {profile_name}"),
                        )
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|page, _, _, cx| {
                            let count = page.csv_mappings.value.as_ref().map_or(0, Vec::len) + 1;
                            page.selected_mapping = (page.selected_mapping + 1) % count;
                            page.csv_preview = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new("finance-preview-csv", "Preview")
                            .size(ControlSize::Sm)
                            .primary()
                            .when(self.csv_file.is_none(), |button| button.disabled(true))
                            .on_click(cx.listener(|page, _, _, cx| page.preview_csv(cx))),
                    ),
            );
        if manual_selected {
            let mut fields = div().flex().flex_col().gap_2();
            if let Some(mapping_fields) = &self.csv_mapping_fields {
                for field in mapping_fields {
                    fields = fields.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(150.))
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(field.label),
                            )
                            .child(
                                div()
                                    .w(px(260.))
                                    .child(Input::new(&field.input).size(ControlSize::Sm)),
                            ),
                    );
                }
            }
            let date_format = self.manual_csv_mapping["date_formats"][0]
                .as_str()
                .unwrap_or("day_month_year_dots");
            let amount_sign = self.manual_csv_mapping["amount_sign"]
                .as_str()
                .unwrap_or("as_provided");
            let row_policy = self.manual_csv_mapping["row_policy"]
                .as_str()
                .unwrap_or("strict");
            fields = fields
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Date format"),
                        )
                        .child(
                            Button::new("finance-manual-date-format", date_format)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    let next = match page.manual_csv_mapping["date_formats"][0]
                                        .as_str()
                                        .unwrap_or("day_month_year_dots")
                                    {
                                        "day_month_year_dots" => "day_month_year_slashes",
                                        "day_month_year_slashes" => "iso_year_month_day",
                                        _ => "day_month_year_dots",
                                    };
                                    page.manual_csv_mapping["date_formats"] =
                                        serde_json::json!([next]);
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Amount sign"),
                        )
                        .child(
                            Button::new("finance-manual-amount-sign", amount_sign)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.manual_csv_mapping["amount_sign"] = serde_json::json!(
                                        if page.manual_csv_mapping["amount_sign"] == "invert" {
                                            "as_provided"
                                        } else {
                                            "invert"
                                        }
                                    );
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(150.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Row policy"),
                        )
                        .child(
                            Button::new("finance-manual-row-policy", row_policy)
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.manual_csv_mapping["row_policy"] =
                                        serde_json::json!(if page.manual_csv_mapping["row_policy"]
                                            == "strict"
                                        {
                                            "required_fields"
                                        } else {
                                            "strict"
                                        });
                                    page.csv_preview = None;
                                    cx.notify();
                                })),
                        ),
                );
            import_card = import_card.child(fields);
        }
        if let Some(preview) = &self.csv_preview {
            import_card = import_card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                            "{} candidates · {} outflows · {} inflows · {} duplicates · {} ignored",
                            preview.candidate_count,
                            preview.outflow_count,
                            preview.inflow_count,
                            preview.duplicate_rows,
                            preview.ignored_non_transaction_rows
                        )),
                    )
                    .child(
                        Button::new("finance-stage-csv", "Stage preview")
                            .size(ControlSize::Sm)
                            .primary()
                            .on_click(cx.listener(|page, _, _, cx| page.stage_csv(cx))),
                    ),
            );
        }
        let mut view = div()
            .id("finance-import-review-scroll")
            .size_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .overflow_y_scroll()
            .track_scroll(&self.import_scroll)
            .vertical_scrollbar(&self.import_scroll)
            .pb_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("Import and review"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!("{pending_count} pending candidates")),
                    ),
            )
            .child(import_card)
            .when_some(self.notice.clone(), |view, notice| {
                view.child(div().text_sm().text_color(rgb(palette.red)).child(notice))
            })
            .child(queue)
            .child(categorize_panel)
            .child(allocation)
            .child(reimbursement);
        if self.csv_mappings.loading {
            view = view.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Loading mapping profiles…"),
            );
        }
        view.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(reference: Option<&str>, transfers: &[&str], cents: i64) -> FinanceCandidate {
        FinanceCandidate {
            id: "c1".into(),
            fingerprint: "f".into(),
            booked_at: "2026-11-14".into(),
            description: "Coffee".into(),
            amount_cents: cents,
            currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            source_reference: reference.map(Into::into),
            proposed_account: "expenses:food".into(),
            confidence_basis_points: 9000,
            state: "pending".into(),
            transfer_match_ids: transfers.iter().map(|id| id.to_string()).collect(),
        }
    }

    #[gpui::test]
    fn manual_csv_mapping_tracks_typed_values_and_cleared_optional_columns(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::Focusable;

        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = FinancePage::new(crate::data::Repo::new(Err(ShellError::NotSetUp)), cx);
            page.set_view(super::super::View::ImportReview, cx);
            page
        });
        cx.update(|window, cx| {
            let field = &page.read(cx).csv_mapping_fields.as_ref().unwrap()[5].input;
            field.read(cx).focus_handle(cx).focus(window, cx);
        });
        cx.simulate_input("Reference");
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.manual_csv_mapping["reference_column"], "Reference")
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_keystrokes("backspace");
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert!(page.manual_csv_mapping["reference_column"].is_null())
        });
    }

    #[test]
    fn a_candidate_offers_the_actions_its_source_allows() {
        let bank = candidate(None, &[], -450);
        assert_eq!(offered(&bank, true), ["confirm", "reject"]);
        assert_eq!(
            offered(&candidate(None, &["c2"], -450), false),
            ["transfer", "reject"]
        );
        assert_eq!(
            offered(&candidate(None, &["c2", "c3"], -450), false),
            ["confirm", "reject"],
            "two matches are not an exact transfer"
        );
        let mail = candidate(Some("gmail:abc"), &[], -450);
        assert_eq!(offered(&mail, true), ["confirm-mail", "reject"]);
        assert_eq!(offered(&mail, false), ["reject"]);
    }

    #[test]
    fn a_mail_row_names_the_account_it_confirms_from() {
        let row = |c: &FinanceCandidate| candidate_row(c, Some("assets:bank:checking"));
        let mail = candidate(Some("gmail:abc"), &[], -450);
        assert_eq!(
            row(&mail),
            Row::new(
                "c1",
                [
                    Cell::Text("2026-11-14".into()),
                    Cell::Text("Coffee".into()),
                    Cell::Lines("expenses:food".into(), "from assets:bank:checking".into()),
                    Cell::Figure(format_money(-450, "EUR").into(), -450.0, Tone::Danger),
                    Cell::Actions(vec!["confirm-mail".into(), "reject".into()]),
                ],
            )
        );
        assert_eq!(
            row(&candidate(None, &[], -450)),
            Row::new(
                "c1",
                [
                    Cell::Text("2026-11-14".into()),
                    Cell::Text("Coffee".into()),
                    Cell::Text("expenses:food".into()),
                    Cell::Figure(format_money(-450, "EUR").into(), -450.0, Tone::Danger),
                    Cell::Actions(vec!["confirm".into(), "reject".into()]),
                ],
            ),
            "a bank row confirms from its own source"
        );
    }

    #[test]
    fn income_reads_green_and_spending_red() {
        let row = candidate_row(&candidate(None, &[], 900), None);
        let income = Row::new(
            "c1",
            [
                Cell::Text("2026-11-14".into()),
                Cell::Text("Coffee".into()),
                Cell::Text("expenses:food".into()),
                Cell::Figure(format_money(900, "EUR").into(), 900.0, Tone::Success),
                Cell::Actions(vec!["confirm".into(), "reject".into()]),
            ],
        );
        assert_eq!(row, income);
    }
}
