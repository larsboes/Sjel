use futures::StreamExt;
use gpui_kit::component::Root;
use gpui_kit::{
    actions, point, prelude::*, px, rgba, size, App, Bounds, Global, KeyBinding, Menu, MenuItem,
    TitlebarOptions, WeakEntity, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};

mod ask;
mod block_menu;
mod blocks;
mod calendar;
mod capabilities;
mod frames;
mod home;
mod inline;
#[cfg(target_os = "macos")]
mod macos_glass;
mod notes;
mod selection;
mod tasks;
mod theme;
mod ui;
mod workspace;

use sjel_gpui_core::{route, shell};

use workspace::{
    GoAsk, GoCapabilities, GoHome, GoNotes, OpenInDashboard, OpenPalette, Refresh, Workspace,
};

actions!(sjel, [Quit, CloseWindow]);

/// The one main window, so a `sjel://` link or a Dock click finds it again.
struct MainWindow {
    window: WindowHandle<Root>,
    workspace: WeakEntity<Workspace>,
}

impl Global for MainWindow {}

fn main() {
    let (url_tx, mut url_rx) = futures::channel::mpsc::unbounded::<String>();
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets::new(""));
    app.on_open_urls(move |urls| {
        for url in urls {
            let _ = url_tx.unbounded_send(url);
        }
    });
    app.on_reopen(|cx| {
        main_window(cx);
    });
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        match std::env::var("SJEL_GPUI_APPEARANCE").as_deref() {
            Ok("light") => cx.set_window_appearance(Some(WindowAppearance::Light)),
            Ok("dark") => cx.set_window_appearance(Some(WindowAppearance::Dark)),
            _ => {}
        }
        workspace::bind_keys(cx);
        notes::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        cx.set_menus([
            Menu {
                name: "Sjel".into(),
                items: vec![MenuItem::action("Quit Sjel", Quit)],
                disabled: false,
            },
            Menu {
                name: "Go".into(),
                items: vec![
                    MenuItem::action("Home", GoHome),
                    MenuItem::action("Capabilities", GoCapabilities),
                    MenuItem::action("Notes", GoNotes),
                    MenuItem::action("Ask", GoAsk),
                    MenuItem::separator(),
                    MenuItem::action("Search…", OpenPalette),
                    MenuItem::action("Refresh", Refresh),
                    MenuItem::action("Open in Dashboard", OpenInDashboard),
                ],
                disabled: false,
            },
            Menu {
                name: "Window".into(),
                items: vec![MenuItem::action("Close", CloseWindow)],
                disabled: false,
            },
        ]);

        main_window(cx);
        if let Ok(url) = std::env::var("SJEL_GPUI_OPEN") {
            open_link(&url, cx);
        }
        cx.spawn(async move |cx| {
            while let Some(url) = url_rx.next().await {
                cx.update(|cx| open_link(&url, cx));
            }
        })
        .detach();
    });
}

/// The main window, opened again when it was closed.
fn main_window(cx: &mut App) -> Option<(WindowHandle<Root>, WeakEntity<Workspace>)> {
    if let Some(main) = cx.try_global::<MainWindow>() {
        if main.workspace.upgrade().is_some() {
            let found = (main.window, main.workspace.clone());
            if activates() {
                let _ = found.0.update(cx, |_, window, _| window.activate_window());
            }
            return Some(found);
        }
    }

    let activate = activates();
    let endpoint =
        std::env::var("SJEL_STATUS_URL").unwrap_or_else(|_| shell::DEFAULT_ENDPOINT.to_owned());
    let window_size = size(px(1040.), px(720.));
    // SJEL_GPUI_FLOAT=1: a test window in the bottom-right corner above other apps. It stays
    // visible, so GPUI keeps painting, and it takes no focus.
    let float = std::env::var_os("SJEL_GPUI_FLOAT").is_some();
    let bounds = match cx.primary_display().filter(|_| float) {
        Some(display) => {
            let screen = display.bounds();
            Bounds::new(
                point(
                    screen.right() - window_size.width - px(16.),
                    screen.bottom() - window_size.height - px(16.),
                ),
                window_size,
            )
        }
        None => Bounds::centered(None, window_size, cx),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        kind: if float {
            WindowKind::PopUp
        } else {
            WindowKind::Normal
        },
        titlebar: Some(TitlebarOptions {
            title: Some("Sjel".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        window_background: if workspace::opaque_window() {
            WindowBackgroundAppearance::Opaque
        } else {
            WindowBackgroundAppearance::Blurred
        },
        window_min_size: Some(size(px(760.), px(520.))),
        focus: activate,
        ..Default::default()
    };
    if activate {
        cx.activate(true);
    }
    let mut workspace = None;
    let window = cx
        .open_window(options, |window, cx| {
            let view = cx.new(|cx| Workspace::new(&endpoint, window, cx));
            workspace = Some(view.downgrade());
            // The workspace paints its own tint over the blur.
            cx.new(|cx| Root::new(view, window, cx).bg(rgba(0x00000000)))
        })
        .ok()?;
    let workspace = workspace?;
    cx.set_global(MainWindow {
        window,
        workspace: workspace.clone(),
    });
    Some((window, workspace))
}

fn open_link(url: &str, cx: &mut App) {
    let Some(target) = route::parse_deep_link(url) else {
        return;
    };
    let Some((window, workspace)) = main_window(cx) else {
        return;
    };
    if activates() {
        cx.activate(true);
    }
    let _ = window.update(cx, |_, window, cx| {
        let _ = workspace.update(cx, |view, cx| view.open_target(target, window, cx));
    });
}

/// `SJEL_GPUI_NO_ACTIVATE=1` keeps the app in the background, also for `sjel://` links, so a
/// test run does not take focus from the person at the Mac.
fn activates() -> bool {
    std::env::var_os("SJEL_GPUI_NO_ACTIVATE").is_none()
}
