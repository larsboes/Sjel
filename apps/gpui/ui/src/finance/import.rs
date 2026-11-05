//! CSV bank import, column mapping, staging, candidate matching, and reconciliation.

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, Context, Entity, FontWeight,
    PathPromptOptions, SharedString, Subscription, Window,
};
use sjel_gpui_core::context::format_money;
use sjel_gpui_core::finance::{
    parse_personal_cents, prioritized_expense_queue, reviewable_expenses, selected_csv_mapping,
    ExpenseReview, ReviewQueueMode, ReviewScope,
};
use sjel_gpui_core::shell::{self, FinanceCandidate, ShellError};

use crate::theme::Palette;

use super::FinancePage;

pub(crate) struct CsvMappingField {
    pub(super) label: &'static str,
    pub(super) input: Entity<InputState>,
    pub(super) _subscription: Subscription,
}

#[derive(Default)]

pub(crate) struct ImportDerived {
    pub(super) key: Option<(Option<Instant>, Option<Instant>, Option<Instant>, u64)>,
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
    pub(super) fn sync_import_derived(&mut self) {
        let key = (
            self.candidates.loaded_at,
            self.dashboard.loaded_at,
            self.trip_plans.loaded_at,
            self.review_rev,
        );
        if self.import_derived.key == Some(key) {
            return;
        }
        let candidates = self.candidates.value.as_deref().unwrap_or_default();
        let pending_indices: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter_map(|(ix, c)| (c.state == "pending").then_some(ix))
            .collect();
        let uncategorized: Vec<_> = candidates
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

    pub(super) fn ensure_csv_mapping_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                        let mut state = InputState::new(window, cx).placeholder(label);
                        state.set_value(initial, window, cx);
                        state
                    });
                    let subscription =
                        cx.subscribe_in(&input, window, move |page, input, event, _, cx| {
                            if matches!(event, InputEvent::Change) {
                                let value = input.read(cx).value().to_string();
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

    pub(super) fn reconcile_candidate(&mut self, id: String, counterpart: String, cx: &mut Context<Self>) {
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
        self.sync_import_derived();
        let candidates = self.candidates.value.as_deref().unwrap_or_default();
        let pending_count = self.import_derived.pending_indices.len();
        let pending: Vec<FinanceCandidate> = self
            .import_derived
            .pending_indices
            .iter()
            .take(self.pending_limit)
            .filter_map(|&ix| candidates.get(ix).cloned())
            .collect();
        let mut queue = div().flex().flex_col().gap_2();
        for candidate in &pending {
            let id = candidate.id.clone();
            let reject_id = id.clone();
            let exact_transfer = candidate.transfer_match_ids.len() == 1;
            let transfer = exact_transfer.then(|| candidate.transfer_match_ids[0].clone());
            let is_mail = candidate
                .source_reference
                .as_deref()
                .is_some_and(|reference| reference.starts_with("gmail:"));
            let mail_source = self
                .csv_mappings
                .value
                .as_ref()
                .and_then(|profiles| profiles.get(self.selected_mapping))
                .and_then(|profile| profile.mapping.get("source_account"))
                .and_then(serde_json::Value::as_str)
                .filter(|account| {
                    account.starts_with("assets:") || account.starts_with("liabilities:")
                })
                .map(str::to_owned);
            let mut row = div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .p_3()
                .rounded(px(7.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(candidate.description.clone())
                        .child(format!(
                            "{} · {} · {}",
                            candidate.booked_at,
                            candidate.proposed_account,
                            format_money(candidate.amount_cents, &candidate.currency)
                        )),
                );
            if let Some(counterpart) = transfer {
                let reconcile_id = id.clone();
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "finance-transfer-{}",
                        candidate.id
                    )))
                    .small()
                    .ghost()
                    .label("Reconcile transfer")
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.reconcile_candidate(reconcile_id.clone(), counterpart.clone(), cx)
                    })),
                );
            }
            if !exact_transfer && !is_mail {
                let confirm_id = id.clone();
                row = row.child(
                    Button::new(SharedString::from(format!(
                        "finance-confirm-{}",
                        candidate.id
                    )))
                    .small()
                    .primary()
                    .label("Confirm proposed account")
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.review_candidate(confirm_id.clone(), "confirm", None, cx)
                    })),
                );
            } else if !exact_transfer && is_mail {
                if let Some(source_account) = mail_source {
                    let confirm_id = id.clone();
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "finance-confirm-mail-{}",
                            candidate.id
                        )))
                        .small()
                        .primary()
                        .label(format!("Confirm from {source_account}"))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.review_candidate(
                                confirm_id.clone(),
                                "confirm",
                                Some(source_account.clone()),
                                cx,
                            )
                        })),
                    );
                }
            }
            row = row.child(
                Button::new(SharedString::from(format!(
                    "finance-reject-{}",
                    candidate.id
                )))
                .small()
                .ghost()
                .label("Reject")
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.review_candidate(reject_id.clone(), "reject", None, cx)
                })),
            );
            queue = queue.child(row);
        }
        if pending_count > self.pending_limit {
            queue = queue.child(
                Button::new("finance-pending-more")
                    .small()
                    .ghost()
                    .label("Show 24 more candidates")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.pending_limit += 24;
                        cx.notify();
                    })),
            );
        }

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
                    .justify_between()
                    .gap_2()
                    .p_2()
                    .rounded(px(5.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(candidate.description.clone())
                            .child(format!(
                                "{} · {}",
                                candidate.booked_at,
                                format_money(candidate.amount_cents, &candidate.currency)
                            )),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "finance-select-category-{}",
                            candidate.id
                        )))
                        .small()
                        .ghost()
                        .label(
                            if self.selected_reclass_candidate.as_deref() == Some(&candidate.id) {
                                "Selected"
                            } else {
                                "Choose"
                            },
                        )
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                page.select_reclass_candidate(candidate_id.clone(), window, cx)
                            },
                        )),
                    ),
            );
        }
        let category_input = Input::new(&self.reclass_field(window, cx)).small();
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
                        Button::new("finance-reclassify-selected")
                            .small()
                            .primary()
                            .label("Reclassify selected")
                            .when(
                                self.selected_reclass_candidate.is_none()
                                    || self.reclass_account.trim().is_empty(),
                                |button| button.disabled(true),
                            )
                            .on_click(cx.listener(|page, _, _, cx| page.reclassify_selected(cx))),
                    ),
            );
        let categorize_panel = if uncategorized_len > self.category_limit {
            categorize_panel.child(
                Button::new("finance-category-more")
                    .small()
                    .ghost()
                    .label("Show more uncategorized")
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
                Button::new(SharedString::from(format!(
                    "finance-expense-{}",
                    entry.candidate.id
                )))
                .small()
                .when(selected, |button| button.primary())
                .when(!selected, |button| button.ghost())
                .label(format!(
                    "{} · {} · {}",
                    entry.candidate.booked_at,
                    entry.candidate.description,
                    format_money(entry.total_cents, &entry.candidate.currency)
                ))
                .on_click(cx.listener(move |page, _, window, cx| {
                    page.select_expense(&selected_entry, window, cx)
                })),
            );
        }
        let personal_input = Input::new(&self.personal_field(window, cx)).small();
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
                        Button::new("finance-mode-unreviewed")
                            .xsmall()
                            .label("Needs purpose")
                            .when(self.queue_mode == ReviewQueueMode::Unreviewed, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Unreviewed, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Unreviewed;
                                page.review_rev = page.review_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-shared")
                            .xsmall()
                            .label("Shared costs")
                            .when(self.queue_mode == ReviewQueueMode::Shared, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::Shared, |button| {
                                button.ghost()
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.queue_mode = ReviewQueueMode::Shared;
                                page.review_rev = page.review_rev.wrapping_add(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-mode-all")
                            .xsmall()
                            .label("All")
                            .when(self.queue_mode == ReviewQueueMode::All, |button| {
                                button.primary()
                            })
                            .when(self.queue_mode != ReviewQueueMode::All, |button| {
                                button.ghost()
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
            Button::new("finance-trip-focus-all")
                .xsmall()
                .label("Any trip")
                .when(self.review_trip_id.is_empty(), |button| button.primary())
                .when(!self.review_trip_id.is_empty(), |button| button.ghost())
                .on_click(cx.listener(|page, _, _, cx| {
                    page.review_trip_id.clear();
                    page.review_rev = page.review_rev.wrapping_add(1);
                    cx.notify();
                })),
        );
        for plan in plans.iter().filter(|plan| plan.status != "archived") {
            let id = plan.id.clone();
            trip_focus = trip_focus.child(
                Button::new(SharedString::from(format!(
                    "finance-trip-focus-{}",
                    plan.id
                )))
                .xsmall()
                .label(plan.title.clone())
                .when(self.review_trip_id == plan.id, |button| button.primary())
                .when(self.review_trip_id != plan.id, |button| button.ghost())
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
                Button::new("finance-expenses-more")
                    .small()
                    .ghost()
                    .label("Show more expenses")
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
                    Button::new(SharedString::from(format!("finance-purpose-{value}")))
                        .xsmall()
                        .label(label)
                        .when(self.allocation_purpose == value, |button| button.primary())
                        .when(self.allocation_purpose != value, |button| button.ghost())
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
                            Button::new("finance-share-100")
                                .xsmall()
                                .ghost()
                                .label("All mine")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(100, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-50")
                                .xsmall()
                                .ghost()
                                .label("Half")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(50, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-25")
                                .xsmall()
                                .ghost()
                                .label("Quarter")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(25, window, cx)
                                })),
                        )
                        .child(
                            Button::new("finance-share-0")
                                .xsmall()
                                .ghost()
                                .label("Fronted all")
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.set_personal_share(0, window, cx)
                                })),
                        ),
                )
                .child(
                    Button::new("finance-save-allocation")
                        .small()
                        .primary()
                        .label("Save purpose and share")
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
                        Button::new(SharedString::from(format!("finance-trip-{}", plan.id)))
                            .xsmall()
                            .label(plan.title.clone())
                            .when(self.selected_trip_id == plan.id, |button| button.primary())
                            .when(self.selected_trip_id != plan.id, |button| button.ghost())
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
                Button::new(SharedString::from(format!("finance-inflow-{}", inflow.id)))
                    .xsmall()
                    .label(format!(
                        "Inflow · {} · {}",
                        inflow.description,
                        format_money(inflow.amount_cents, &inflow.currency)
                    ))
                    .when(
                        self.selected_inflow.as_deref() == Some(&inflow.id),
                        |button| button.primary(),
                    )
                    .when(
                        self.selected_inflow.as_deref() != Some(&inflow.id),
                        |button| button.ghost(),
                    )
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.selected_inflow = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        if inflows_len > self.inflow_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-inflows-more")
                    .small()
                    .ghost()
                    .label("Show more inflows")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.inflow_limit += 8;
                        cx.notify();
                    })),
            );
        }
        for expense in open_receivables.iter().take(self.receivable_limit) {
            let id = expense.candidate_id.clone();
            reimbursement = reimbursement.child(
                Button::new(SharedString::from(format!(
                    "finance-receivable-{}",
                    expense.candidate_id
                )))
                .xsmall()
                .label(format!(
                    "Expense · {} · {} open",
                    expense.description,
                    format_money(expense.outstanding_cents, &expense.currency)
                ))
                .when(
                    self.reimbursement_target.as_deref() == Some(&expense.candidate_id),
                    |button| button.primary(),
                )
                .when(
                    self.reimbursement_target.as_deref() != Some(&expense.candidate_id),
                    |button| button.ghost(),
                )
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.reimbursement_target = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        if open_receivables.len() > self.receivable_limit {
            reimbursement = reimbursement.child(
                Button::new("finance-receivables-more")
                    .small()
                    .ghost()
                    .label("Show more receivables")
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.receivable_limit += 8;
                        cx.notify();
                    })),
            );
        }
        reimbursement = reimbursement.child(
            Button::new("finance-link-reimbursement")
                .small()
                .primary()
                .label("Link selected inflow")
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
                        Button::new("finance-choose-csv")
                            .small()
                            .ghost()
                            .label("Choose CSV")
                            .on_click(cx.listener(Self::choose_csv)),
                    )
                    .child(
                        Button::new("finance-mapping-profile")
                            .small()
                            .ghost()
                            .label(format!("Mapping: {profile_name}"))
                            .on_click(cx.listener(|page, _, _, cx| {
                                let count =
                                    page.csv_mappings.value.as_ref().map_or(0, Vec::len) + 1;
                                page.selected_mapping = (page.selected_mapping + 1) % count;
                                page.csv_preview = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("finance-preview-csv")
                            .small()
                            .primary()
                            .label("Preview")
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
                            .child(div().w(px(260.)).child(Input::new(&field.input).small())),
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
                            Button::new("finance-manual-date-format")
                                .xsmall()
                                .ghost()
                                .label(date_format)
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
                            Button::new("finance-manual-amount-sign")
                                .xsmall()
                                .ghost()
                                .label(amount_sign)
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
                            Button::new("finance-manual-row-policy")
                                .xsmall()
                                .ghost()
                                .label(row_policy)
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
                        Button::new("finance-stage-csv")
                            .small()
                            .primary()
                            .label("Stage preview")
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
