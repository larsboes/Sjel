// GPUI macros expand to `gpui::` paths. The Kit re-exports the GPUI it is built on.
extern crate gpui_kit as gpui;

use futures::StreamExt;
use gpui_kit::component::Root;
use gpui_kit::{
    actions, point, prelude::*, px, rgba, size, App, Bounds, Global, KeyBinding, Menu, MenuItem,
    TitlebarOptions, WeakEntity, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};

mod audio;
pub mod clipboard;
mod data;
mod finder;
mod frames;
mod header;
mod home;
#[cfg(target_os = "macos")]
mod macos_glass;
#[cfg(feature = "nsh-trial")]
mod nsh_trial;
mod permissions;
#[cfg(target_os = "macos")]
mod roomplan;
mod script;
mod selection;
mod settings;
mod sidebar;
mod soundscape;
pub mod update;
mod workspace;

use sjel_gpui_core::area::Area;
use sjel_gpui_core::{route, shell};

use workspace::{
    OpenArea, OpenPalette, OpenSettings, OpenSoundscape, Refresh, ToggleSoundscapePanel, UiZoomIn,
    UiZoomOut, UiZoomReset, Workspace,
};

actions!(sjel, [Quit, CloseWindow]);

/// The one main window, so a `sjel://` link or a Dock click finds it again.
struct MainWindow {
    window: WindowHandle<Root>,
    workspace: WeakEntity<Workspace>,
}

impl Global for MainWindow {}

fn main() {
    sjel_gpui_ui::documents::set_default_pdf_renderer(sjel_gpui_ui::pdf_kit::default_renderer);
    sjel_gpui_ui::clipper::set_rich_clipboard_reader(clipboard::read_rich_clipboard);
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
        // The theme mode saved in Settings, unless SJEL_GPUI_APPEARANCE overrides it for a test.
        let forced = match std::env::var("SJEL_GPUI_APPEARANCE").as_deref() {
            Ok("light") => Some(WindowAppearance::Light),
            Ok("dark") => Some(WindowAppearance::Dark),
            _ => sjel_gpui_ui::theme::AppearanceSettings::load()
                .theme_mode
                .forced_appearance(),
        };
        if forced.is_some() {
            cx.set_window_appearance(forced);
        }
        workspace::bind_keys(cx);
        sjel_gpui_ui::editor::bind_keys(cx);
        sjel_gpui_ui::feed::bind_keys(cx);
        sjel_gpui_ui::inbox::triage::bind_keys(cx);
        sjel_gpui_ui::mail::bind_keys(cx);
        sjel_gpui_ui::calendar::page::bind_keys(cx);
        sjel_gpui_ui::context::bind_keys(cx);
        sjel_gpui_ui::finance::bind_keys(cx);
        sjel_gpui_ui::people::page::bind_keys(cx);
        sjel_gpui_ui::backup::bind_keys(cx);
        sjel_gpui_ui::inventory::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        // The keystroke and the menu both land on the focused workspace, which owns the panel. With
        // no window up there is nothing to slide it over, so the app-level fallback raises the main
        // window first.
        cx.on_action(|_: &ToggleSoundscapePanel, cx| {
            if let Some((window, workspace)) = main_window(cx) {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = workspace.update(cx, |view, cx| {
                        view.open_soundscape(window, cx);
                    });
                });
            }
        });
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        cx.set_menus([
            Menu {
                name: "Sjel".into(),
                items: vec![
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("Quit Sjel", Quit),
                ],
                disabled: false,
            },
            Menu {
                name: "Go".into(),
                items: Area::ALL
                    .into_iter()
                    .map(|area| MenuItem::action(area.label(), OpenArea(area)))
                    .chain([
                        MenuItem::separator(),
                        MenuItem::action("Search…", OpenPalette),
                        MenuItem::action("Refresh", Refresh),
                    ])
                    .collect(),
                disabled: false,
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Actual Size", UiZoomReset),
                    MenuItem::action("Zoom In", UiZoomIn),
                    MenuItem::action("Zoom Out", UiZoomOut),
                ],
                disabled: false,
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Soundscape", OpenSoundscape),
                    MenuItem::separator(),
                    MenuItem::action("Close", CloseWindow),
                ],
                disabled: false,
            },
        ]);

        main_window(cx);
        if let Ok(url) = std::env::var("SJEL_GPUI_OPEN") {
            open_link(&url, cx);
        }
        script::run_from_env(cx);
        #[cfg(feature = "nsh-trial")]
        if std::env::var("SJEL_NSH_TRIAL").as_deref() == Ok("1") {
            nsh_trial::open(cx);
        }
        // `SJEL_GPUI_SOUNDSCAPE=1`: open the player at launch, so a test or a screenshot finds it
        // without a click. `SJEL_GPUI_SOUNDSCAPE=play` also starts it, and starts it *through the
        // conductor* -- the instructor of state is not this window, so a local `set_playing` here
        // would be overwritten by the first read.
        match std::env::var("SJEL_GPUI_SOUNDSCAPE").as_deref() {
            Ok("1") | Ok("play") => {
                if let Some((window, workspace)) = main_window(cx) {
                    let _ = window.update(cx, |_, window, cx| {
                        let _ = workspace.update(cx, |view, cx| {
                            view.open_soundscape(window, cx);
                        });
                    });
                }
                if std::env::var("SJEL_GPUI_SOUNDSCAPE").as_deref() == Ok("play") {
                    soundscape::play_through_conductor(cx);
                }
            }
            _ => {}
        }
        if let Some(seconds) = audio_probe_seconds() {
            probe_audio(seconds, cx);
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
    let endpoint = sjel_gpui_core::environment::current().endpoint.clone();
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
        window_bounds: Some(if float {
            WindowBounds::Windowed(bounds)
        } else {
            WindowBounds::Maximized(bounds)
        }),
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

/// `SJEL_GPUI_AUDIO_PROBE=<seconds>`: start the soundscape on the real device, print what the
/// synth measures, and quit. This is the check for "there is no sound": it exercises the device,
/// the engine and the spectrum in one run, without a window and without anyone having to listen.
fn audio_probe_seconds() -> Option<u64> {
    std::env::var("SJEL_GPUI_AUDIO_PROBE")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|seconds| *seconds > 0)
}

fn probe_audio(seconds: u64, cx: &mut App) {
    let audio = audio::audio(cx);
    match (audio.device_name(), audio.sample_rate(), audio.error()) {
        (Some(name), Some(rate), _) => println!("device: {name} at {rate:.0} Hz"),
        (_, _, Some(error)) => println!("no device: {error}"),
        _ => println!("no device"),
    }
    audio
        .control
        .set_preset(sjel_gpui_core::soundscape::Preset::Focus);
    audio.control.set_volume(0.8);
    audio.control.set_playing(true);

    let visual = audio.visual.clone();
    cx.spawn(async move |cx| {
        let mut loudest = 0f32;
        let mut all: Vec<f32> = Vec::new();
        for tick in 0..(seconds * 4) {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let (rms, kicks, notes) = (visual.rms(), visual.kicks(), visual.notes());
            let bands = visual.bands();
            let top = bands.iter().fold(0f32, |best, band| best.max(*band));
            loudest = loudest.max(rms);
            all.push(rms);
            println!(
                "{:.2}s rms {rms:.4} peak {:.4} bands {top:.2} voices {} kicks {kicks} notes {notes} preset {} profile {:?}",
                tick as f32 / 4.,
                visual.peak(),
                visual.voices(),
                visual.preset().key(),
                bands.map(|band| (band * 100.) as u32),
            );
        }
        let sounded = all.iter().filter(|rms| **rms > 0.001).count();
        println!(
            "measured {sounded} of {} ticks above silence, loudest rms {loudest:.4}",
            all.len()
        );
        if loudest <= 0.001 {
            println!("VERDICT: silent -- the device opened but nothing reached it");
        } else {
            println!("VERDICT: sounding");
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
