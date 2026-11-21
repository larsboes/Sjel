use gpui::{Entity, TestAppContext, VisualTestContext};
use gpui_kit::component::WindowExt;
use sjel_gpui_core::terminal::Host;
use sjel_gpui_ui::inbox::triage as inbox;

use super::*;

/// The real window shape: `Root` around the workspace.
///
/// Building a `Workspace` starts the pages' own reads against `http://example.invalid`. They
/// fail at once, but the failure arrives on the HTTP runtime's thread, which the deterministic
/// test scheduler reports as foreign activity. `allow_parking` is the scheduler's own switch for
/// a test that mixes GPUI's executor with real I/O.
fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    let (view, cx) = sjel_gpui_ui::test_support::mount_root(cx, None, |window, cx| {
        Workspace::new("http://example.invalid", window, cx)
    });
    // After Ely's init, as `main.rs` orders it: a later binding wins at the same depth.
    cx.update(|_, cx| bind_keys(cx));
    view.update(cx, |view, cx| {
        // Through `go`, because that is the only way a page is opened: it hands the frame the
        // focus the page it replaced was holding.
        view.go(Page::Capabilities, cx);
        view.health.value = Some(
            serde_json::from_str(
                r#"{"version":"t","uptime_seconds":1,"capabilities":{
                    "a":{"up":true},"b":{"up":false,"autostart":false},"c":{"up":true}}}"#,
            )
            .unwrap(),
        );
        cx.notify();
    });
    (view, cx)
}

#[gpui::test]
fn j_and_k_move_the_selection_and_escape_clears_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("j j j j k");
    view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, Some(1)));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
}

#[gpui::test]
fn enter_on_a_stopped_capability_starts_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("j j");
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.selected(),
            Some(Item::Capability {
                name: "b".into(),
                up: false
            })
        );
    });
    // The test endpoint is not on loopback, so the start fails at once and says why.
    cx.simulate_keystrokes("enter");
    let shown = cx.update(|window, cx| {
        assert!(window.notifications(cx).is_empty());
        sjel_gpui_ui::feedback::toaster(window, cx).read(cx).shown()
    });
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].1, Some(Severity::Danger));
}

/// The player is a panel over this window now, so the keys and the sidebar row both have to
/// reach it through the workspace. This is the wiring test: opening builds it once, the
/// keystroke toggles the same view, and a second open does not build a second player on the same
/// session.
/// Escape closes the player when focus is still on the page, as after the sidebar opens it.
#[gpui::test]
fn escape_closes_the_soundscape_panel_without_its_focus(cx: &mut TestAppContext) {
    std::env::set_var("SJEL_GPUI_NO_AUDIO", "1");
    let (view, cx) = open(cx);
    let panel = view.update_in(cx, |view, window, cx| {
        view.open_soundscape(window, cx);
        view.soundscape_panel.clone().expect("the panel was built")
    });
    cx.run_until_parked();
    view.update_in(cx, |view, window, cx| window.focus(&view.focus, cx));
    cx.simulate_keystrokes("escape");
    assert!(
        !cx.update(|_, cx| panel.read(cx).is_open()),
        "escape did not close the panel"
    );
}

#[gpui::test]
fn the_soundscape_panel_opens_once_and_toggles_from_the_keyboard(cx: &mut TestAppContext) {
    // Opening the panel starts the audio global, and a test must not take the machine's output
    // device. This is the same switch a headless run uses.
    std::env::set_var("SJEL_GPUI_NO_AUDIO", "1");
    let (view, cx) = open(cx);
    assert!(
        view.read_with(cx, |view, _| view.soundscape_panel.is_none()),
        "a fresh window pays for no panel"
    );
    cx.simulate_keystrokes("shift-cmd-m");
    let panel = view.read_with(cx, |view, _| {
        view.soundscape_panel.clone().expect("the panel was built")
    });
    let opened = cx.update(|_, cx| panel.read(cx).is_open());
    assert!(opened, "the keystroke did not open the panel");

    // The same keystroke closes the same view, rather than building a second one.
    cx.simulate_keystrokes("shift-cmd-m");
    let again = view.read_with(cx, |view, _| {
        view.soundscape_panel.clone().expect("the panel stays")
    });
    assert_eq!(
        panel.entity_id(),
        again.entity_id(),
        "a second panel was built"
    );
    let closed = cx.update(|_, cx| again.read(cx).is_open());
    assert!(!closed, "the second press did not close it");

    // ⌘0 is the same toggle, one key from the nine areas.
    cx.simulate_keystrokes("cmd-0");
    assert!(
        cx.update(|_, cx| again.read(cx).is_open()),
        "⌘0 did not open it"
    );

    view.update_in(cx, |view, window, cx| {
        view.selection.cursor = Some(0);
        window.focus(&view.focus, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|_, cx| again.read(cx).is_open()));
    view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, Some(0)));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
}

#[gpui::test]
fn the_soundscape_mini_player_stays_compact_with_a_repeatable_session(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_resize(gpui_kit::size(px(1200.), px(800.)));
    view.update(cx, |view, cx| {
        view.appearance_settings.soundscape_mini_player = true;
        view.soundscape.value = Some(
            serde_json::from_value(serde_json::json!({
                "preset": "focus",
                "last_session": {
                    "scenario": "deep-focus",
                    "duration_ms": 3_600_000,
                    "preset": "focus",
                    "params": {},
                    "energy": 0.5
                }
            }))
            .unwrap(),
        );
        cx.notify();
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("soundscape-mini-player")
        .expect("visible player");
    assert!(bounds.size.height < px(110.), "{bounds:?}");
}

/// `sjel://terminal` opens the edge on its own seat. The test endpoint is not loopback, so no
/// shell can be opened here — which is the second half of the assertion: a Mac that cannot
/// answer leaves the seat saying so rather than a grid that never fills.
#[gpui::test]
fn a_terminal_link_opens_the_edge_on_the_terminal_seat(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update_in(cx, |view, window, cx| {
        view.open_target(Target::Terminal, window, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.right_panel.is_open(), "the edge stayed closed");
        assert_eq!(view.right_panel.seat(), Seat::Terminal);
        assert!(
            view.terminal.is_none(),
            "a terminal was built without a capability to hold it"
        );
    });
}

/// The page's single-key navigation must not take a key the program in the terminal needs.
#[gpui::test]
fn keys_typed_in_the_terminal_do_not_drive_the_page(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let terminal = view.update_in(cx, |view, window, cx| {
        let shell = std::sync::Arc::new(
            sjel_gpui_core::shell::Shell::new("http://127.0.0.1:9").expect("a loopback endpoint"),
        );
        let peer = sjel_gpui_core::terminal::Terminal {
            id: "t1".into(),
            alive: true,
            ..Default::default()
        };
        let _ = window;
        let terminal = cx.new(|cx| TerminalDeck::new(Host::Shell(shell), false, 48, 44, cx));
        terminal.update(cx, |deck, cx| deck.adopt(peer, cx));
        view.terminal = Some(terminal.clone());
        view.right_panel.open_on(Seat::Terminal);
        cx.notify();
        terminal
    });
    cx.run_until_parked();
    view.update_in(cx, |_, window, cx| {
        window.focus(&gpui::Focusable::focus_handle(&terminal, cx), cx);
    });
    cx.simulate_keystrokes("j j enter escape");
    view.read_with(cx, |view, _| {
        assert_eq!(view.selection.cursor, None, "j moved the page's selection");
        assert_eq!(view.right_panel.seat(), Seat::Terminal);
    });
}

#[gpui::test]
fn command_t_opens_the_terminal_edge(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("cmd-t");
    view.read_with(cx, |view, _| {
        assert!(view.right_panel.is_open(), "⌘T left the edge closed");
        assert_eq!(view.right_panel.seat(), Seat::Terminal);
    });
}

#[gpui::test]
fn typing_in_the_palette_does_not_move_the_selection(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("cmd-k j k j");
    view.read_with(cx, |view, _| assert_eq!(view.selection.cursor, None));
}

/// The edit field sits inside the capture list, so the letters the list binds must type into
/// it instead of moving the cursor or marking the capture done.
#[gpui::test]
fn typing_in_a_capture_edit_does_not_triage(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.update(|_, cx| inbox::bind_keys(cx));
    let inbox = view.update(cx, |view, cx| {
        view.go(Page::Inbox, cx);
        view.inbox.clone()
    });
    cx.run_until_parked();
    inbox.update(cx, |inbox, cx| {
        let capture = |id: &str| sjel_gpui_core::capture::Capture {
            id: id.into(),
            text: Some("shirt".into()),
            url: None,
            tag: None,
            status: "new".into(),
            routed_to: None,
            client: None,
            created_at: "2026-10-24".into(),
        };
        inbox.rows.value = Some(vec![capture("a"), capture("b")]);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("j");
    inbox.read_with(cx, |inbox, _| {
        assert_eq!(inbox.cursor.as_deref(), Some("a"))
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" jed");
    inbox.read_with(cx, |inbox, cx| {
        assert_eq!(inbox.cursor.as_deref(), Some("a"));
        let (id, field, _) = inbox.editing.as_ref().expect("the text is open");
        assert_eq!(id, "a");
        assert_eq!(field.read(cx).text(), "shirt jed");
    });
}

/// A question in Auto goes to the one Ask session, and the finder stays open to show it; ⌘↩
/// sends a search-looking text as a question too.
#[gpui::test]
fn the_finder_asks_when_the_words_are_a_question(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("cmd-k");
    let (finder, ask) = view.read_with(cx, |view, _| (view.finder.clone(), view.ask.clone()));
    cx.simulate_input("what is on my calendar today?");
    cx.simulate_keystrokes("enter");
    let prompt = cx.update(|_, cx| ask.read(cx).latest().map(|exchange| exchange.prompt));
    assert_eq!(prompt.as_deref(), Some("what is on my calendar today?"));
    assert!(
        cx.update(|_, cx| finder.read(cx).is_open()),
        "the answer shows in place"
    );

    cx.simulate_input("packs");
    cx.simulate_keystrokes("cmd-enter");
    let prompt = cx.update(|_, cx| ask.read(cx).latest().map(|exchange| exchange.prompt));
    assert_eq!(
        prompt.as_deref(),
        Some("packs"),
        "⌘↩ asked instead of opening Packs"
    );
}

/// ⌘K opens the finder; typing narrows it; the arrows move through the results and Enter
/// goes there; escape closes it and the page keys work again.
#[gpui::test]
fn the_finder_searches_opens_and_closes(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("cmd-k");
    let finder = view.read_with(cx, |view, _| view.finder.clone());
    assert!(cx.update(|_, cx| finder.read(cx).is_open()));
    cx.simulate_keystrokes("escape");
    assert!(
        !cx.update(|_, cx| finder.read(cx).is_open()),
        "escape closes it"
    );
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("packs");
    let first = cx.update(|_, cx| finder.read(cx).hits().first().map(|hit| hit.title.clone()));
    assert_eq!(first.as_deref(), Some(page_title(Page::Packs).as_str()));
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
    assert!(
        !cx.update(|_, cx| finder.read(cx).is_open()),
        "Enter closes it"
    );

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("set");
    // The first arrow picks the top row, the second moves to the next one.
    let second = cx.update(|_, cx| match finder.read(cx).hits()[1].open {
        Open::Page(page) => page,
        _ => panic!("\"set\" ranks pages first"),
    });
    cx.simulate_keystrokes("down down enter");
    view.read_with(cx, |view, _| {
        assert_eq!(view.page, second, "the arrows chose it")
    });

    // The input takes the focus on the frame that draws it, so the keys come one frame apart,
    // as a hand would send them.
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_keystrokes("escape");
    assert!(
        !cx.update(|_, cx| finder.read(cx).is_open()),
        "escape closes it"
    );
}

#[gpui::test]
fn page_shortcuts_switch_and_reset_the_selection(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("j cmd-1");
    view.read_with(cx, |view, _| {
        assert_eq!(view.page, Page::Home);
        assert_eq!(view.selection.cursor, None);
    });
}

#[gpui::test]
fn the_number_keys_open_the_nine_areas(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    for (area, n) in Area::ALL.into_iter().zip(1..=9) {
        cx.simulate_keystrokes(&format!("cmd-{n}"));
        view.read_with(cx, |view, _| assert_eq!(Area::of(view.page), Some(area)));
    }
}

#[gpui::test]
fn an_area_returns_to_the_tab_it_showed_last(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("cmd-8");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Capabilities));
    view.update(cx, |view, cx| view.go(Page::Packs, cx));
    cx.simulate_keystrokes("cmd-1");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
    cx.simulate_keystrokes("cmd-8");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
}

#[test]
fn calendar_travel_action_opens_the_existing_plan_overview() {
    assert_eq!(
        calendar_trip_route("trip:existing".into()),
        sjel_gpui_core::travel::TravelRoute::Trip {
            id: "trip:existing".into(),
            section: sjel_gpui_core::travel::TripSection::Overview,
        }
    );
}

#[test]
fn failed_task_write_restores_previous_state_without_a_recovery_read() {
    let mut tasks = vec![GlobalTask {
        fields: Default::default(),
        id: "Inbox/Tasks/Task.md".into(),
        kind: "task".into(),
        title: "Task".into(),
        priority: 1,
        due: None,
        done: true,
        summary: None,
        project: "Inbox".into(),
    }];
    tasks[0].done = false;
    restore_task_done(&mut tasks, "Inbox/Tasks/Task.md", true);
    assert!(tasks[0].done);
}

#[gpui::test]
fn failed_task_write_restores_state_when_the_shell_is_unreachable(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let path = "Inbox/Tasks/Offline.md".to_owned();
    view.update_in(cx, |view, window, cx| {
        view.repo = Repo::new(shell::Shell::new("http://127.0.0.1:9").map(std::sync::Arc::new));
        view.focus_tasks.value = Some(vec![GlobalTask {
            fields: Default::default(),
            id: path.clone(),
            kind: "task".into(),
            title: "Offline task".into(),
            priority: 1,
            due: None,
            done: false,
            summary: None,
            project: "Inbox".into(),
        }]);
        view.toggle_task(path.clone(), true, window, cx);
        assert!(view.focus_tasks.value.as_ref().unwrap()[0].done);
    });
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(500) {
        cx.run_until_parked();
        let settled = view.read_with(cx, |view, _| {
            !view.focus_tasks.value.as_ref().unwrap()[0].done
        });
        if settled {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    view.read_with(cx, |view, _| {
        assert!(!view.focus_tasks.value.as_ref().unwrap()[0].done);
    });
}

#[gpui::test]
fn home_focus_tasks_keyboard_navigation(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |view, cx| {
        view.page = Page::Home;
        view.health.value = None;
        view.focus_tasks.value = Some(vec![
            GlobalTask {
                fields: Default::default(),
                id: "Atlas/Projects/Test/Tasks/Task1.md".into(),
                kind: "task".into(),
                title: "Focus on LibSQL".into(),
                priority: 1,
                due: None,
                done: false,
                summary: None,
                project: "Test".into(),
            },
            GlobalTask {
                fields: Default::default(),
                id: "Atlas/Projects/Test/Tasks/Task2.md".into(),
                kind: "task".into(),
                title: "Fix bug".into(),
                priority: 2,
                due: Some("2020-01-01".into()),
                done: false,
                summary: None,
                project: "Test".into(),
            },
        ]);
        cx.notify();
    });
    cx.simulate_keystrokes("j");
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.selected(),
            Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task1.md".into()))
        );
    });
    cx.simulate_keystrokes("j");
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.selected(),
            Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task2.md".into()))
        );
    });
    cx.simulate_keystrokes("k");
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.selected(),
            Some(Item::FocusTask("Atlas/Projects/Test/Tasks/Task1.md".into()))
        );
    });
    // Space runs primary on the selected focus task
    cx.simulate_keystrokes("space");
}

#[gpui::test]
fn home_toolbar_renders_date_and_status(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |view, cx| {
        view.page = Page::Home;
        view.health.value = None;
        view.focus_tasks.value = Some(vec![GlobalTask {
            fields: Default::default(),
            id: "Atlas/Projects/Test/Tasks/Task1.md".into(),
            kind: "task".into(),
            title: "Focus on LibSQL".into(),
            priority: 1,
            due: None,
            done: false,
            summary: None,
            project: "Test".into(),
        }]);
        cx.notify();
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.page, Page::Home);
        assert_eq!(view.urgent_task_count(), 1);
    });
}

#[gpui::test]
fn every_sidebar_page_has_a_shortcut(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("shift-cmd-d");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Documents));
    cx.simulate_keystrokes("shift-cmd-b");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Backup));
    cx.simulate_keystrokes("shift-cmd-f");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Finance));
    cx.simulate_keystrokes("shift-cmd-p");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::People));
    cx.simulate_keystrokes("shift-cmd-i");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inventory));
    cx.simulate_keystrokes("shift-cmd-s");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Software));
    // Import has no ⌘⇧ letter: shift-cmd-u toggles the soundscape. It has `g u`, which
    // needs a page without its own text input.
    cx.simulate_keystrokes("shift-cmd-h");
    cx.simulate_keystrokes("g u");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Import));
    // Systems is the same shape: no ⌘ letter of its own, so `g x`.
    cx.simulate_keystrokes("shift-cmd-h");
    cx.simulate_keystrokes("g x");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Systems));
}

#[gpui::test]
fn tab_keys_step_inside_the_open_area(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |view, cx| view.go(Page::Inbox, cx));
    cx.simulate_keystrokes("cmd-alt-right");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Feed));
    cx.simulate_keystrokes("shift-cmd-]");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
    cx.simulate_keystrokes("cmd-alt-left");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Feed));
    cx.simulate_keystrokes("ctrl-tab");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
    // An area with one tab keeps it: stepping never leaves the area.
    view.update(cx, |view, cx| view.go(Page::Calendar, cx));
    cx.simulate_keystrokes("shift-cmd-]");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Calendar));
    // Life opens on People, and stepping back from it wraps to its last tab.
    cx.simulate_keystrokes("cmd-7");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::People));
    cx.simulate_keystrokes("cmd-alt-left");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Posts));
}

#[gpui::test]
fn the_area_keys_step_down_the_sidebar(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |view, cx| view.go(Page::Home, cx));
    cx.simulate_keystrokes("cmd-alt-down");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Inbox));
    cx.simulate_keystrokes("cmd-alt-up");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
    // Up from the first area wraps to the last one.
    cx.simulate_keystrokes("cmd-alt-up");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Capabilities));
}

#[gpui::test]
fn vim_g_chords_switch_pages(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    cx.simulate_keystrokes("g h");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Home));
    cx.simulate_keystrokes("g d");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Documents));
    cx.simulate_keystrokes("g f");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Finance));
    cx.simulate_keystrokes("g b");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Backup));
    cx.simulate_keystrokes("g x");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Systems));
    cx.simulate_keystrokes("g k");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Packs));
    cx.simulate_keystrokes("g l");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::SelfModel));
    cx.simulate_keystrokes("g y");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Cv));
    cx.simulate_keystrokes("g w");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Media));
    cx.simulate_keystrokes("g z");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Research));
    cx.simulate_keystrokes("g 0");
    view.read_with(cx, |view, _| assert_eq!(view.page, Page::Setup));
}

#[gpui::test]
fn focus_search_opens_on_pages(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    // On Documents
    view.update(cx, |view, cx| view.go(Page::Documents, cx));
    cx.simulate_keystrokes("cmd-f");
    // On Finance
    view.update(cx, |view, cx| view.go(Page::Finance, cx));
    cx.simulate_keystrokes("/");
    // On People
    view.update(cx, |view, cx| view.go(Page::People, cx));
    cx.simulate_keystrokes("/");
    // On Inventory
    view.update(cx, |view, cx| view.go(Page::Inventory, cx));
    cx.simulate_keystrokes("/");
}

#[gpui::test]
fn scratchpad_seat_toggles_and_deep_link_opens_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update_in(cx, |view, window, cx| {
        view.open_target(Target::Scratchpad, window, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.right_panel.is_open(), "scratchpad link opens the edge");
        assert_eq!(view.right_panel.seat(), Seat::Scratchpad);
    });
    view.update_in(cx, |view, window, cx| {
        view.toggle_scratchpad(window, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(
            !view.right_panel.is_open(),
            "second toggle closes the scratchpad edge"
        );
    });
}
