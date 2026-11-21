//! `SJEL_GPUI_KEYS`: replay keys and text into the main window after launch, so an agent or a
//! screenshot script can drive a page the way a person would. GPUI draws its own pixels, so the
//! system's accessibility tools cannot type into it.
//!
//! Steps are separated by `;`: `wait 1500` pauses in milliseconds, `keys cmd-k enter` sends
//! keystrokes in GPUI's notation, `type Köln Hbf` types the text as written, and `click 320 96`
//! moves the pointer to that point in window pixels and clicks there.
//!
//! ```sh
//! SJEL_GPUI_OPEN=sjel://travel/connections \
//! SJEL_GPUI_KEYS='wait 1000; type Köln Hbf; wait 1500; keys enter' cargo run
//! ```

use std::time::Duration;

use gpui::{
    px, AnyWindowHandle, App, AppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, OperatingSystem, PlatformInput, Point,
};

/// About two frames at 60 Hz.
const KEY_GAP: Duration = Duration::from_millis(30);

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Wait(u64),
    Keys(Vec<Keystroke>),
    Click(f32, f32),
}

/// The script's steps, or the first step it cannot read.
fn parse(script: &str, os: OperatingSystem) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (verb, rest) = step.split_once(' ').unwrap_or((step, ""));
        steps.push(match verb {
            "wait" => Step::Wait(
                rest.trim()
                    .parse()
                    .map_err(|_| format!("wait needs milliseconds: {step:?}"))?,
            ),
            "keys" => Step::Keys(
                rest.split_whitespace()
                    .map(|k| {
                        Keystroke::parse_with_os(k, os)
                            .map_err(|_| format!("not a keystroke: {k:?}"))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            "type" => Step::Keys(rest.chars().map(typed).collect()),
            "click" => {
                let point: Vec<f32> = rest
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("click needs x and y in pixels: {step:?}"))?;
                match point[..] {
                    [x, y] => Step::Click(x, y),
                    _ => return Err(format!("click needs x and y in pixels: {step:?}")),
                }
            }
            _ => {
                return Err(format!(
                    "unknown step {step:?}: use wait, keys, type or click"
                ))
            }
        });
    }
    Ok(steps)
}

/// One typed character as the keystroke that produces it.
fn typed(c: char) -> Keystroke {
    match c {
        ' ' => Keystroke::parse("space").expect("space is a key"),
        c => Keystroke {
            modifiers: Modifiers::default(),
            key: c.to_lowercase().to_string(),
            key_char: Some(c.to_string()),
        },
    }
}

/// A pointer move to the point, so hover state settles, then a left press and release there.
fn click(x: f32, y: f32) -> [PlatformInput; 3] {
    let position = Point::new(px(x), px(y));
    let modifiers = Modifiers::default();
    [
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }),
    ]
}

/// Starts the script in `SJEL_GPUI_KEYS`, if there is one. A script that cannot be read is
/// reported on stderr and not run at all.
pub fn run_from_env(cx: &mut App) {
    let Ok(script) = std::env::var("SJEL_GPUI_KEYS") else {
        return;
    };
    let steps = match parse(&script, cx.platform_info().os) {
        Ok(steps) => steps,
        Err(error) => {
            eprintln!("SJEL_GPUI_KEYS: {error}");
            return;
        }
    };
    // Untyped, as GPUI's own test context dispatches: a typed handle holds the root view for the
    // whole call, and a keystroke that updates it again panics.
    let Some(window) = crate::main_window(cx).map(|(window, _)| AnyWindowHandle::from(window))
    else {
        return;
    };
    cx.spawn(async move |cx| {
        for step in steps {
            match step {
                Step::Wait(ms) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(ms))
                        .await
                }
                Step::Keys(keys) => {
                    for key in keys {
                        let _ = cx.update_window(window, |_, window, cx| {
                            window.dispatch_keystroke(key, cx);
                        });
                        // A field takes typed text through the input handler of the last frame,
                        // so each key waits for the next one.
                        cx.background_executor().timer(KEY_GAP).await;
                    }
                }
                Step::Click(x, y) => {
                    for event in click(x, y) {
                        let _ = cx.update_window(window, |_, window, cx| {
                            window.dispatch_event(event, cx);
                        });
                    }
                }
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_reads_as_waits_keys_and_text() {
        let steps = parse(
            "wait 500; keys cmd-k enter; type Kö l",
            OperatingSystem::Mac,
        )
        .unwrap();
        assert_eq!(steps[0], Step::Wait(500));
        let Step::Keys(keys) = &steps[1] else {
            panic!()
        };
        assert_eq!(keys.len(), 2);
        assert!(keys[0].modifiers.platform);
        let Step::Keys(typed) = &steps[2] else {
            panic!()
        };
        let chars: Vec<_> = typed
            .iter()
            .map(|k| k.key_char.clone().unwrap_or_default())
            .collect();
        assert_eq!(chars, ["K", "ö", "", "l"]);
        assert_eq!(typed[2].key, "space");
        assert!(parse("wait soon", OperatingSystem::Mac).is_err());
        assert_eq!(
            parse("click 320 96.5", OperatingSystem::Mac).unwrap(),
            [Step::Click(320., 96.5)]
        );
        assert!(parse("click 3", OperatingSystem::Mac).is_err());
        assert!(parse("tap 3 4", OperatingSystem::Mac).is_err());
    }
}
