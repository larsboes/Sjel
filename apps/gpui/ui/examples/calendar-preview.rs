//! Render a synthetic calendar to PNG without showing a window or reading personal data.

use gpui::{prelude::*, px, size, App, Bounds, WindowBounds, WindowOptions};
use gpui_kit::component::Root;
use sjel_gpui_core::shell::{CalendarEntry, ShellError};
use sjel_gpui_ui::{
    calendar::{CalendarPage, View},
    data::Repo,
    Palette,
};

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    let Some(output) = arguments.get(1).cloned() else {
        eprintln!("Usage: calendar-preview OUTPUT.png [day|week|month] [light|dark]");
        std::process::exit(2);
    };
    let view = match arguments.get(2).map(String::as_str).unwrap_or("week") {
        "day" => View::Day,
        "week" => View::Week,
        "month" => View::Month,
        _ => {
            eprintln!("Choose day, week or month.");
            std::process::exit(2);
        }
    };
    let palette = if arguments.get(3).map(String::as_str) == Some("dark") {
        Palette::dark()
    } else {
        Palette::light()
    };
    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1280.), px(900.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            },
            |window, cx| {
                let page = cx.new(|cx| {
                    let mut page = CalendarPage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
                    page.day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
                    page.view = view;
                    page.set_palette(palette, cx);
                    page.entries.settle(Ok(vec![
                        entry(
                            "Review",
                            "2026-11-05T09:00:00",
                            "2026-11-05T11:00:00",
                            false,
                        ),
                        entry(
                            "Dentist",
                            "2026-11-05T09:30:00",
                            "2026-11-05T10:15:00",
                            false,
                        ),
                        entry("Train", "2026-11-05T14:15:00", "2026-11-05T16:00:00", false),
                        entry("Trip", "2026-11-03", "2026-11-07", true),
                    ]));
                    page
                });
                cx.new(|cx| Root::new(page, window, cx))
            },
        );
        let result = result.and_then(|handle| {
            handle.update(cx, |_, window, cx| {
                window.draw(cx).clear(cx);
                window.render_to_image()?.save(&output)?;
                Ok::<_, Box<dyn std::error::Error>>(())
            })
        });
        match result {
            Ok(Ok(())) => println!("Saved {output}"),
            other => {
                eprintln!("Calendar render failed: {other:?}");
                std::process::exit(1);
            }
        }
        cx.quit();
    });
}

fn entry(title: &str, start: &str, end: &str, all_day: bool) -> CalendarEntry {
    CalendarEntry {
        id: title.into(),
        title: title.into(),
        starts_at: start.into(),
        ends_at: end.into(),
        all_day,
        kind: "event".into(),
        commitment: "planned".into(),
        source: "manual".into(),
        ..Default::default()
    }
}
