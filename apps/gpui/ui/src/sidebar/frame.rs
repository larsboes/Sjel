use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, FontWeight, MouseButton, SharedString, Window,
};

use super::types::Seat;
use crate::Palette;

/// The width of the overlay edge on a phone: an edge panel, not a full-screen sheet, so the page it
/// slid over stays visible enough to know where you are.
pub const OVERLAY_WIDTH: f32 = 340.;

/// The frame: title, seat chips, close, and the body a host hands in.
///
/// `on_seat` and `on_close` are called from the tap handlers; the host decides what those mean (the
/// phone pauses the sound when the edge goes away). Two seats always get their chips, because a seat
/// nobody can see is one nobody switches to.
pub fn sidebar_frame(
    palette: Palette,
    seat: Seat,
    body: AnyElement,
    on_seat: impl Fn(Seat, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut chips = div().flex().flex_wrap().items_center().gap_1();
    // One callback for every chip: an `Rc` because each chip's handler owns its own closure, and the
    // frame is drawn on one thread.
    let on_seat = std::rc::Rc::new(on_seat);
    for option in Seat::ALL {
        let is_active = option == seat;
        let on_seat = on_seat.clone();
        chips = chips.child(
            div()
                .id(SharedString::from(format!("sidebar-seat-{}", option.key())))
                .px_3()
                .py(px(6.))
                .rounded_full()
                .text_xs()
                .when(is_active, |chip| {
                    chip.bg(rgb(palette.accent))
                        .text_color(rgb(0xffffff))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!is_active, |chip| {
                    chip.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                })
                .child(option.label())
                .on_mouse_down(MouseButton::Left, move |_, window, app| {
                    on_seat(option, window, app)
                }),
        );
    }
    div()
        .id("sidebar")
        .size_full()
        .flex()
        .flex_col()
        // The terminal seat's frame takes the terminal's opacity, so lowering it lets the window
        // show through the grid and the frame alike, with the frame still tinted enough to read.
        .bg(rgba(if seat == Seat::Terminal {
            (palette.panel() & 0xffff_ff00) | (palette.terminal_surface & 0xff)
        } else {
            palette.panel()
        }))
        .text_color(rgb(palette.text))
        // Title and close share the first line and the chips wrap below, so a narrow edge never
        // pushes the close button out of reach.
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .pb(px(0.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child(seat.title()),
                        )
                        .child(
                            div()
                                .id("sidebar-close")
                                .flex_none()
                                .size(px(36.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgba(palette.raised))
                                .text_color(rgb(palette.muted))
                                .child("✕")
                                .on_mouse_down(MouseButton::Left, move |_, window, app| {
                                    on_close(window, app)
                                }),
                        ),
                )
                .child(chips),
        )
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

/// The scrim behind an overlay edge, which is what the phone draws: the panel stands at the right
/// edge at full height, and a tap anywhere on the dimmed page beside it puts it away. A docked panel
/// has no scrim, so this stays a separate piece a host may skip.
pub fn sidebar_scrim(
    on_tap: impl Fn(&mut Window, &mut App) + 'static,
    body: AnyElement,
) -> AnyElement {
    div()
        .id("sidebar-overlay")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(rgba(0x00000059))
        // A tap that closes the edge must not reach the page underneath it. The close button sits
        // exactly where Today's gear does, so without this a tap on ✕ also opened Settings: the frame
        // had already taken the overlay away when the finger came up, and the up landed on the page.
        .occlude()
        .flex()
        .flex_row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .on_mouse_down(MouseButton::Left, move |_, window, app| on_tap(window, app)),
        )
        .child(
            div()
                .flex_none()
                .w(px(OVERLAY_WIDTH))
                .h_full()
                .flex()
                .flex_col()
                .child(body),
        )
        .into_any_element()
}
