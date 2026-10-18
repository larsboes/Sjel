//! Frame probe for `bench-scroll`. With `SJEL_GPUI_FRAMES=<file>`, every frame of the main
//! window appends `<render start µs> <render end µs> <paint end µs> <first unpainted input µs or 0>`,
//! all from app start.

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{canvas, prelude::*, AnyElement};

#[derive(Clone)]
pub struct FrameProbe(Rc<RefCell<Probe>>);

struct Probe {
    origin: Instant,
    render_start: u128,
    render_end: u128,
    pending_input: u128,
    /// In a self-driven bench, only frames while the bench runs are written.
    armed: bool,
    out: File,
}

impl FrameProbe {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("SJEL_GPUI_FRAMES")?;
        let out = File::create(path).ok()?;
        Some(Self(Rc::new(RefCell::new(Probe {
            origin: Instant::now(),
            render_start: 0,
            render_end: 0,
            pending_input: 0,
            armed: std::env::var_os("SJEL_GPUI_BENCH_SCROLL").is_none()
                && std::env::var_os("SJEL_GPUI_BENCH_NOTE").is_none(),
            out,
        }))))
    }

    pub fn render_started(&self) {
        let mut probe = self.0.borrow_mut();
        probe.render_start = probe.origin.elapsed().as_micros();
    }

    pub fn render_finished(&self) {
        let mut probe = self.0.borrow_mut();
        probe.render_end = probe.origin.elapsed().as_micros();
    }

    pub fn arm(&self, armed: bool) {
        self.0.borrow_mut().armed = armed;
    }

    /// A scroll or key event arrived; the next paint closes its latency.
    pub fn input(&self) {
        let mut probe = self.0.borrow_mut();
        if probe.pending_input == 0 {
            probe.pending_input = probe.origin.elapsed().as_micros();
        }
    }

    /// An empty element painted last, so its paint callback marks the end of the frame's paint.
    pub fn paint_marker(&self) -> AnyElement {
        let probe = self.0.clone();
        canvas(
            |_, _, _| {},
            move |_, _, _, _| {
                let mut probe = probe.borrow_mut();
                if !probe.armed {
                    return;
                }
                let end = probe.origin.elapsed().as_micros();
                let input = std::mem::take(&mut probe.pending_input);
                let line = format!(
                    "{} {} {} {}\n",
                    probe.render_start, probe.render_end, end, input
                );
                let _ = probe.out.write_all(line.as_bytes());
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }
}

/// `SJEL_GPUI_BENCH_SCROLL=1`: once data has loaded, scroll the page down and back up, one step
/// per 8 ms, then quit. No OS input and no focus needed, so it runs behind other apps.
pub fn drive_scroll_bench(
    probe: FrameProbe,
    scroll: gpui_kit::ScrollHandle,
    cx: &mut gpui_kit::Context<crate::workspace::Workspace>,
) {
    use gpui_kit::{point, px};
    use std::time::Duration;
    const STEPS: usize = 120;
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        probe.arm(true);
        let max = scroll.max_offset().y;
        let path = (0..=STEPS).chain((0..STEPS).rev());
        for step in path {
            scroll.set_offset(point(px(0.), -max * (step as f32 / STEPS as f32)));
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(8))
                .await;
        }
        probe.arm(false);
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// `SJEL_GPUI_BENCH_NOTE=<record path>`: open that note with saving off, type one character
/// into its last block every 8 ms, 240 in all, then quit. Measures what a keystroke costs.
pub fn drive_type_bench(
    probe: FrameProbe,
    path: String,
    notes: gpui_kit::Entity<crate::notes::Notes>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<crate::workspace::Workspace>,
) {
    use std::time::Duration;
    cx.spawn_in(window, async move |_, cx| {
        let editor = loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let opened = notes.update_in(cx, |notes, window, cx| {
                notes
                    .bench_open(&path, window, cx)
                    .then(|| notes.editor())
                    .flatten()
            });
            match opened {
                Ok(Some(editor)) => break editor,
                Ok(None) => continue,
                Err(_) => return,
            }
        };
        cx.background_executor().timer(Duration::from_secs(1)).await;
        probe.arm(true);
        for _ in 0..240 {
            probe.input();
            if editor
                .update_in(cx, |editor, window, cx| editor.bench_type(window, cx))
                .is_err()
            {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_millis(8))
                .await;
        }
        probe.arm(false);
        let _ = cx.update(|_, cx| cx.quit());
    })
    .detach();
}
