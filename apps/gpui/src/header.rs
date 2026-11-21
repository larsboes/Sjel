//! The header: the area's name, the page's one line, its tab strip when it has more than one tab,
//! and the two window-wide actions.
//!
//! It reads the workspace and dispatches its actions, so the header holds no state of its own.

use std::time::Duration;

use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::area::Area;

use crate::route::Page;
use crate::workspace::Workspace;
use sjel_gpui_core::travel::{TravelRoute, TripSection};
use sjel_gpui_ui::backup::BackupTab;
use sjel_gpui_ui::calendar::View as CalendarView;
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant, IconButton};
use sjel_gpui_ui::ely::data_display::{Badge, Tone};
use sjel_gpui_ui::ely::primitives::IconName;
use sjel_gpui_ui::ely::theme::ControlSize;
use sjel_gpui_ui::feed::Tab as FeedTab;
use sjel_gpui_ui::finance::View as FinanceView;
use sjel_gpui_ui::inventory::InventoryTab;
use sjel_gpui_ui::primitives::sub_tabs;
use sjel_gpui_ui::projects::{ProjectSection, ProjectsTab};
use sjel_gpui_ui::research::ResearchTab;
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::systems::SystemsTab;
use sjel_gpui_ui::theme::Palette;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let checked = view
        .health
        .age()
        .map(|elapsed| format!("checked {} ago", ago(elapsed)));
    let area = Area::of(view.page);
    let tabs = area.and_then(|area| tab_strip(area, view, palette, cx));
    let scratch_open = view.right_panel.is_open() && view.right_panel.seat() == Seat::Scratchpad;
    let scratch_count = view.scratchpad.read(cx).state.parse_targets().len();
    let scratch_label = if scratch_count > 0 {
        format!("Scratchpad ({scratch_count})")
    } else {
        "Scratchpad".to_owned()
    };
    // Settings › General says the same at length, so the pill stays out of Settings.
    let update_pill = view
        .update_info
        .as_ref()
        .filter(|info| info.is_update_available() && view.page != Page::Settings)
        .map(|info| {
            div().flex_none().mr_2().child(crate::update::header_pill(
                info,
                view.rebuild_state == crate::update::RebuildState::Building,
                palette,
                cx.entity(),
            ))
        });
    // Every page on the demo node says so, so synthetic data is never read as real.
    let dev_badge = sjel_gpui_core::environment::current().dev.then(|| {
        div()
            .flex_none()
            .mr_2()
            .child(Badge::new("DEV NODE").tone(Tone::Warning))
    });
    let shell_recovery = view.health.error.as_ref().map(|_| {
        Button::new("restart-shell", "Restart shell")
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .loading(view.busy.contains("sjel-status-recovery"))
            .on_click(cx.listener(|view, _, window, cx| view.restart_shell(window, cx)))
    });
    let area_title: AnyElement = if let Some(project_name) = (view.page == Page::Projects)
        .then(|| view.projects.read(cx).active_project_name())
        .flatten()
    {
        let owner = cx.entity();
        sjel_gpui_ui::ely::navigation::Breadcrumb::new(
            "header-projects-breadcrumb",
            [
                sjel_gpui_ui::ely::navigation::Crumb::new("projects", "Projects"),
                sjel_gpui_ui::ely::navigation::Crumb::new("current", project_name),
            ],
        )
        .on_select(move |level, _, _, cx| {
            if level == 0 {
                owner.update(cx, |view, cx| {
                    view.projects
                        .update(cx, |project, cx| project.close_project(cx));
                });
            }
        })
        .into_any_element()
    } else if let Some(trip_title) = (view.page == Page::Travel)
        .then(|| view.travel.read(cx).active_plan_title())
        .flatten()
    {
        let owner = cx.entity();
        sjel_gpui_ui::ely::navigation::Breadcrumb::new(
            "header-travel-breadcrumb",
            [
                sjel_gpui_ui::ely::navigation::Crumb::new("trips", "Trips"),
                sjel_gpui_ui::ely::navigation::Crumb::new("current", trip_title),
            ],
        )
        .on_select(move |level, _, _, cx| {
            if level == 0 {
                owner.update(cx, |view, cx| {
                    view.travel
                        .update(cx, |travel, cx| travel.open_route(TravelRoute::Trips, cx));
                });
            }
        })
        .into_any_element()
    } else {
        div()
            .child(area.map_or(view.page.label(), Area::label))
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .px_8()
        .pt(px(44.))
        .pb_5()
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .truncate()
                                .child(area_title),
                        )
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .text_color(rgb(palette.muted))
                                .child(subtitle(view, cx)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .children(dev_badge)
                        .children(update_pill)
                        .children(shell_recovery)
                        .children(checked.map(|label| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .mr_2()
                                .child(label)
                        }))
                        .when(matches!(view.page, Page::Home | Page::Context), |el| {
                            let mode = view.home_prefs.mode;
                            let label = if mode == crate::home::HomeMode::Auto {
                                "Customize".to_string()
                            } else {
                                format!("Customize · {}", mode.label())
                            };
                            el.child(
                                Button::new("home-customize-toggle", label)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ControlSize::Sm)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.home_customize_open = !view.home_customize_open;
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            Button::new("header-scratchpad-toggle", scratch_label)
                                .size(ControlSize::Sm)
                                .variant(if scratch_open {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.toggle_scratchpad(window, cx);
                                })),
                        )
                        .child(
                            IconButton::new("refresh", IconName::RotateCw)
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .disabled(view.health.loading || view.agent.loading)
                                .tooltip("Refresh (⌘R)")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.refresh_current_page(cx);
                                })),
                        )
                        .child(
                            IconButton::new("settings", IconName::Settings)
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .tooltip("Settings (⌘,)")
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.open_settings(None, cx)),
                                ),
                        ),
                ),
        )
        // Its own row, so eight tabs can never squeeze the name of the area they belong to.
        .children(tabs)
}

/// The area's tabs and unified view sub-menus, so page-internal switches sit in the header rather
/// than stacking a second tab bar inside the page body.
fn tab_strip(
    area: Area,
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    let current = view.page;
    let pages = area.pages();

    if area == Area::Projects {
        let projects = view.projects.read(cx);
        if projects.active_project_name().is_some() {
            let section = projects.project_section;
            let owner = cx.entity();
            return Some(sub_tabs(
                "header-project-sections",
                vec![
                    ("Overview", None, section == ProjectSection::Note),
                    ("Tasks", None, section == ProjectSection::Tasks),
                    ("Notes", None, section == ProjectSection::Notes),
                    ("Mail review", None, section == ProjectSection::MailReview),
                    (
                        "Inventory links",
                        None,
                        section == ProjectSection::InventoryLinks,
                    ),
                ],
                move |ix, _, cx| {
                    owner.update(cx, |view, cx| {
                        view.projects.update(cx, |projects, cx| {
                            let target = match ix {
                                0 => ProjectSection::Note,
                                1 => ProjectSection::Tasks,
                                2 => ProjectSection::Notes,
                                3 => ProjectSection::MailReview,
                                _ => ProjectSection::InventoryLinks,
                            };
                            projects.set_project_section(target, cx);
                        });
                    });
                },
            ));
        }

        let tab = projects.tab;
        let projects_count = projects
            .projects
            .value
            .as_ref()
            .map(|list| list.iter().filter(|p| !p.archived).count())
            .filter(|&c| c > 0);
        let tasks_count = projects
            .all_tasks
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let notes_count = projects
            .all_notes
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let owner = cx.entity();
        let owner_browse = cx.entity();
        return Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(sub_tabs(
                    "header-projects-tabs",
                    vec![
                        ("Projects", projects_count, tab == ProjectsTab::Hubs),
                        ("All Tasks", tasks_count, tab == ProjectsTab::AllTasks),
                        ("All Notes", notes_count, tab == ProjectsTab::AllNotes),
                    ],
                    move |ix, _, cx| {
                        owner.update(cx, |view, cx| {
                            view.projects.update(cx, |projects, cx| {
                                projects.tab = match ix {
                                    1 => ProjectsTab::AllTasks,
                                    2 => ProjectsTab::AllNotes,
                                    _ => ProjectsTab::Hubs,
                                };
                                cx.notify();
                            });
                        });
                    },
                ))
                .child(
                    Button::new("header-projects-registry", "Browse objects")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            owner_browse.update(cx, |workspace, cx| {
                                workspace.go(Page::Record, cx);
                                workspace.ensure_record_page(cx).update(cx, |page, cx| {
                                    page.open("registry:projects".into(), cx);
                                });
                            });
                        }),
                )
                .into_any_element(),
        );
    }

    if area == Area::Inbox {
        let inbox_count = view
            .inbox
            .read(cx)
            .rows
            .value
            .as_ref()
            .map(|rows| rows.iter().filter(|c| c.status == "new").count())
            .filter(|&c| c > 0);
        let owner_area = cx.entity();
        let area_strip = sub_tabs(
            "inbox-area",
            vec![
                ("Inbox", inbox_count, current == Page::Inbox),
                ("Feed", None, current == Page::Feed),
            ],
            move |ix, _, cx| {
                let target = if ix == 0 { Page::Inbox } else { Page::Feed };
                owner_area.update(cx, |view, cx| view.go(target, cx));
            },
        );
        if current == Page::Feed {
            let active_feed_tab = view
                .feed
                .as_ref()
                .map(|f| f.read(cx).tab)
                .unwrap_or(FeedTab::Reading);
            let reading_new = view
                .feed
                .as_ref()
                .map(|f| f.read(cx).count(sjel_gpui_ui::feed::Filter::New))
                .filter(|&c| c > 0);
            let owner_feed = cx.entity();
            let owner_clip = cx.entity();
            return Some(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(area_strip)
                            .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                            .child(sub_tabs(
                                "feed-sub",
                                vec![
                                    ("Reading", reading_new, active_feed_tab == FeedTab::Reading),
                                    ("Mail", None, active_feed_tab == FeedTab::Mail),
                                    ("Discover", None, active_feed_tab == FeedTab::Discover),
                                    ("Clips", None, active_feed_tab == FeedTab::Clips),
                                    ("Review", None, active_feed_tab == FeedTab::Review),
                                    ("Library", None, active_feed_tab == FeedTab::Library),
                                ],
                                move |ix, _, cx| {
                                    let target = match ix {
                                        0 => FeedTab::Reading,
                                        1 => FeedTab::Mail,
                                        2 => FeedTab::Discover,
                                        3 => FeedTab::Clips,
                                        4 => FeedTab::Review,
                                        _ => FeedTab::Library,
                                    };
                                    owner_feed.update(cx, |view, cx| {
                                        view.ensure_feed(cx)
                                            .update(cx, |feed, cx| feed.set_tab(target, cx));
                                    });
                                },
                            )),
                    )
                    .child(
                        Button::new("header-feed-quick-clip", "+ Quick Clip")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, window, cx| {
                                owner_clip.update(cx, |view, cx| {
                                    view.ensure_feed(cx).update(cx, |feed, cx| {
                                        feed.open_quick_clip(window, cx);
                                    });
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        return Some(area_strip);
    }

    if area == Area::Calendar {
        let (active_view, proposals_n) = view
            .calendar_page
            .as_ref()
            .map(|cp| {
                let c = cp.read(cx);
                (c.view, c.pending_proposals_count())
            })
            .unwrap_or((CalendarView::Month, 0));
        let owner_cal = cx.entity();
        let owner_prop = cx.entity();
        return Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(sub_tabs(
                    "header-calendar-views",
                    vec![
                        ("Day", None, active_view == CalendarView::Day),
                        ("Week", None, active_view == CalendarView::Week),
                        ("Month", None, active_view == CalendarView::Month),
                    ],
                    move |ix, _, cx| {
                        let target = match ix {
                            0 => CalendarView::Day,
                            1 => CalendarView::Week,
                            _ => CalendarView::Month,
                        };
                        owner_cal.update(cx, |view, cx| {
                            view.ensure_calendar_page(cx)
                                .update(cx, |page, cx| page.set_view(target, cx));
                        });
                    },
                ))
                .when(proposals_n > 0, |el| {
                    el.child(
                        Button::new(
                            "header-calendar-proposals",
                            format!("Proposals ({proposals_n})"),
                        )
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            owner_prop.update(cx, |view, cx| {
                                view.ensure_calendar_page(cx)
                                    .update(cx, |page, cx| page.toggle_proposals(cx));
                            });
                        }),
                    )
                })
                .into_any_element(),
        );
    }

    if area == Area::Travel {
        let travel = view.travel.read(cx);
        let in_trip = travel.active_plan_title().is_some();
        let owner_nav = cx.entity();
        let owner_new = cx.entity();
        let left = if in_trip {
            let section = travel.active_section();
            sub_tabs(
                "header-travel-trip-sections",
                TripSection::ALL
                    .iter()
                    .map(|&s| (s.label(), None, s == section))
                    .collect(),
                move |ix, _, cx| {
                    if let Some(&target) = TripSection::ALL.get(ix) {
                        owner_nav.update(cx, |view, cx| {
                            view.travel
                                .update(cx, |travel, cx| travel.set_section(target, cx));
                        });
                    }
                },
            )
        } else {
            let upcoming = travel.upcoming_count();
            let route = travel.route().clone();
            sub_tabs(
                "header-travel-routes",
                vec![
                    ("Map & Places", None, matches!(route, TravelRoute::Map)),
                    (
                        "Trips",
                        (upcoming > 0).then_some(upcoming),
                        matches!(route, TravelRoute::Trips),
                    ),
                    (
                        "Connections",
                        None,
                        matches!(route, TravelRoute::Connections),
                    ),
                ],
                move |ix, _, cx| {
                    let next = match ix {
                        0 => TravelRoute::Map,
                        2 => TravelRoute::Connections,
                        _ => TravelRoute::Trips,
                    };
                    owner_nav.update(cx, |view, cx| {
                        view.travel
                            .update(cx, |travel, cx| travel.open_route(next, cx));
                    });
                },
            )
        };
        return Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(left)
                .child(
                    Button::new("header-travel-new-trip", "+ New trip")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, window, cx| {
                            owner_new.update(cx, |view, cx| {
                                view.travel
                                    .update(cx, |travel, cx| travel.open_new_trip(window, cx));
                            });
                        }),
                )
                .into_any_element(),
        );
    }

    if !area.has_tabs() {
        return None;
    }

    if area == Area::Documents {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum DocGroup {
            Library,
            Ingest,
            Reference,
        }
        impl DocGroup {
            fn label(self) -> &'static str {
                match self {
                    Self::Library => "Library",
                    Self::Ingest => "Ingest",
                    Self::Reference => "Reference",
                }
            }
            fn pages(self) -> &'static [Page] {
                match self {
                    Self::Library => &[Page::Documents, Page::Media],
                    Self::Ingest => &[Page::Import],
                    Self::Reference => &[Page::Docs],
                }
            }
            fn of(page: Page) -> Self {
                match page {
                    Page::Documents | Page::Media => Self::Library,
                    Page::Import => Self::Ingest,
                    Page::Docs => Self::Reference,
                    _ => Self::Library,
                }
            }
        }
        let current_group = DocGroup::of(current);
        let groups = [DocGroup::Library, DocGroup::Ingest, DocGroup::Reference];
        let sub_pages = current_group.pages();
        let owner_group = cx.entity();
        let owner_sub = cx.entity();
        return Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(sub_tabs(
                    "docs-category",
                    groups
                        .into_iter()
                        .map(|g| (g.label(), Some(g.pages().len()), g == current_group))
                        .collect(),
                    move |ix, _, cx| {
                        let target_page = match ix {
                            0 => Page::Documents,
                            1 => Page::Import,
                            _ => Page::Docs,
                        };
                        owner_group.update(cx, |view, cx| view.go(target_page, cx));
                    },
                ))
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "docs-sub",
                    sub_pages
                        .iter()
                        .map(|page| (page.label(), None, *page == current))
                        .collect(),
                    move |ix, _, cx| {
                        if let Some(page) = sub_pages.get(ix) {
                            owner_sub.update(cx, |view, cx| view.go(*page, cx));
                        }
                    },
                ))
                .into_any_element(),
        );
    }

    if area == Area::Life {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum LifeGroup {
            Household,
            Personal,
        }
        impl LifeGroup {
            fn label(self) -> &'static str {
                match self {
                    Self::Household => "Household & Money",
                    Self::Personal => "People & Career",
                }
            }
            fn pages(self) -> &'static [Page] {
                match self {
                    Self::Household => &[Page::Finance, Page::Inventory],
                    Self::Personal => &[Page::People, Page::Cv, Page::Posts],
                }
            }
            fn of(page: Page) -> Self {
                match page {
                    Page::Finance | Page::Inventory => Self::Household,
                    _ => Self::Personal,
                }
            }
        }
        let current_group = LifeGroup::of(current);
        let groups = [LifeGroup::Personal, LifeGroup::Household];
        let sub_pages = current_group.pages();
        let owner_group = cx.entity();
        let owner_sub = cx.entity();
        let mut strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(sub_tabs(
                "life-category",
                groups
                    .into_iter()
                    .map(|g| (g.label(), Some(g.pages().len()), g == current_group))
                    .collect(),
                move |ix, _, cx| {
                    let target_page = match ix {
                        0 => Page::People,
                        _ => Page::Finance,
                    };
                    owner_group.update(cx, |view, cx| view.go(target_page, cx));
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
            .child(sub_tabs(
                "life-sub",
                sub_pages
                    .iter()
                    .map(|page| (page.label(), None, *page == current))
                    .collect(),
                move |ix, _, cx| {
                    if let Some(page) = sub_pages.get(ix) {
                        owner_sub.update(cx, |view, cx| view.go(*page, cx));
                    }
                },
            ));

        if current == Page::Finance {
            let (active_view, tx_count, subs_count, import_count, ideas_count) = view
                .finance_page
                .as_ref()
                .map(|fp| {
                    let f = fp.read(cx);
                    let (tx, subs, imp, ideas) = f.badge_counts();
                    (f.view, tx, subs, imp, ideas)
                })
                .unwrap_or((FinanceView::Overview, None, None, None, None));
            let owner_fin = cx.entity();
            let owner_csv = cx.entity();
            strip = div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(strip)
                        .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                        .child(sub_tabs(
                            "finance-header-sub",
                            vec![
                                ("Overview", None, active_view == FinanceView::Overview),
                                (
                                    "Transactions",
                                    tx_count,
                                    active_view == FinanceView::Transactions,
                                ),
                                (
                                    "Subscriptions",
                                    subs_count,
                                    active_view == FinanceView::Subscriptions,
                                ),
                                (
                                    "Import & Review",
                                    import_count.filter(|&c| c > 0),
                                    active_view == FinanceView::ImportReview,
                                ),
                                (
                                    "Ideas",
                                    ideas_count.filter(|&c| c > 0),
                                    active_view == FinanceView::Ideas,
                                ),
                            ],
                            move |ix, _, cx| {
                                let target = match ix {
                                    0 => FinanceView::Overview,
                                    1 => FinanceView::Transactions,
                                    2 => FinanceView::Subscriptions,
                                    3 => FinanceView::ImportReview,
                                    _ => FinanceView::Ideas,
                                };
                                owner_fin.update(cx, |view, cx| {
                                    view.ensure_finance_page(cx)
                                        .update(cx, |page, cx| page.set_view(target, cx));
                                });
                            },
                        )),
                )
                .child(
                    Button::new("header-finance-csv-import", "CSV Import…")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            owner_csv.update(cx, |view, cx| {
                                view.ensure_finance_page(cx)
                                    .update(cx, |page, cx| page.open_csv_import(cx));
                            });
                        }),
                );
        } else if current == Page::Inventory {
            let (active_tab, owned_n, wish_n, layouts_n, mail_n) = view
                .inventory_page
                .as_ref()
                .map(|ip| {
                    let inv = ip.read(cx);
                    let owned = inv.rows.value.as_ref().map(|r| r.len()).unwrap_or(0);
                    let wish = inv
                        .wishlist
                        .value
                        .as_ref()
                        .map(|w| w.items.len())
                        .unwrap_or(0);
                    let layouts = inv.layouts.value.as_ref().map(|l| l.len()).unwrap_or(0);
                    let mail = inv.mail_review_pending_count();
                    (inv.tab, owned, wish, layouts, mail)
                })
                .unwrap_or((InventoryTab::Owned, 0, 0, 0, 0));
            let owner_inv = cx.entity();
            strip = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(strip)
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "inventory-header-sub",
                    vec![
                        (
                            "Owned Items",
                            (owned_n > 0).then_some(owned_n),
                            active_tab == InventoryTab::Owned,
                        ),
                        (
                            "Wishlist",
                            (wish_n > 0).then_some(wish_n),
                            active_tab == InventoryTab::Wishlist,
                        ),
                        (
                            "Interior Layouts",
                            (layouts_n > 0).then_some(layouts_n),
                            active_tab == InventoryTab::Interior,
                        ),
                        (
                            "Mail review",
                            (mail_n > 0).then_some(mail_n),
                            active_tab == InventoryTab::MailReview,
                        ),
                    ],
                    move |ix, _, cx| {
                        let next = match ix {
                            0 => InventoryTab::Owned,
                            1 => InventoryTab::Wishlist,
                            2 => InventoryTab::Interior,
                            _ => InventoryTab::MailReview,
                        };
                        owner_inv.update(cx, |view, cx| {
                            view.ensure_inventory_page(cx)
                                .update(cx, |page, cx| page.set_tab(next, cx));
                        });
                    },
                ));
        } else if current == Page::People {
            let (active_ix, total_contacts, dup_count) = view
                .people_page
                .as_ref()
                .map(|pp| {
                    let p = pp.read(cx);
                    let (tot, dup) = p.badge_counts();
                    (p.active_tab_index(), tot, dup)
                })
                .unwrap_or((0, 0, None));
            let owner_ppl = cx.entity();
            strip = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(strip)
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "people-header-sub",
                    vec![
                        (
                            "Directory",
                            (total_contacts > 0).then_some(total_contacts),
                            active_ix == 0,
                        ),
                        ("Duplicates", dup_count.filter(|&c| c > 0), active_ix == 1),
                        ("Network", None, active_ix == 2),
                    ],
                    move |ix, _, cx| {
                        owner_ppl.update(cx, |view, cx| {
                            view.ensure_people_page(cx)
                                .update(cx, |page, cx| page.set_tab_index(ix, cx));
                        });
                    },
                ));
        }

        return Some(strip.into_any_element());
    }

    if area == Area::Sjel {
        // Area::Sjel has 8 tabs: group them into 3 intuitive sub-menus to keep the header clean.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum SjelGroup {
            Runtime,
            Architecture,
            Catalog,
        }
        impl SjelGroup {
            fn label(self) -> &'static str {
                match self {
                    Self::Runtime => "Runtime",
                    Self::Architecture => "Architecture",
                    Self::Catalog => "Catalog",
                }
            }
            fn pages(self) -> &'static [Page] {
                match self {
                    Self::Runtime => {
                        &[Page::Capabilities, Page::Systems, Page::Backup, Page::Setup]
                    }
                    Self::Architecture => &[Page::SelfModel, Page::Packs, Page::Research],
                    Self::Catalog => &[Page::Software],
                }
            }
            fn of(page: Page) -> Self {
                match page {
                    Page::Capabilities | Page::Systems | Page::Backup | Page::Setup => {
                        Self::Runtime
                    }
                    Page::SelfModel | Page::Packs | Page::Research => Self::Architecture,
                    Page::Software => Self::Catalog,
                    _ => Self::Runtime,
                }
            }
        }

        let current_group = SjelGroup::of(current);
        let groups = [
            SjelGroup::Runtime,
            SjelGroup::Architecture,
            SjelGroup::Catalog,
        ];
        let sub_pages = current_group.pages();
        let owner_group = cx.entity();
        let owner_sub = cx.entity();

        let mut strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(sub_tabs(
                "sjel-category",
                groups
                    .into_iter()
                    .map(|g| (g.label(), Some(g.pages().len()), g == current_group))
                    .collect(),
                move |ix, _, cx| {
                    let target_page = match ix {
                        0 => Page::Capabilities,
                        1 => Page::SelfModel,
                        2 => Page::Software,
                        _ => Page::Capabilities,
                    };
                    owner_group.update(cx, |view, cx| view.go(target_page, cx));
                },
            ))
            .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
            .child(sub_tabs(
                "sjel-sub",
                sub_pages
                    .iter()
                    .map(|page| (page.label(), None, *page == current))
                    .collect(),
                move |ix, _, cx| {
                    if let Some(page) = sub_pages.get(ix) {
                        owner_sub.update(cx, |view, cx| view.go(*page, cx));
                    }
                },
            ));

        if current == Page::Systems {
            let (active_tab, findings, stale, pending) = view
                .systems
                .as_ref()
                .map(|sp| {
                    let s = sp.read(cx);
                    let (f, st, p) = s.badge_counts();
                    (s.tab, f, st, p)
                })
                .unwrap_or((SystemsTab::Overview, 0, 0, 0));
            let owner_sys = cx.entity();
            strip = strip
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "systems-header-sub",
                    vec![
                        (
                            "Overview",
                            (findings > 0).then_some(findings),
                            active_tab == SystemsTab::Overview,
                        ),
                        (
                            "Storage & Updates",
                            (stale > 0).then_some(stale),
                            active_tab == SystemsTab::Storage,
                        ),
                        (
                            "Agent Gate",
                            (pending > 0).then_some(pending),
                            active_tab == SystemsTab::Agent,
                        ),
                    ],
                    move |ix, _, cx| {
                        let tab = match ix {
                            0 => SystemsTab::Overview,
                            1 => SystemsTab::Storage,
                            _ => SystemsTab::Agent,
                        };
                        owner_sys.update(cx, |view, cx| {
                            view.ensure_systems(cx)
                                .update(cx, |page, cx| page.set_tab(tab, cx));
                        });
                    },
                ));
        } else if current == Page::Backup {
            let (active_tab, backups_n, targets_n, runs_n) = view
                .backup_page
                .as_ref()
                .map(|bp| {
                    let b = bp.read(cx);
                    let (bc, tc, rc) = b.badge_counts();
                    (b.tab, bc, tc, rc)
                })
                .unwrap_or((BackupTab::Contracts, 0, 0, 0));
            let owner_bak = cx.entity();
            strip = strip
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "backup-header-sub",
                    vec![
                        (
                            "Capability Contracts",
                            (backups_n > 0).then_some(backups_n),
                            active_tab == BackupTab::Contracts,
                        ),
                        (
                            "Targets & Cadence",
                            (targets_n > 0).then_some(targets_n),
                            active_tab == BackupTab::Targets,
                        ),
                        (
                            "Attempt Ledger",
                            (runs_n > 0).then_some(runs_n),
                            active_tab == BackupTab::History,
                        ),
                    ],
                    move |ix, _, cx| {
                        let next = match ix {
                            0 => BackupTab::Contracts,
                            1 => BackupTab::Targets,
                            _ => BackupTab::History,
                        };
                        owner_bak.update(cx, |view, cx| {
                            view.ensure_backup_page(cx)
                                .update(cx, |page, cx| page.set_tab(next, cx));
                        });
                    },
                ));
        } else if current == Page::Research {
            let active_tab = view
                .research_page
                .as_ref()
                .map(|rp| rp.read(cx).active_tab)
                .unwrap_or(ResearchTab::Articles);
            let owner_res = cx.entity();
            strip = strip
                .child(div().w(px(1.)).h(px(16.)).bg(rgb(palette.border)))
                .child(sub_tabs(
                    "research-header-sub",
                    vec![
                        ("Articles", None, active_tab == ResearchTab::Articles),
                        ("Projects", None, active_tab == ResearchTab::Projects),
                        ("Benchmarks", None, active_tab == ResearchTab::Benchmarks),
                    ],
                    move |ix, _, cx| {
                        let next = match ix {
                            0 => ResearchTab::Articles,
                            1 => ResearchTab::Projects,
                            _ => ResearchTab::Benchmarks,
                        };
                        owner_res.update(cx, |view, cx| {
                            view.ensure_research_page(cx)
                                .update(cx, |page, cx| page.switch_tab(next, cx));
                        });
                    },
                ));
        }

        return Some(strip.into_any_element());
    }

    let owner = cx.entity();
    Some(sub_tabs(
        "area",
        pages
            .iter()
            .map(|page| (page.label(), None, *page == current))
            .collect(),
        move |ix, _, cx| {
            if let Some(page) = pages.get(ix) {
                owner.update(cx, |view, cx| view.go(*page, cx));
            }
        },
    ))
}

/// The one line under the title: what this page is for, or what it is looking at.
fn subtitle(view: &Workspace, cx: &Context<Workspace>) -> SharedString {
    match view.page {
        // `/` and `/context` are one page, and the day block below carries its own date.
        Page::Home | Page::Context => {
            "What needs you now, what is urgent, and the day itself".into()
        }
        Page::Capabilities => view.endpoint(),
        Page::Projects => {
            if let Some(name) = view.projects.read(cx).active_project_name() {
                format!("{name} · tasks, notes and links").into()
            } else {
                "Projects, tasks and notes, in one place".into()
            }
        }
        Page::Travel => "Where you are, where you’re going, and where you’ll stay".into(),
        Page::Feed => "What the collectors brought in, ranked for you".into(),
        Page::Inbox => "Captured, not sorted yet".into(),
        Page::Calendar => "Your days, from the calendar capability".into(),
        Page::Documents => "Personal documents, filing and review".into(),
        Page::Backup => "Backup targets, rehearsal verification, and attempt history".into(),
        Page::Finance => "Personal ledger, accounts and recurring subscriptions".into(),
        Page::People => "Contact registry and personal relationship profiles".into(),
        Page::Inventory => {
            "Household inventory, wishlist budget, and interior spatial layout".into()
        }
        Page::Software => "Apps on this Mac: size, last used, and what to keep".into(),
        Page::Docs => "Architecture, capability contracts, and operating guides".into(),
        Page::Record => "A record and the Sjel data connected to it".into(),
        Page::Import => "Bring a folder into Sjel, each file to its owner".into(),
        Page::Systems => {
            "This Mac now: temperature, power, disk, updates and the agent's reach".into()
        }
        Page::Packs => "Agent skills across installed harnesses, drift status, and sync".into(),
        Page::SelfModel => {
            "Sjel about Sjel: units, compile-time couplings, and live topology".into()
        }
        Page::Cv => "CV builder, profile variations, applications ledger, and LinkedIn".into(),
        Page::Posts => "Post drafts, ordered Photos media, collages and upload exports".into(),
        Page::Media => "Exact-byte ingest gate, volume integrity ledger, and run inspection".into(),
        Page::Research => "Why Sjel exists, evidence behind it, and questions still open".into(),
        Page::Setup => {
            "What this machine still needs, primary replica, and companion devices".into()
        }
        Page::Settings => view.settings_section.summary().into(),
    }
}

fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_reads_in_the_largest_unit_that_fits() {
        assert_eq!(ago(Duration::ZERO), "0s");
        assert_eq!(ago(Duration::from_secs(59)), "59s");
        assert_eq!(ago(Duration::from_secs(60)), "1m");
        assert_eq!(ago(Duration::from_secs(7_200)), "2h");
    }
}
