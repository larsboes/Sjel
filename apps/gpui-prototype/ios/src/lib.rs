//! The iOS app's GPUI content. UIKit owns the window, the navigation bar and the safe areas
//! (`host/App.swift`); GPUI draws inside them (ISA.md Decisions 2026-10-15).
//!
//! Step 0 shows the dashboard's destinations and one loopback read of the shell. Authenticated
//! reads wait for device pairing: the Mac's inbound token lives in the Mac's login Keychain.

use gpui::{div, prelude::*, px, rgb, Context, SharedString, Window};
use sjel_gpui_core::{route, shell};

pub struct Destinations {
    loopback: SharedString,
}

impl Destinations {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let health = shell::run(async {
            let response = reqwest::get(format!("{}/health", shell::DEFAULT_ENDPOINT)).await?;
            response.text().await
        });
        cx.spawn(async move |this, cx| {
            let loopback = match health.await {
                Some(Ok(body)) => format!("GET /health: {body}"),
                Some(Err(error)) => format!("GET /health failed: {error}"),
                None => "GET /health: no answer".to_owned(),
            };
            let _ = this.update(cx, |view, cx| {
                view.loopback = loopback.into();
                cx.notify();
            });
        })
        .detach();
        Self {
            loopback: "GET /health: waiting".into(),
        }
    }
}

impl Render for Destinations {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("destinations")
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(0xf3f5f9))
            .text_color(rgb(0x1c1c1e))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x646b77))
                    .child(self.loopback.clone()),
            )
            .child(
                div()
                    .rounded(px(12.))
                    .bg(rgb(0xffffff))
                    .flex()
                    .flex_col()
                    .children(route::DESTINATIONS.iter().map(|d| {
                        div()
                            .px_4()
                            .py(px(10.))
                            .flex()
                            .justify_between()
                            .child(d.label)
                            .child(div().text_color(rgb(0x646b77)).child(d.href))
                    })),
            )
    }
}

/// Called by `host/App.swift` before `gpui_ios_run_demo`: registers what the GPUI window shows.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_register_app() {
    use gpui::{App, WindowOptions};
    use gpui_kit::component::Root;

    gpui_mobile::ios::ffi::set_app_callback(Box::new(|cx: &mut App| {
        gpui_kit::init(cx);
        let _ = cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(Destinations::new);
            cx.new(|cx| Root::new(view, window, cx))
        });
    }));
}
