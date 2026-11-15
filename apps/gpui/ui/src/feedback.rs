//! Window-owned Ely toast presentation.

use gpui::{
    div, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
};
use gpui_kit::base::{Root, RootPlugin};

use crate::ely::feedback::{Toast, ToastViewport, Toaster};

struct Feedback {
    toaster: Entity<Toaster>,
}

impl RootPlugin for Feedback {}

impl Render for Feedback {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .child(ToastViewport::new("sjel-toasts", &self.toaster))
    }
}

/// Registers one toast viewport per window before constructing any roots.
pub fn init(cx: &mut App) {
    Root::register_plugin(cx, |_, cx: &mut Context<Feedback>| {
        let toaster = cx.new(|_| Toaster::default());
        cx.observe(&toaster, |_, _, cx| cx.notify()).detach();
        Feedback { toaster }
    });
}

/// Returns the window's toaster, installed by [`init`].
///
/// # Panics
/// Panics when the window has no root or feedback plugin.
pub fn toaster(window: &Window, cx: &App) -> Entity<Toaster> {
    Root::read(window, cx)
        .plugin::<Feedback>()
        .expect("register feedback before constructing the window root")
        .read(cx)
        .toaster
        .clone()
}

/// Shows mutation and workspace feedback in the current window.
pub trait WindowToastExt {
    /// Pushes a toast to the window's Ely viewport, installed by [`init`].
    ///
    /// # Panics
    /// Panics when the window has no root or feedback plugin.
    fn push_toast(&mut self, toast: Toast, cx: &mut App);
}

impl WindowToastExt for Window {
    fn push_toast(&mut self, toast: Toast, cx: &mut App) {
        toaster(self, cx).update(cx, |toaster, cx| {
            toaster.push(toast, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{EmptyView, TestAppContext};
    use gpui_kit::component::WindowExt;

    #[gpui::test]
    fn feedback_stays_in_its_window_and_never_uses_the_kit_queue(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let mut toasters = Vec::new();
        for title in ["First window", "Second window"] {
            let (_, window_cx) = crate::test_support::mount_root(cx, None, |_, _| EmptyView);
            let entity = window_cx.update(|window, cx| {
                window.push_toast(Toast::new(title), cx);
                assert!(window.notifications(cx).is_empty());
                toaster(window, cx)
            });
            window_cx.run_until_parked();
            toasters.push(entity);
        }
        assert_ne!(toasters[0], toasters[1]);
        for (entity, title) in toasters.iter().zip(["First window", "Second window"]) {
            entity.read_with(cx, |toaster, _| {
                assert_eq!(toaster.shown(), vec![(title.into(), None)]);
            });
        }
    }
}
