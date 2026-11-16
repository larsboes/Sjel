//! Application ledger, status filtering, and career pipeline actions.

use gpui_kit::{div, prelude::*, px, rems, AnyElement, Context};
use sjel_gpui_core::cv::{
    filter_applications, last_status_date, CvApplication, CvView, CvWriteRequest,
};

use crate::ely::buttons::{ToggleButton, ToggleItem};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use crate::ely::theme::{ControlSize, Density};
use crate::primitives::card;
use crate::theme::Palette;

use super::CvPage;

fn application_columns(statuses: &[String]) -> [Column; 6] {
    [
        Column::new("company", "Company / Role").min_width(rems(14.)),
        Column::new("profile", "Profile / Language").width(rems(10.)),
        Column::new("status", "Status").width(rems(9.)),
        Column::new("date", "Updated").width(rems(7.)).end().muted(),
        Column::new("posting", "Posting").width(rems(8.)),
        Column::new("actions", "Actions").width(rems(22.)).actions(
            std::iter::once(RowAction::new("build", "Build CV")).chain(
                statuses.iter().map(|status| {
                    RowAction::new(format!("status:{status}"), format!("→ {status}"))
                }),
            ),
        ),
    ]
}

fn application_row(app: &CvApplication, statuses: &[String], busy: bool) -> Row {
    let tone = match app.status.as_str() {
        "offer" | "accepted" => Tone::Success,
        "interviewing" | "interview" => Tone::Accent,
        "rejected" => Tone::Danger,
        _ => Tone::Neutral,
    };
    let actions = if busy {
        Vec::new()
    } else {
        std::iter::once("build".into())
            .chain(
                statuses
                    .iter()
                    .filter(|status| *status != &app.status)
                    .take(2)
                    .map(|status| format!("status:{status}").into()),
            )
            .collect()
    };
    Row::new(
        app.slug.clone(),
        [
            Cell::Lines(app.company.clone().into(), app.role.clone().into()),
            Cell::Text(format!("{} · {}", app.profile, app.lang).into()),
            Cell::Tag(app.status.clone().into(), tone),
            Cell::Text(last_status_date(app).into()),
            match app.url.as_ref().filter(|url| !url.is_empty()) {
                Some(url) => Cell::Link("Open posting".into(), url.clone().into()),
                None => Cell::Empty,
            },
            Cell::Actions(actions),
        ],
    )
}

fn application_action(view: &CvView, slug: &str, action: &str) -> Option<CvWriteRequest> {
    let app = view.applications.iter().find(|app| app.slug == slug)?;
    let mut request = CvWriteRequest {
        slug: app.slug.clone(),
        ..Default::default()
    };
    if action == "build" {
        request.action = "app-build".into();
    } else {
        let status = action.strip_prefix("status:")?;
        if status == app.status || !view.statuses.iter().any(|known| known == status) {
            return None;
        }
        request.action = "app-status".into();
        request.status = status.into();
    }
    Some(request)
}

impl CvPage {
    pub(super) fn render_applications_table(
        &self,
        view: &CvView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = !self.busy.is_empty();
        let apps = if self.filter_status == "all" {
            view.applications.iter().collect()
        } else {
            filter_applications(view, &self.filter_status)
        };
        let filters = std::iter::once((
            "all".to_owned(),
            format!("All ({})", view.applications.len()),
        ))
        .chain(view.statuses.iter().map(|status| {
            let count = view
                .applications
                .iter()
                .filter(|app| app.status == *status)
                .count();
            (status.clone(), format!("{status} ({count})"))
        }));
        let toggles = filters.map(|(status, label)| {
            ToggleButton::new(
                format!("cv-status-{status}"),
                ToggleItem::new(status.clone()).label(label),
                status == self.filter_status,
            )
            .size(ControlSize::Sm)
            .on_toggle({
                let page = cx.entity();
                move |_, _, cx| {
                    page.update(cx, |page, cx| {
                        page.filter_status = status.clone();
                        cx.notify();
                    })
                }
            })
        });
        let page = cx.entity();
        let content = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().child("Applications Ledger"))
            .child(div().flex().flex_wrap().gap(px(6.)).children(toggles))
            .child(
                DataTable::new("cv-applications", application_columns(&view.statuses))
                    .label("Applications ledger")
                    .rows(std::rc::Rc::new(
                        apps.into_iter()
                            .map(|app| application_row(app, &view.statuses, busy))
                            .collect(),
                    ))
                    .density(Density::Comfortable)
                    .paged(20)
                    .empty("No applications found matching this status.")
                    .on_action(move |slug, action, window, cx| {
                        page.update(cx, |page, cx| {
                            if !page.busy.is_empty() {
                                return;
                            }
                            let request = page
                                .cv
                                .value
                                .as_ref()
                                .and_then(|view| application_action(view, slug, action));
                            if let Some(request) = request {
                                page.run_action(request, window, cx);
                            }
                        })
                    }),
            );
        card(content.into_any_element(), palette)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CvView {
        serde_json::from_value(serde_json::json!({
            "master": "master.yaml", "master_mtime": 0, "profiles": ["default"],
            "statuses": ["draft", "applied", "interview", "offer"], "pdfs": [],
            "applications": [{"slug": "acme-engineer", "company": "Acme", "role": "Engineer",
                "profile": "default", "lang": "en", "status": "interview", "log": [],
                "url": "https://example.org/job"}]
        }))
        .unwrap()
    }

    #[test]
    fn ledger_actions_keep_the_slug_and_validate_the_current_statuses() {
        let view = fixture();
        let build = application_action(&view, "acme-engineer", "build").unwrap();
        assert_eq!(
            (build.action.as_str(), build.slug.as_str()),
            ("app-build", "acme-engineer")
        );
        let status = application_action(&view, "acme-engineer", "status:offer").unwrap();
        assert_eq!(
            (status.action.as_str(), status.status.as_str()),
            ("app-status", "offer")
        );
        assert!(application_action(&view, "missing", "build").is_none());
        assert!(application_action(&view, "acme-engineer", "status:unknown").is_none());
        assert!(application_action(&view, "acme-engineer", "status:interview").is_none());
    }

    #[test]
    fn ledger_rows_keep_status_tone_posting_link_and_busy_actions() {
        let view = fixture();
        let app = &view.applications[0];
        let expected = |actions| {
            Row::new(
                "acme-engineer",
                [
                    Cell::Lines("Acme".into(), "Engineer".into()),
                    Cell::Text("default · en".into()),
                    Cell::Tag("interview".into(), Tone::Accent),
                    Cell::Text("".into()),
                    Cell::Link("Open posting".into(), "https://example.org/job".into()),
                    Cell::Actions(actions),
                ],
            )
        };
        assert_eq!(
            application_row(app, &view.statuses, false),
            expected(vec![
                "build".into(),
                "status:draft".into(),
                "status:applied".into()
            ])
        );
        assert_eq!(application_row(app, &view.statuses, true), expected(vec![]));
    }

    #[gpui_kit::test]
    fn status_toggles_filter_the_production_ledger(cx: &mut gpui_kit::TestAppContext) {
        let (page, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui_kit::size(px(1400.), px(1200.))),
            |_, _| {
                let mut page = CvPage::new(crate::data::Repo::new(Err(
                    sjel_gpui_core::shell::ShellError::NotSetUp,
                )));
                page.cv.value = Some(fixture());
                page
            },
        );
        let offer = cx
            .debug_bounds("toggle offer")
            .expect("offer filter renders");
        cx.simulate_click(offer.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.filter_status, "offer"));
        assert!(cx.debug_bounds("table-row acme-engineer").is_none());
        let all = cx
            .debug_bounds("toggle all")
            .expect("all filter remains reachable");
        cx.simulate_click(all.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.filter_status, "all"));
        assert!(cx.debug_bounds("table-row acme-engineer").is_some());
    }

    #[gpui_kit::test]
    fn capture_keeps_typed_fields_when_the_shell_is_unavailable(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::Focusable;

        let (page, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui_kit::size(px(1400.), px(1600.))),
            |_, _| {
                let mut page = CvPage::new(crate::data::Repo::new(Err(
                    sjel_gpui_core::shell::ShellError::NotSetUp,
                )));
                page.cv.value = Some(fixture());
                page
            },
        );
        cx.update(|window, cx| {
            page.read(cx)
                .company_input
                .as_ref()
                .unwrap()
                .0
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
        cx.simulate_input("Example Company");
        cx.update(|window, cx| {
            page.read(cx)
                .role_input
                .as_ref()
                .unwrap()
                .0
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
        cx.simulate_input("Engineer");
        cx.run_until_parked();
        let submit = cx
            .debug_bounds("cv-capture-submit")
            .expect("capture action renders");
        cx.simulate_click(submit.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let toaster = crate::feedback::toaster(window, cx);
            assert_eq!(
                toaster.read(cx).shown().len(),
                1,
                "the failed capture reports through the write boundary"
            );
        });
        page.read_with(cx, |page, cx| {
            assert_eq!(
                page.company_input.as_ref().unwrap().0.read(cx).text(),
                "Example Company"
            );
            assert_eq!(
                page.role_input.as_ref().unwrap().0.read(cx).text(),
                "Engineer"
            );
            assert!(page.busy.is_empty());
        });
    }
}
