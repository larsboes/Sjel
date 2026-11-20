//! The NSH mail triage view used by non-phone hosts.

use nsh_base::{button, text_view as nsh_markdown_view, TextViewLinks};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    mail::{self, category_label, MailAction, MailProposal, StatusFilter, CATEGORIES},
    shell::FeedContentDetail,
};

use crate::phone_mail::{
    apply_action, apply_gmail_job, closable, close, model_summary, open_detail, refresh,
    row_subtitle, subject, subscribe, Update,
};
pub use crate::phone_mail::{connect, set_offline};

#[derive(Clone, Copy)]
struct OpenThread {
    proposal: Signal<Option<MailProposal>>,
    detail: Signal<Option<FeedContentDetail>>,
    detail_error: Signal<Option<String>>,
    pending_confirm: Signal<Option<MailAction>>,
}

#[component]
pub fn MailScreen() -> impl IntoElement {
    let (initial, mut rx) = subscribe();
    let rows = signal(initial.rows);
    let status = signal(initial.status);
    let status_filter = signal(StatusFilter::Pending);
    let category_filter = signal(None::<&'static str>);
    let open_thread = OpenThread {
        proposal: signal(None::<MailProposal>),
        detail: signal(None::<FeedContentDetail>),
        detail_error: signal(None::<String>),
        pending_confirm: signal(None::<MailAction>),
    };

    spawn(async move {
        while let Some(update) = rx.recv().await {
            match update {
                Update::Status(text) => status.set(text),
                Update::Rows(batch) => {
                    if let Some(current) = open_thread
                        .proposal
                        .with(|p| p.as_ref().map(|m| m.id.clone()))
                    {
                        let updated = batch.iter().find(|m| m.id == current).cloned();
                        open_thread.proposal.set(updated);
                    }
                    rows.set(batch);
                }
                Update::Detail { id, detail } => {
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| m.id == id))
                    {
                        open_thread.detail_error.set(None);
                        open_thread.detail.set(Some(*detail));
                    }
                }
                Update::DetailError { id, error } => {
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| m.id == id))
                    {
                        open_thread.detail_error.set(Some(error));
                    }
                }
                Update::ActionDone { succeeded, notice } => {
                    status.set(notice);
                    rows.update(|all| all.retain(|m| !succeeded.contains(&m.id)));
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| succeeded.contains(&m.id)))
                    {
                        open_thread.proposal.set(None);
                        open_thread.detail.set(None);
                        open_thread.pending_confirm.set(None);
                    }
                }
            }
        }
    });

    view()
        .flex_1()
        .child(closable().then(|| {
            button()
                .px_4()
                .pt_2()
                .on_press(close)
                .child(text("‹ Back").text_base())
        }))
        .child(
            view()
                .px_4()
                .pt_4()
                .pb_2()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(text("Mail").text_2xl().font_weight(FontWeight::SEMIBOLD))
                        .child(
                            button()
                                .px_3()
                                .py_1()
                                .on_press(refresh)
                                .child(text("Refresh").text_sm()),
                        ),
                )
                .child(text(move || status.get()).text_sm()),
        )
        .child(dynamic(move || {
            match open_thread.proposal.with(|p| p.clone()) {
                Some(proposal) => thread_reader(proposal, open_thread).into_element(),
                None => {
                    thread_list(rows, status_filter, category_filter, open_thread).into_element()
                }
            }
        }))
}

fn visible_mail(
    rows: Signal<Vec<MailProposal>>,
    status_filter: Signal<StatusFilter>,
    category_filter: Signal<Option<&'static str>>,
) -> Memo<Vec<MailProposal>> {
    computed(move || {
        let status = status_filter.get();
        let category = category_filter.get();
        rows.with(|all| {
            mail::in_view(all.iter(), status, category)
                .into_iter()
                .cloned()
                .collect()
        })
    })
}

fn thread_list(
    rows: Signal<Vec<MailProposal>>,
    status_filter: Signal<StatusFilter>,
    category_filter: Signal<Option<&'static str>>,
    open_thread: OpenThread,
) -> impl IntoElement {
    let status_chip = move |target: StatusFilter| {
        dynamic(move || {
            let count = rows.with(|all| all.iter().filter(|m| target.matches(&m.status)).count());
            let active = status_filter.get() == target;
            crate::nsh_ui::pill_tab(format!("{} ({count})", target.label()), active, move || {
                status_filter.set(target)
            })
            .into_element()
        })
    };

    let cat_chip = move |key: Option<&'static str>, label: &'static str| {
        button()
            .px_2()
            .py_1()
            .on_press(move || category_filter.set(key))
            .child(
                text(move || {
                    let active_status = status_filter.get();
                    let count = rows.with(|all| {
                        all.iter()
                            .filter(|m| {
                                active_status.matches(&m.status)
                                    && key.is_none_or(|k| m.stream == k)
                            })
                            .count()
                    });
                    if category_filter.get() == key {
                        format!("{label} {count} •")
                    } else {
                        format!("{label} {count}")
                    }
                })
                .text_xs(),
            )
    };

    let visible_rows = visible_mail(rows, status_filter, category_filter);

    let mut status_bar = view().flex_row().px_4().pb_1().gap_2();
    for s in StatusFilter::ALL {
        status_bar = status_bar.child(status_chip(s));
    }

    let mut category_bar = view().flex_row().gap_2().child(cat_chip(None, "All"));
    for (key, label) in CATEGORIES {
        category_bar = category_bar.child(cat_chip(Some(key), label));
    }

    view()
        .flex_1()
        .child(status_bar)
        .child(scroll().horizontal().px_4().pb_2().child(category_bar))
        .child(
            virtual_list(
                move || visible_rows.with(Vec::len),
                move |at| {
                    let item = move || {
                        visible_rows
                            .with(|visible| visible.get(at.index()).cloned())
                            .unwrap_or_else(|| MailProposal {
                                id: String::new(),
                                from_addr: None,
                                subject: None,
                                snippet: None,
                                internal_date: None,
                                stream: String::new(),
                                status: String::new(),
                                gmail_sync_status: None,
                                gmail_sync_action: None,
                                waiting: false,
                                relevance: Vec::new(),
                                authenticity: None,
                                model: None,
                            })
                    };
                    button()
                        .px_4()
                        .py_3()
                        .gap_1()
                        .on_press(move || {
                            let chosen = item();
                            if !chosen.id.is_empty() {
                                let id = chosen.id.clone();
                                open_thread.detail.set(None);
                                open_thread.detail_error.set(None);
                                open_thread.pending_confirm.set(None);
                                open_thread.proposal.set(Some(chosen.clone()));
                                open_detail(id, Some(chosen));
                            }
                        })
                        .child(
                            text(move || subject(&item()))
                                .text_base()
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .child(text(move || row_subtitle(&item())).text_xs())
                        .child(
                            text(move || {
                                let current = item();
                                if let Some((model_line, _)) = model_summary(&current) {
                                    model_line
                                } else {
                                    current.snippet.unwrap_or_default()
                                }
                            })
                            .text_xs(),
                        )
                },
            )
            .estimated_item_size(px(76.0))
            .flex_1(),
        )
}

fn action_btn(
    label: impl IntoReactive<SharedString>,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .on_press(press)
        .child(text(label).text_sm())
}

fn request_or_run(id: String, action: MailAction, open_thread: OpenThread) {
    if action.confirmation(1).is_some() {
        open_thread.pending_confirm.set(Some(action));
    } else {
        open_thread.pending_confirm.set(None);
        apply_action(id, action);
    }
}

fn thread_reader(proposal: MailProposal, open_thread: OpenThread) -> impl IntoElement {
    let id = proposal.id.clone();
    let id_dismiss = id.clone();
    let id_wait = id.clone();
    let id_archive = id.clone();
    let id_trash = id.clone();
    let id_secret = id.clone();
    let waiting = proposal.waiting;
    let model = model_summary(&proposal);
    let more_actions = signal(false);
    let attention_id =
        (proposal.gmail_sync_status.as_deref() == Some("attention")).then(|| id.clone());

    let top_nav = view()
        .flex_row()
        .items_center()
        .justify_between()
        .px_4()
        .py_1p5()
        .child(action_btn("‹ Mail", move || {
            open_thread.proposal.set(None);
            open_thread.detail.set(None);
            open_thread.detail_error.set(None);
            open_thread.pending_confirm.set(None);
        }))
        .child(
            button()
                .px_3()
                .py_1p5()
                .rounded_full()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().border)
                .bg(move || nsh_base::theme().card)
                .on_press(move || more_actions.update(|v| *v = !*v))
                .child(
                    text(move || {
                        if more_actions.get() {
                            "Hide actions"
                        } else {
                            "Category & More"
                        }
                    })
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD),
                ),
        );

    let bottom_triage_bar = crate::nsh_ui::surface_card()
        .mx_4()
        .mb_2()
        .px_2()
        .py_1p5()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(action_btn("Archive", move || {
            request_or_run(id_archive.clone(), MailAction::Archive, open_thread);
        }))
        .child(action_btn(
            if waiting { "Clear waiting" } else { "Waiting" },
            move || {
                let act = if waiting {
                    MailAction::ClearWaiting
                } else {
                    MailAction::Waiting
                };
                request_or_run(id_wait.clone(), act, open_thread);
            },
        ))
        .child(action_btn("Dismiss", move || {
            request_or_run(id_dismiss.clone(), MailAction::Dismiss, open_thread);
        }));

    let id_for_confirm = id.clone();
    let confirm_strip = dynamic(move || match open_thread.pending_confirm.get() {
        Some(pending) => {
            let prompt = pending.confirmation(1).unwrap_or_default();
            let id_yes = id_for_confirm.clone();
            crate::nsh_ui::surface_card()
                .mx_4()
                .px_4()
                .py_2p5()
                .gap_2()
                .child(text(prompt).text_sm().font_weight(FontWeight::MEDIUM))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action_btn("Confirm", move || {
                            open_thread.pending_confirm.set(None);
                            apply_action(id_yes.clone(), pending.clone());
                        }))
                        .child(action_btn("Cancel", move || {
                            open_thread.pending_confirm.set(None);
                        })),
                )
                .into_element()
        }
        None => view().into_element(),
    });

    let id_for_cats = id.clone();
    let current_stream = proposal.stream.clone();
    let secondary_drawer = dynamic(move || {
        more_actions.get().then(|| {
            let mut category_buttons = view().flex_row().flex_wrap().gap_1p5();
            for (key, label) in CATEGORIES {
                let cat_id = id_for_cats.clone();
                let cat_key = (*key).to_owned();
                category_buttons = category_buttons.child(crate::nsh_ui::pill_tab(
                    format!("→ {label}"),
                    current_stream == *key,
                    move || {
                        request_or_run(
                            cat_id.clone(),
                            MailAction::Categorize(cat_key.clone()),
                            open_thread,
                        );
                    },
                ));
            }
            let trash_id = id_trash.clone();
            let secret_id = id_secret.clone();
            crate::nsh_ui::surface_card()
                .mx_4()
                .mb_2()
                .p_3()
                .gap_2()
                .child(crate::nsh_ui::section_heading("MOVE TO STREAM"))
                .child(category_buttons)
                .child(crate::nsh_ui::section_heading("SENSITIVE & DESTRUCTIVE"))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action_btn("Trash", move || {
                            request_or_run(trash_id.clone(), MailAction::Trash, open_thread);
                        }))
                        .child(action_btn("Secret (c3)", move || {
                            request_or_run(
                                secret_id.clone(),
                                MailAction::SetDataClass("c3".into()),
                                open_thread,
                            );
                        })),
                )
                .into_element()
        })
    });

    let model_strip = model.map(|(line, proposed)| {
        let id_accept = id.clone();
        crate::nsh_ui::surface_card()
            .mx_4()
            .mb_2()
            .px_3()
            .py_2()
            .gap_1()
            .child(text(line).text_xs())
            .child(proposed.map(|stream| {
                let label = format!("Accept {}", category_label(&stream));
                action_btn(label, move || {
                    request_or_run(
                        id_accept.clone(),
                        MailAction::Categorize(stream.clone()),
                        open_thread,
                    );
                })
            }))
    });

    let attention_strip = attention_id.map(|att_id| {
        let retry_id = att_id.clone();
        let cancel_id = att_id;
        crate::nsh_ui::surface_card()
            .mx_4()
            .mb_2()
            .px_3()
            .py_2()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .child(text("Automatic Gmail retries stopped.").text_xs())
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(action_btn("Retry", move || {
                        apply_gmail_job(retry_id.clone(), "retry");
                    }))
                    .child(action_btn("Cancel sync", move || {
                        apply_gmail_job(cancel_id.clone(), "cancel");
                    })),
            )
    });

    let fallback_snippet = proposal
        .snippet
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "No preview text stored for this thread.".into());
    let head_title = subject(&proposal);
    let head_sub = row_subtitle(&proposal);

    view()
        .flex_1()
        .child(top_nav)
        .child(confirm_strip)
        .child(secondary_drawer)
        .child(model_strip)
        .child(attention_strip)
        .child(
            scroll().flex_1().child(
                view()
                    .px_4()
                    .pb_4()
                    .gap_2()
                    .child(
                        text(head_title)
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .select_text(),
                    )
                    .child(text(head_sub).text_sm())
                    .child(dynamic(move || {
                        let body = match open_thread.detail.with(|d| d.clone()) {
                            Some(detail) => detail
                                .content
                                .filter(|t| !t.trim().is_empty())
                                .or_else(|| detail.digest.and_then(|d| d.text))
                                .filter(|t| !t.trim().is_empty())
                                .unwrap_or_else(|| fallback_snippet.clone()),
                            None => match open_thread.detail_error.get() {
                                Some(err) => format!("{fallback_snippet}\n\n({err})"),
                                None => fallback_snippet.clone(),
                            },
                        };
                        nsh_markdown_view()
                            .markdown(body)
                            .selectable(true)
                            .links(TextViewLinks::Open)
                            .into_element()
                    })),
            ),
        )
        .child(bottom_triage_bar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone_mail::format_bulk_notice;
    use sjel_gpui_core::mail::BulkResult;
    use sjel_gpui_core::mail::{BulkFailure, ModelVerdict};

    fn sample_proposal() -> MailProposal {
        MailProposal {
            id: "thread_1".into(),
            from_addr: Some("Ada <ada@example.com>".into()),
            subject: Some(" Tax return 2026 ".into()),
            snippet: Some("Please review the attached return.".into()),
            internal_date: Some("2026-11-08T09:30:00Z".into()),
            stream: "aktiv".into(),
            status: "proposed".into(),
            gmail_sync_status: Some("synced".into()),
            gmail_sync_action: None,
            waiting: true,
            relevance: Vec::new(),
            authenticity: None,
            model: Some(ModelVerdict {
                mode: "shadow".into(),
                state: "generated".into(),
                model_stream: Some("steuern".into()),
                confidence_bp: Some(9200),
                rationale: Some("Mentions tax return.".into()),
                held_reason: None,
                classification_version: "v1".into(),
            }),
        }
    }

    #[test]
    fn visible_mail_reuses_rows_and_tracks_filter_and_source_changes() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let (owner, ()) = nsh_core::element::create_root(|| {
            let pending = sample_proposal();
            let mut archived = pending.clone();
            archived.id = "archived".into();
            archived.status = "archived".into();
            archived.stream = "steuern".into();
            let rows = signal(vec![pending, archived]);
            let status = signal(StatusFilter::Pending);
            let category = signal(None);
            let visible = visible_mail(rows, status, category);
            assert_eq!(visible.with(|rows| rows[0].id.clone()), "thread_1");
            let retained = visible.with(|rows| rows.as_ptr());
            for _ in 0..100 {
                assert_eq!(visible.with(Vec::len), 1);
                assert_eq!(visible.with(|rows| rows.as_ptr()), retained);
            }
            status.set(StatusFilter::All);
            assert_eq!(visible.with(Vec::len), 2);
            category.set(Some("steuern"));
            assert_eq!(visible.with(|rows| rows[0].id.clone()), "archived");
            status.set(StatusFilter::Pending);
            assert_eq!(visible.with(Vec::len), 0);
            rows.update(|rows| rows[1].status = "proposed".into());
            assert_eq!(visible.with(|rows| rows[0].id.clone()), "archived");
            rows.update(|rows| rows.retain(|row| row.id != "archived"));
            assert_eq!(visible.with(Vec::len), 0);
        });
        owner.dispose();
    }

    #[test]
    fn row_helpers_format_subject_subtitle_and_model_verdict() {
        let m = sample_proposal();
        assert_eq!(subject(&m), "Tax return 2026");
        assert_eq!(
            row_subtitle(&m),
            "Active · Ada <ada@example.com> · 2026-11-08 · Waiting"
        );
        let (line, proposed) = model_summary(&m).expect("model summary present");
        assert_eq!(proposed.as_deref(), Some("steuern"));
        assert!(line.contains(&format!("Model proposes {}", category_label("steuern"))));
        assert!(line.contains("92%"));
        assert!(line.contains("Mentions tax return."));
    }

    #[test]
    fn bulk_notice_formats_success_redaction_and_failure() {
        let ok = BulkResult {
            succeeded: vec!["a".into()],
            failures: Vec::new(),
            narrowed: 0,
        };
        assert_eq!(
            format_bulk_notice(&MailAction::Archive, &ok),
            "Archived in Gmail."
        );
        let redacted = BulkResult {
            succeeded: vec!["a".into()],
            failures: Vec::new(),
            narrowed: 1,
        };
        assert!(
            format_bulk_notice(&MailAction::SetDataClass("c3".into()), &redacted)
                .contains("redacted")
        );
        let failed = BulkResult {
            succeeded: Vec::new(),
            failures: vec![BulkFailure {
                id: "a".into(),
                error: "gmail offline".into(),
            }],
            narrowed: 0,
        };
        assert!(format_bulk_notice(&MailAction::Trash, &failed).contains("gmail offline"));
    }
}
