//! Canvas view bindings and window framing for soundscape visualizers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{div, prelude::*, IntoElement};
use sjel_gpui_core::soundscape::Visual;

use super::field::Field;

/// The field, and the canvas element that paints it.
///
/// The size is explicit because a canvas has no content of its own: without it the element is laid
/// out at zero and the paint callback receives an empty rectangle, which looks exactly like a
/// visual that does not work.
pub fn canvas(field: Rc<RefCell<Field>>, visual: Arc<Visual>) -> impl IntoElement {
    gpui_kit::canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            field.borrow().paint(&visual, bounds, window);
        },
    )
    .w_full()
    .h_full()
}

/// The field's frame, so the window can hand it a bounded area.
pub fn frame(child: impl IntoElement) -> impl IntoElement {
    div().w_full().h_full().overflow_hidden().child(child)
}
