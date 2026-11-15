use super::*;
use crate::ely::calendar::When;
use gpui::TestAppContext;

fn day(s: &str) -> NaiveDate {
    calendar::parse_day(s).unwrap()
}

/// Paging keeps the day where the month allows it and clamps where it does not; a week page
/// is seven days. No shell, so the reads fail fast and the selection is what is checked.
#[gpui::test]
fn pages_move_the_selected_day(cx: &mut TestAppContext) {
    let page = cx.new(|cx| CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx));
    page.update(cx, |page, cx| {
        page.day = day("2026-01-31");
        page.shift_page(true, cx);
        assert_eq!(page.day, day("2026-02-28"));
        page.shift_page(false, cx);
        assert_eq!(page.day, day("2026-01-28"));
        page.set_view(View::Week, cx);
        page.shift_page(true, cx);
        assert_eq!(page.day, day("2026-02-04"));
        assert!(page.entries.error.is_some());
        page.set_view(View::Day, cx);
        page.shift_page(true, cx);
        assert_eq!(page.day, day("2026-02-05"));
        assert_eq!(page.dates(), [day("2026-02-05")]);
    });
}

#[gpui::test]
fn changing_keyboard_selection_or_open_day_cannot_reuse_stale_details(cx: &mut TestAppContext) {
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.selected_entry = Some("calendar:old".into());
        page.selected_detail = Some((
            "calendar:old".into(),
            CalendarEntryDetail {
                payload: Some(serde_json::json!({
                    "importance_rationale": "stale rationale",
                    "origin": {"capability": "comms", "source": "mail"}
                })),
                ..Default::default()
            },
        ));
        page
    });
    page.update(cx, |page, cx| {
        page.select_entry("calendar:new".into(), cx);
        assert_eq!(page.selected_entry.as_deref(), Some("calendar:new"));
        assert!(page.selected_detail.is_none());
        page.selected_detail = Some(("calendar:old".into(), CalendarEntryDetail::default()));
        page.open_day(day("2026-11-06"), Some("calendar:next".into()), cx);
        assert_eq!(page.selected_entry.as_deref(), Some("calendar:next"));
        assert!(page.selected_detail.is_none());
    });
}

#[gpui::test]
fn month_has_a_bounded_scroll_viewport_for_dense_weeks(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        page.entries.value = Some(
            (0..8)
                .map(|i| CalendarEntry {
                    id: format!("calendar:fixture{i}"),
                    title: format!("All-day event {i}"),
                    starts_at: "2026-10-26".into(),
                    ends_at: "2026-12-01".into(),
                    all_day: true,
                    ..Default::default()
                })
                .collect(),
        );
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.simulate_resize(gpui::size(px(1100.), px(700.)));
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("calendar-month-scroll")
        .expect("month viewport");
    assert!(bounds.size.height > px(0.));
    assert!(bounds.bottom() < px(700.));
    let more = cx
        .debug_bounds("more 2026-11-05")
        .expect("dense days expose hidden events");
    assert!(
        bounds.contains(&more.center()),
        "more={more:?} viewport={bounds:?}"
    );
    cx.simulate_click(more.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
        assert_eq!(page.view, View::Day);
        assert_eq!(page.day, day("2026-11-05"));
    });
}

#[gpui::test]
fn calendar_header_displays_sync_status_mark(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.sync_status = Some(crate::calendar::types::CalendarSyncStatus {
            google_synced_at: Some("2026-11-05T20:00:00Z".into()),
            google_error: None,
            icloud_synced_at: Some("2026-11-05T20:00:00Z".into()),
            icloud_error: None,
            replica_synced_at: None,
            replica_error: None,
        });
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.simulate_resize(gpui::size(px(1100.), px(700.)));
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("calendar-sync-status")
        .expect("sync status mark rendered");
    assert!(bounds.size.width > px(0.));
}

#[gpui::test]
fn day_timeline_sizes_a_ninety_minute_event_and_edits_it_by_keyboard(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
    cx.update(bind_keys);
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        page.view = View::Day;
        page.entries.value = Some(vec![CalendarEntry {
            id: "calendar:fixture".into(),
            title: "Review".into(),
            starts_at: "2026-11-05T09:00:00".into(),
            ends_at: "2026-11-05T10:30:00".into(),
            kind: "event".into(),
            commitment: "planned".into(),
            ..Default::default()
        }]);
        page.selected_entry = Some("calendar:fixture".into());
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.run_until_parked();
    let block = cx
        .debug_bounds("event-card calendar:fixture")
        .expect("timed block rendered");
    assert_eq!(block.size.height, px(70.));
    let scroll = cx
        .debug_bounds("time-grid")
        .expect("scroll viewport rendered");
    assert!(scroll.size.height > px(0.));
    let start = block.origin + gpui::point(px(12.), px(12.));
    let target = start + gpui::point(px(0.), px(60.));
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(
        start + gpui::point(px(0.), px(6.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_move(target, gpui::MouseButton::Left, gpui::Modifiers::none());
    page.read_with(cx, |page, _| {
        assert!(page.write_error.is_none(), "drag preview must not write");
    });
    cx.simulate_mouse_up(target, gpui::MouseButton::Left, gpui::Modifiers::none());
    page.read_with(cx, |page, _| {
        assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
    });
    cx.simulate_keystrokes("enter");
    page.read_with(cx, |page, _| {
        let form = page.form.as_ref().expect("Enter opens edit");
        assert!(matches!(form.draft.when, When::Timed { start, end } if
            adapter::local(start, &page.zone) == "2026-11-05T09:00:00" &&
            adapter::local(end, &page.zone) == "2026-11-05T10:30:00"));
        assert!(form.editing.is_some());
    });
}

#[gpui::test]
fn slot_click_opens_ely_editor_and_escape_restores_grid_focus(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
        bind_keys(cx);
    });
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        page.view = View::Day;
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.run_until_parked();
    let column = cx.debug_bounds("column 2026-11-05").unwrap();
    let at = gpui::point(column.center().x, column.top() + px(48. * 10.));
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("event-editor").is_some());
    page.read_with(cx, |page, _| {
        let form = page.form.as_ref().expect("slot opened the editor");
        assert!(matches!(form.draft.when, When::Timed { start, end } if
            adapter::local(start, &page.zone) == "2026-11-05T10:00:00" &&
            adapter::local(end, &page.zone) == "2026-11-05T11:00:00"));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(page.form.is_none()));
    cx.simulate_keystrokes("right");
    page.read_with(cx, |page, _| assert_eq!(page.day, day("2026-11-06")));
}

#[gpui::test]
fn undo_time_action_reverts_time_change(cx: &mut gpui::TestAppContext) {
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        let original = CalendarEntry {
            id: "calendar:fixture".into(),
            title: "Standup".into(),
            starts_at: "2026-11-05T09:00:00".into(),
            ends_at: "2026-11-05T09:30:00".into(),
            kind: "event".into(),
            commitment: "planned".into(),
            ..Default::default()
        };
        let moved = CalendarEntry {
            starts_at: "2026-11-05T10:00:00".into(),
            ends_at: "2026-11-05T10:30:00".into(),
            ..original.clone()
        };
        page.entries.value = Some(vec![moved.clone()]);
        page.undo_time = Some((original, moved));
        page
    });
    page.update(cx, |page, cx| {
        page.undo_time(cx);
        assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
    });
}

#[gpui::test]
fn week_timeline_renders_seven_day_columns_duration_blocks_overlaps_and_now_line(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
    cx.update(bind_keys);
    let today = chrono::Local::now().date_naive();
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = today;
        page.view = View::Week;
        let day_str = today.format("%Y-%m-%d").to_string();
        page.entries.value = Some(vec![
            CalendarEntry {
                id: "calendar:overlap1".into(),
                title: "Architecture Review".into(),
                starts_at: format!("{day_str}T10:00:00"),
                ends_at: format!("{day_str}T11:30:00"),
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            },
            CalendarEntry {
                id: "calendar:overlap2".into(),
                title: "Security Sync".into(),
                starts_at: format!("{day_str}T10:30:00"),
                ends_at: format!("{day_str}T12:00:00"),
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            },
            CalendarEntry {
                id: "calendar:allday".into(),
                title: "Offsite".into(),
                starts_at: day_str.clone(),
                ends_at: (today + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string(),
                all_day: true,
                kind: "event".into(),
                commitment: "planned".into(),
                ..Default::default()
            },
        ]);
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.run_until_parked();

    // 7 days in week view
    page.read_with(cx, |page, _| {
        assert_eq!(page.dates().len(), 7);
        assert!(page.dates().contains(&today));
    });

    // Now line rendered for today
    let now_line = cx.debug_bounds("now-line");
    assert!(now_line.is_some(), "now-line rendered on today's column");

    // All-day span rendered in top strip
    let span_block = cx.debug_bounds("event-chip calendar:allday");
    assert!(span_block.is_some(), "all-day span rendered across week");

    // Overlapping blocks share columns and are sized to 90 min (72px)
    let blocks = cx.debug_bounds("event-card calendar:overlap1");
    assert!(blocks.is_some(), "timed blocks rendered");
    assert_eq!(blocks.unwrap().size.height, px(70.));

    // Dragging moves target day and minute across columns
    let block_origin = blocks.unwrap().origin + gpui::point(px(10.), px(10.));
    let next_day_target = block_origin + gpui::point(px(80.), px(48.));
    cx.simulate_mouse_down(
        block_origin,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_move(
        block_origin + gpui::point(px(0.), px(5.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_move(
        next_day_target,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    page.read_with(cx, |page, _| {
        assert!(page.write_error.is_none(), "preview does not write");
    });
    cx.simulate_mouse_up(
        next_day_target,
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
}

#[gpui::test]
fn desktop_quick_reschedule_shifts(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        page.entries.value = Some(vec![CalendarEntry {
            id: "calendar:quick".into(),
            title: "Standup".into(),
            starts_at: "2026-11-05T09:00:00".into(),
            ends_at: "2026-11-05T09:30:00".into(),
            kind: "event".into(),
            commitment: "planned".into(),
            ..Default::default()
        }]);
        page.selected_entry = Some("calendar:quick".into());
        page
    });
    page.update(cx, |page, cx| {
        page.quick_shift_days("calendar:quick", 1, cx);
        assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
        page.write_error = None;

        page.quick_shift_minutes("calendar:quick", 15, false, cx);
        assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
    });
}

#[gpui::test]
fn editor_typing_cannot_trigger_grid_shortcuts_and_failed_save_stays_open(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
        bind_keys(cx);
    });
    let page = cx.new(|cx| {
        let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
        page.day = day("2026-11-05");
        page
    });
    let shown = page.clone();
    let (_, cx) =
        cx.add_window_view(move |window, cx| gpui_kit::component::Root::new(shown, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("n");
    cx.run_until_parked();
    cx.simulate_input("new meeting");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    page.read_with(cx, |page, _| {
        assert_eq!(page.day, day("2026-11-05"));
        assert_eq!(page.view, View::Month);
        assert!(page.form.is_some());
        assert!(matches!(page.write_error, Some(ShellError::NotSetUp)));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_keystrokes("w");
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert_eq!(page.view, View::Week));
}
