use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString, Window,
};
use sjel_gpui_core::scratchpad::{now_unix_secs, ScratchTarget};
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::data_display::{Badge, Tone};
use sjel_gpui_ui::ely::forms::Input;
use sjel_gpui_ui::ely::primitives::Tooltip;
use sjel_gpui_ui::ely::theme::ControlSize;

use crate::route::Page;
use crate::workspace::Workspace;
use sjel_gpui_ui::primitives::{list, mono_num, progress_bar, row, section, surface_card};
use sjel_gpui_ui::theme::{tint, Palette};

/// Prompts an agent drafted for you to paste elsewhere. Copy empties the card; none draws nothing.
pub fn prompt_outbox(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if view.prompt_drafts.is_empty() {
        return None;
    }
    let rows = view
        .prompt_drafts
        .iter()
        .map(|draft| {
            let id = draft.id;
            row()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().truncate().text_sm().child(draft.title.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(draft.body.lines().next().unwrap_or_default().to_string()),
                        ),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("copy-prompt-tip-{id}")))
                        .tooltip(Tooltip::text(
                            "Copy the prompt and remove it from this list",
                        ))
                        .child(
                            Button::new(SharedString::from(format!("copy-prompt-{id}")), "Copy")
                                .variant(ButtonVariant::Primary)
                                .size(ControlSize::Sm)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.copy_prompt_draft(id, cx)
                                })),
                        ),
                )
                .into_any_element()
        })
        .collect();
    Some(section("Prompts for you", list(rows, palette), palette).into_any_element())
}

/// The trip that matters now: full map card when active or within 48h (or Travel mode), compact strip otherwise.
/// Returns `None` when there is no upcoming trip (unless Travel mode is active).
pub fn travel_card(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if !view.home_prefs.effective_show_travel() {
        return None;
    }
    let force_map = view.home_prefs.force_expanded_map();
    let large_map = view.home_prefs.large_map;
    let days_away = view.travel.read(cx).home_trip_days_away();
    let trip_info = view
        .travel
        .update(cx, |travel, cx| travel.home_next_trip(palette, cx));
    let imminent = force_map || days_away.is_some_and(|days| days <= 2);

    let apple_maps_button = div()
        .id("home-apple-maps-tip")
        .tooltip(Tooltip::text("Open interactive native Apple Maps window"))
        .child(
            Button::new("home-apple-maps", "Apple Maps")
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Sm)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.travel
                        .update(cx, |travel, cx| travel.open_apple_maps(cx));
                })),
        );

    let size_button = div()
        .id("home-map-size-tip")
        .tooltip(Tooltip::text("Toggle map size on Today"))
        .child(
            Button::new(
                "home-map-size",
                if large_map { "Compact" } else { "Larger" },
            )
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .on_click(cx.listener(|view, _, _, cx| {
                view.home_prefs.large_map = !view.home_prefs.large_map;
                let _ = view.home_prefs.save();
                cx.notify();
            })),
        );

    let map_button = div()
        .id("home-open-map-tip")
        .tooltip(Tooltip::text("Open full map in Travel"))
        .child(
            Button::new("home-open-map", if imminent { "Expand ↗" } else { "Map" })
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Sm)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.travel.update(cx, |travel, cx| {
                        travel.set_compact_map(false);
                        travel.open_route(sjel_gpui_core::travel::TravelRoute::Map, cx);
                    });
                    view.go(Page::Travel, cx);
                })),
        );

    let Some((trip, plan_id)) = trip_info else {
        if !force_map {
            return None;
        }
        let map = view.travel.update(cx, |travel, cx| {
            travel.set_home_map_large(large_map);
            travel.home_map(palette, cx)
        });
        let body = surface_card(palette)
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(map)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(apple_maps_button)
                    .child(size_button)
                    .child(map_button),
            );
        return Some(section("Location", body.into_any_element(), palette).into_any_element());
    };

    let open_trip_button = plan_id.map(|id| {
        Button::new("home-open-trip", "Open trip")
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Sm)
            .on_click(cx.listener(move |view, _, _, cx| {
                let route = sjel_gpui_core::travel::TravelRoute::Trip {
                    id: id.clone(),
                    section: sjel_gpui_core::travel::TripSection::Overview,
                };
                view.travel
                    .update(cx, |travel, cx| travel.open_route(route, cx));
                view.go(Page::Travel, cx);
            }))
    });

    if imminent {
        let map = view.travel.update(cx, |travel, cx| {
            travel.set_home_map_large(large_map);
            travel.home_map(palette, cx)
        });
        let body = surface_card(palette)
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_1()
                    .child(trip)
                    .children(open_trip_button),
            )
            .child(map)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(apple_maps_button)
                    .child(size_button)
                    .child(map_button),
            );

        Some(section("Active & upcoming trip", body.into_any_element(), palette).into_any_element())
    } else {
        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .child(div().w_full().min_w_0().child(trip))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(map_button)
                    .children(open_trip_button),
            );

        Some(section("Next trip", body.into_any_element(), palette).into_any_element())
    }
}

/// Compact Inbox triage summary card showing up to 3 pending captures.
pub fn inbox_triage_card(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if !view.home_prefs.effective_show_inbox() {
        return None;
    }
    let inbox = view.inbox.read(cx);
    let rows_val = inbox.rows.value.as_ref()?;
    let pending: Vec<_> = rows_val
        .iter()
        .filter(|c| matches!(c.status.as_str(), "new" | "pending"))
        .take(3)
        .collect();
    if pending.is_empty() {
        return None;
    }
    let total_pending = rows_val
        .iter()
        .filter(|c| matches!(c.status.as_str(), "new" | "pending"))
        .count();

    let mut items: Vec<AnyElement> = pending
        .into_iter()
        .enumerate()
        .map(|(ix, cap)| {
            let label = cap
                .text
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(cap.url.as_deref())
                .unwrap_or("Untitled capture")
                .to_string();
            let tag = cap.tag.clone();
            row()
                .id(("home-inbox-item", ix))
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(palette.raised)))
                .on_click(cx.listener(|view, _, _, cx| view.go(Page::Inbox, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(label),
                )
                .when_some(tag, |el, t| el.child(Badge::new(t).tone(Tone::Accent)))
                .into_any_element()
        })
        .collect();

    if total_pending > 3 {
        items.push(
            row()
                .id("home-inbox-more")
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(palette.raised)))
                .on_click(cx.listener(|view, _, _, cx| view.go(Page::Inbox, cx)))
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{total_pending} pending · Open Inbox →")),
                )
                .into_any_element(),
        );
    }

    Some(section("Inbox triage", list(items, palette), palette).into_any_element())
}

/// Active projects momentum strip showing top open projects and completion progress.
pub fn projects_momentum(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if !view.home_prefs.effective_show_projects() {
        return None;
    }
    let projects = view.projects.read(cx);
    let list_val = projects.projects.value.as_ref()?;
    let mut active: Vec<_> = list_val
        .iter()
        .filter(|p| !p.archived && p.open > 0)
        .collect();
    if active.is_empty() {
        return None;
    }
    active.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| a.name.cmp(&b.name)));
    let shown: Vec<AnyElement> = active
        .into_iter()
        .take(4)
        .enumerate()
        .map(|(ix, proj)| {
            let name = proj.name.clone();
            let total = proj.open + proj.done;
            let ratio = if total > 0 {
                proj.done as f32 / total as f32
            } else {
                0.0
            };
            row()
                .id(("home-project-row", ix))
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(palette.raised)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.go_to_project(name.clone(), cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(proj.name.clone()),
                )
                .child(progress_bar(ratio, palette.accent, palette))
                .child(
                    div()
                        .w(px(56.))
                        .flex()
                        .justify_end()
                        .child(mono_num(format!("{} open", proj.open), palette.muted)),
                )
                .into_any_element()
        })
        .collect();

    Some(section("Active projects", list(shown, palette), palette).into_any_element())
}

/// The day on screen: the date, its metric tiles, pill filters, and the timeline.
pub fn today(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    let show_timeline = view.home_prefs.show_timeline;
    let show_spending = view.home_prefs.effective_show_spending();
    let inbox_count = view.home_prefs.effective_show_inbox().then(|| {
        view.inbox
            .read(cx)
            .rows
            .value
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter(|c| matches!(c.status.as_str(), "new" | "pending"))
                    .count()
            })
            .unwrap_or(0)
    });
    view.context_page.update(cx, |page, cx| {
        page.set_sections(show_timeline, show_spending);
        page.set_inbox_count(inbox_count);
        page.set_palette(palette, cx);
    });
    view.context_page.clone().into_any_element()
}

/// Today's 24-hour quick Scratchpad card: jot a thought, link, or `- [ ]` task directly from Today,
/// see what is staged and when it auto-sorts, or pop open the docked Scratchpad seat.
pub fn scratchpad_card(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let quick_input = view
        .scratchpad
        .update(cx, |seat, cx| seat.ensure_quick_input(window, cx));
    let (targets, countdown, sorting, last_outcome) = {
        let seat = view.scratchpad.read(cx);
        let now = now_unix_secs();
        (
            seat.state.parse_targets(),
            seat.state.countdown_label(now),
            seat.sorting,
            seat.last_outcome.clone(),
        )
    };
    let count = targets.len();

    let header_row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child("Daily Scratchpad"),
                )
                .child(match countdown {
                    Some(label) => Badge::new(label).tone(Tone::Warning).into_any_element(),
                    None => Badge::new("24h auto-sort")
                        .tone(Tone::Neutral)
                        .into_any_element(),
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(count > 0, |el| {
                    el.child(
                        Button::new(
                            "home-scratchpad-sort",
                            if sorting { "Sorting…" } else { "Sort now" },
                        )
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Primary)
                        .disabled(sorting)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.scratchpad.update(cx, |seat, cx| {
                                seat.sort_now(Some(window), cx);
                            });
                        })),
                    )
                })
                .child(
                    Button::new("home-scratchpad-open", "Open panel ⌘\\")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.open_scratchpad(window, cx);
                        })),
                ),
        );

    let mut preview_items: Vec<AnyElement> = targets
        .iter()
        .take(4)
        .enumerate()
        .map(|(ix, target)| {
            let color = match target {
                ScratchTarget::Task { done: true, .. } => palette.muted,
                ScratchTarget::Task { done: false, .. } => palette.accent,
                ScratchTarget::Wishlist { .. } => palette.amber,
                ScratchTarget::Place { .. } => palette.green,
                ScratchTarget::Feed { .. } => palette.accent,
                ScratchTarget::Inbox { .. } => palette.amber,
                ScratchTarget::Note { .. } => palette.muted,
            };
            div()
                .id(("home-scratch-row", ix))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_2p5()
                .py_1()
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(tint(color, 0x16))))
                .on_click(cx.listener(|view, _, window, cx| {
                    view.open_scratchpad(window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .child(target.summary()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(color))
                        .child(target.badge_label()),
                )
                .into_any_element()
        })
        .collect();

    if count > 4 {
        preview_items.push(
            div()
                .id("home-scratch-more")
                .px_2p5()
                .py_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(|view, _, window, cx| {
                    view.open_scratchpad(window, cx);
                }))
                .child(format!("+{} more in Scratchpad →", count - 4))
                .into_any_element(),
        );
    }

    let has_preview = !preview_items.is_empty();
    let body = surface_card(palette)
        .flex()
        .flex_col()
        .gap_2p5()
        .p_3()
        .child(header_row)
        .child(Input::new(&quick_input).size(ControlSize::Sm))
        .when(has_preview, |card| {
            card.child(div().flex().flex_col().gap_1().children(preview_items))
        })
        .when(!has_preview, |card| {
            card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(match last_outcome {
                        Some(last) => format!("Last sort ({}) · {}", last.date_label, last.summary_line()),
                        None => "Jot notes, paste links, or type - [ ] tasks. Auto-sorted into Sjel after 24h.".to_owned(),
                    }),
            )
        });

    section("Quick Scratchpad", body.into_any_element(), palette).into_any_element()
}
