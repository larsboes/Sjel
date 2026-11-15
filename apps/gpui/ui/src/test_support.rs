//! Shared GPUI Kit + Ely test initialization and `Root` mounting for `sjel-gpui-ui` tests.

use std::{cell::RefCell, rc::Rc};

use gpui::{AppContext, Entity, Pixels, Render, Size, TestAppContext, VisualTestContext, Window};

/// Initializes GPUI Kit and Ely test assets/theme state on `cx` in the required order.
pub fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ely::init_for_tests(cx);
    });
}

/// Initializes Kit + Ely, mounts `build` inside `gpui_kit::component::Root`, optionally resizes
/// the window, and pumps the executor until parked.
pub fn mount_root<V: Render + 'static>(
    cx: &mut TestAppContext,
    viewport: Option<Size<Pixels>>,
    build: impl FnOnce(&mut Window, &mut gpui::Context<V>) -> V + 'static,
) -> (Entity<V>, &mut VisualTestContext) {
    init(cx);
    let slot = Rc::new(RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    if let Some(size) = viewport {
        cx.simulate_resize(size);
    }
    cx.run_until_parked();
    let view = slot.take().expect("mounted root view");
    (view, cx)
}
