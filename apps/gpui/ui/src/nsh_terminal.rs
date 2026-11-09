//! NSH terminal grid and native keyboard over the shared VT parser and signed pty API.
use crate::{
    nsh_ui::{action, caption, field, heading, snapshot},
    terminal_screen::{spans, Screen, Span},
    theme::MONOKAI,
};
use alacritty_terminal::{index::Side, selection::SelectionType};
use futures::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use nsh_core::{element::TouchEvent, prelude::*, TouchPhase};
use sjel_gpui_core::{
    shell,
    terminal::{Host, Options, Position, Terminal, Update, MAX_INPUT_BYTES},
};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Default)]
struct Snapshot {
    sessions: Vec<Terminal>,
    selected: Option<String>,
    lines: Vec<Vec<Span>>,
    cursors: Vec<(usize, usize)>,
    options: Options,
    unlocked: bool,
    busy: bool,
    error: Option<String>,
    status: String,
    font_size: u8,
    appearance: crate::AppearanceSettings,
}
struct State {
    snap: Snapshot,
    host: Option<Host>,
    screens: HashMap<String, Screen>,
    positions: HashMap<String, Position>,
    generation: u64,
    request: Option<AbortHandle>,
    size: (usize, usize),
    drag: Option<(f32, f32)>,
    select: bool,
}
struct Input {
    id: String,
    data: Vec<u8>,
    generation: u64,
}
type ZoomHost = Box<dyn Fn(i32) + Send>;
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
    input: OnceLock<mpsc::Sender<Input>>,
    unlock: Mutex<Option<Box<dyn Fn() + Send>>>,
    zoom: Mutex<Option<ZoomHost>>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let snap = Snapshot {
            font_size: 13,
            ..Default::default()
        };
        let (tx, _) = watch::channel(snap.clone());
        Feed {
            state: Mutex::new(State {
                snap,
                host: None,
                screens: HashMap::new(),
                positions: HashMap::new(),
                generation: 0,
                request: None,
                size: (48, 28),
                drag: None,
                select: false,
            }),
            tx,
            input: OnceLock::new(),
            unlock: Mutex::new(None),
            zoom: Mutex::new(None),
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &mut State) {
    s.snap.cursors = s
        .snap
        .selected
        .as_ref()
        .and_then(|id| s.screens.get(id))
        .map(|grid| {
            grid.lines()
                .iter()
                .enumerate()
                .filter_map(|(row, line)| line.cursor.map(|col| (row, col)))
                .collect()
        })
        .unwrap_or_default();
    s.snap.lines = s
        .snap
        .selected
        .as_ref()
        .and_then(|id| s.screens.get(id))
        .map(|grid| {
            grid.lines()
                .into_iter()
                .map(|line| spans(line, &MONOKAI))
                .collect()
        })
        .unwrap_or_default();
    feed().tx.send_replace(s.snap.clone());
}
/// Registers the host's Face ID or device-passcode request.
pub fn set_unlock(unlock: impl Fn() + Send + 'static) {
    *feed().unlock.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(unlock));
}
/// Replaces the paired host and locks the terminal before any new pty request.
pub fn connect(host: Host) {
    {
        let mut s = state();
        if let Some(old) = s.request.take() {
            old.abort();
        }
        s.generation += 1;
        s.host = Some(host);
        s.snap.unlocked = false;
        s.screens.clear();
        s.positions.clear();
        s.snap.selected = None;
        s.snap.sessions.clear();
        publish(&mut s);
    }
    feed().input.get_or_init(|| {
        let (tx, mut rx) = mpsc::channel::<Input>(64);
        shell::spawn(async move {
            while let Some(input) = rx.recv().await {
                let host = {
                    let s = state();
                    if !s.snap.unlocked || s.generation != input.generation {
                        continue;
                    }
                    s.host.clone()
                };
                if let Some(host) = host {
                    if let Err(e) = host.input(&input.id, &input.data).await {
                        let mut s = state();
                        if s.generation == input.generation {
                            s.snap.error = Some(e);
                            publish(&mut s);
                        }
                    }
                }
            }
        });
        tx
    });
}
/// Invalidates streams and removes visible terminal contents on background entry.
pub fn set_unlocked(unlocked: bool) {
    {
        let mut s = state();
        s.snap.unlocked = unlocked;
        s.snap.busy = false;
        s.generation += 1;
        if let Some(old) = s.request.take() {
            old.abort();
        }
        if !unlocked {
            s.screens.clear();
            s.positions.clear();
            s.snap.sessions.clear();
            s.snap.selected = None;
        }
        publish(&mut s);
    }
    if unlocked {
        refresh();
    }
}
/// Applies the persisted terminal font size to the grid.
pub fn set_appearance(appearance: crate::AppearanceSettings) {
    let mut s = state();
    if s.snap.appearance == appearance && s.snap.font_size == appearance.terminal_font_size {
        return;
    }
    s.snap.font_size = appearance.terminal_font_size;
    s.snap.appearance = appearance;
    publish(&mut s);
}
fn unlock() {
    if let Some(unlock) = feed()
        .unlock
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        unlock();
    }
}
/// Lists live ptys only after the native authentication gate succeeded.
pub fn refresh() {
    let (host, generation, size) = {
        let s = state();
        if !s.snap.unlocked {
            return;
        }
        (s.host.clone(), s.generation, s.size)
    };
    let Some(host) = host else {
        return;
    };
    shell::spawn(async move {
        let (sessions, options) = futures::join!(host.terminals(), host.options());
        let mut attach = None;
        {
            let mut s = state();
            if s.generation != generation || !s.snap.unlocked {
                return;
            }
            match sessions {
                Ok(sessions) => {
                    if s.snap.selected.is_none() {
                        attach = sessions.iter().find(|t| t.alive).map(|t| t.id.clone());
                    }
                    s.snap.sessions = sessions;
                }
                Err(e) => s.snap.error = Some(e),
            }
            match options {
                Ok(options) => s.snap.options = options,
                Err(e) => s.snap.error = Some(e),
            }
            publish(&mut s);
        }
        if let Some(id) = attach {
            select(id);
        } else if state().snap.selected.is_none() {
            open("shell".into(), None, size);
        }
    });
}
fn open(preset: String, cwd: Option<String>, size: (usize, usize)) {
    let (host, generation) = {
        let mut s = state();
        if !s.snap.unlocked || s.snap.busy {
            return;
        }
        let Some(host) = s.host.clone() else {
            return;
        };
        s.snap.busy = true;
        publish(&mut s);
        (host, s.generation)
    };
    shell::spawn(async move {
        let result = host
            .open(&preset, cwd.as_deref(), size.0 as u16, size.1 as u16)
            .await;
        let id = {
            let mut s = state();
            if s.generation != generation {
                return;
            }
            s.snap.busy = false;
            match result {
                Ok(id) => Some(id),
                Err(e) => {
                    s.snap.error = Some(e);
                    None
                }
            }
        };
        if let Some(id) = id {
            select(id);
            refresh();
        } else {
            let mut s = state();
            publish(&mut s);
        }
    });
}
fn select(id: String) {
    let (host, generation, registration) = {
        let mut s = state();
        if !s.snap.unlocked {
            return;
        }
        let Some(host) = s.host.clone() else {
            return;
        };
        if let Some(old) = s.request.take() {
            old.abort();
        }
        s.generation += 1;
        let (abort, registration) = AbortHandle::new_pair();
        s.request = Some(abort);
        let (cols, rows) = s.size;
        s.screens
            .entry(id.clone())
            .or_insert_with(|| Screen::new(cols, rows));
        s.positions.entry(id.clone()).or_default();
        s.snap.selected = Some(id.clone());
        s.snap.error = None;
        publish(&mut s);
        (host, s.generation, registration)
    };
    shell::spawn(async move {
        let _ = Abortable::new(
            async move {
                for _ in 0..5 {
                    let offset = {
                        let s = state();
                        if s.generation != generation || !s.snap.unlocked {
                            return;
                        }
                        s.positions.get(&id).copied().unwrap_or_default().offset()
                    };
                    match host.stream(&id, offset).await {
                        Ok(mut stream) => {
                            while let Some(update) = stream.next().await {
                                let mut s = state();
                                if s.generation != generation || !s.snap.unlocked {
                                    return;
                                }
                                match update {
                                    Update::Output { offset, data } => {
                                        if s.positions
                                            .get_mut(&id)
                                            .is_some_and(|p| p.accept(offset, data.len()))
                                        {
                                            if let Some(grid) = s.screens.get_mut(&id) {
                                                grid.feed(&data);
                                            }
                                        }
                                    }
                                    Update::Truncated { first_offset } => {
                                        s.positions
                                            .entry(id.clone())
                                            .or_default()
                                            .restart(first_offset);
                                        let (cols, rows) = s.size;
                                        s.screens.insert(id.clone(), Screen::new(cols, rows));
                                        publish(&mut s);
                                        break;
                                    }
                                    Update::Exit { code, signal } => {
                                        s.snap.status =
                                            signal.unwrap_or_else(|| format!("Exited with {code}"));
                                        publish(&mut s);
                                        return;
                                    }
                                    Update::Command(command) => {
                                        s.snap.status = if command.running {
                                            "Command running".into()
                                        } else {
                                            format!("Last exit: {:?}", command.last_exit_code)
                                        }
                                    }
                                }
                                publish(&mut s);
                            }
                        }
                        Err(e) => {
                            let mut s = state();
                            if s.generation != generation {
                                return;
                            }
                            s.snap.error = Some(e);
                            publish(&mut s);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                let mut s = state();
                if s.generation == generation {
                    s.snap.error =
                        Some("Terminal stream disconnected. Select its tab to reconnect.".into());
                    publish(&mut s);
                }
            },
            registration,
        )
        .await;
    });
}
fn send(data: Vec<u8>) {
    let input = {
        let s = state();
        if !s.snap.unlocked {
            return;
        }
        if data.len() > MAX_INPUT_BYTES {
            drop(s);
            let mut s = state();
            s.snap.error = Some(format!(
                "Paste exceeds the terminal's {MAX_INPUT_BYTES}-byte input limit."
            ));
            publish(&mut s);
            return;
        }
        let Some(id) = s.snap.selected.clone() else {
            return;
        };
        Input {
            id,
            data,
            generation: s.generation,
        }
    };
    if feed()
        .input
        .get()
        .is_none_or(|tx| tx.try_send(input).is_err())
    {
        let mut s = state();
        s.snap.error = Some("Terminal input queue is full. Wait before typing again.".into());
        publish(&mut s);
    }
}
fn resize(width: f32, height: f32) {
    let (host, id, size, generation) = {
        let mut s = state();
        let (cell, line) = cell_size(&s.snap);
        let size = (
            ((width / cell).floor() as usize).clamp(10, 300),
            ((height / line).floor() as usize).clamp(4, 200),
        );
        if s.size == size || !s.snap.unlocked {
            return;
        }
        s.size = size;
        for grid in s.screens.values_mut() {
            if grid.cols() != size.0 || grid.rows() != size.1 {
                grid.resize(size.0, size.1);
            }
        }
        publish(&mut s);
        (s.host.clone(), s.snap.selected.clone(), size, s.generation)
    };
    if let (Some(host), Some(id)) = (host, id) {
        shell::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            {
                let s = state();
                if s.generation != generation || s.size != size || !s.snap.unlocked {
                    return;
                }
            }
            if let Err(e) = host.resize(&id, size.0 as u16, size.1 as u16).await {
                let mut s = state();
                if s.generation == generation {
                    s.snap.error = Some(e);
                    publish(&mut s);
                }
            }
        });
    }
}
fn close() {
    let (host, id, generation) = {
        let s = state();
        (s.host.clone(), s.snap.selected.clone(), s.generation)
    };
    let (Some(host), Some(id)) = (host, id) else {
        return;
    };
    shell::spawn(async move {
        let answer = host.close(&id).await;
        {
            let mut s = state();
            if s.generation != generation {
                return;
            }
            match answer {
                Ok(()) => {
                    if let Some(old) = s.request.take() {
                        old.abort();
                    }
                    s.snap.selected = None;
                    s.screens.remove(&id);
                    s.positions.remove(&id);
                    s.snap.sessions.retain(|t| t.id != id);
                }
                Err(e) => s.snap.error = Some(e),
            }
            publish(&mut s);
        }
        refresh();
    });
}
fn touch(event: &TouchEvent) {
    let Some(point) = event.points.first() else {
        return;
    };
    let at = (point.location.x.0, point.location.y.0);
    let mut s = state();
    if !s.snap.unlocked {
        return;
    }
    let Some(id) = s.snap.selected.clone() else {
        return;
    };
    let (cell, line) = cell_size(&s.snap);
    let selecting = s.select;
    match event.phase {
        TouchPhase::Start => {
            s.drag = Some(at);
            if selecting {
                if let Some(grid) = s.screens.get_mut(&id) {
                    let point = grid.point_at(
                        (at.0 / cell).max(0.) as usize,
                        (at.1 / line).max(0.) as usize,
                    );
                    grid.select(SelectionType::Simple, point, Side::Left);
                }
            }
        }
        TouchPhase::Move => {
            if let Some(before) = s.drag {
                if let Some(grid) = s.screens.get_mut(&id) {
                    if selecting {
                        let point = grid.point_at(
                            (at.0 / cell).max(0.) as usize,
                            (at.1 / line).max(0.) as usize,
                        );
                        grid.extend_selection(point, Side::Right);
                    } else {
                        let rows = ((at.1 - before.1) / line).round() as i32;
                        if rows != 0 {
                            grid.scroll(rows);
                            s.drag = Some(at);
                        }
                    }
                }
            }
        }
        TouchPhase::End | TouchPhase::Cancel => {
            s.drag = None;
            if let Some(grid) = s.screens.get_mut(&id) {
                grid.drop_empty_selection();
            }
        }
    }
    publish(&mut s);
}
/// Registers persisted terminal font-size changes with the application coordinator.
pub fn set_zoom(zoom: impl Fn(i32) + Send + 'static) {
    *feed().zoom.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(zoom));
}
fn zoom(steps: i32) {
    if let Some(zoom) = feed()
        .zoom
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        zoom(steps);
    }
}

fn cell_size(snap: &Snapshot) -> (f32, f32) {
    let size = snap.font_size.max(1);
    let family = snap
        .appearance
        .terminal_font
        .resolve(&["Menlo".into()], "Menlo");
    (
        measure_text(&format!("{size}px {family}"), "M").0.max(1.),
        f32::from(size) * 1.4,
    )
}

fn edit_bytes(before: &str, after: &str) -> Vec<u8> {
    let common = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let mut bytes = vec![0x7f; before[common..].chars().count()];
    bytes.extend_from_slice(&after.as_bytes()[common..]);
    bytes
}

/// Authenticated pty tabs with the same ANSI grid, scrollback, selection and paste rules.
#[component]
pub fn TerminalScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let unlocked = computed(move || snap.with(|s| s.unlocked));
    view().flex_1().child(dynamic(move || {
        if !unlocked.get() { view().px_4().child(heading("Terminal locked")).child(caption("Confirm with Face ID or the device passcode before opening a terminal on the Mac.")).child(action("Unlock terminal", unlock)).into_element() }
        else { terminal_content().into_element() }
    }))
}
#[component]
fn TerminalContent() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let draft = signal(String::new());
    let last = stored(String::new());
    let cwd = signal(String::new());
    let confirm = signal(false);
    let frame = signal((320., 350.));
    let selected = computed(move || snap.with(|s| s.selected.clone()));
    let previous = stored(state().snap.selected.clone());
    effect(move || {
        let selected = selected.get();
        if selected != previous.get() {
            previous.replace(selected);
            draft.set(String::new());
            last.replace(String::new());
            confirm.set(false);
        }
    });
    let tools = signal(false);
    let raw = move |bytes: &[u8]| {
        draft.set(String::new());
        last.replace(String::new());
        send(bytes.to_vec());
    };
    view()
        .flex_1()
        .gap_1()
        .child(heading("Terminal"))
        .child(scroll().horizontal().h(px(44.)).child(dynamic(move || {
            let mut out = view().flex_row();
            for terminal in snap.with(|s| s.sessions.clone()) {
                let id = terminal.id.clone();
                out = out.child(action(terminal.label(), move || select(id.clone())));
            }
            out
        })))
        .child(
            canvas(move || {
                let current = snap.get();
                let font = f32::from(current.font_size);
                let (width, height) = frame.get();
                let (cell, line) = cell_size(&current);
                let mut p = Painter::new()
                    .fill_color(color(nsh_core::element::AdaptiveColor {
                        light: nsh_core::color::rgb(crate::theme::nsh_terminal_background(
                            &current.appearance,
                            false,
                        ))
                        .into(),
                        dark: nsh_core::color::rgb(crate::theme::nsh_terminal_background(
                            &current.appearance,
                            true,
                        ))
                        .into(),
                    }))
                    .fill_rect(0., 0., width, height);
                for (row, spans) in current.lines.into_iter().enumerate() {
                    for span in spans {
                        let x = span.column as f32 * cell;
                        let y = row as f32 * line;
                        if let Some(bg) = span.bg {
                            p = p.fill_color(nsh_core::color::rgb(bg)).fill_rect(
                                x,
                                y,
                                span.text.chars().count() as f32 * cell,
                                line,
                            );
                        }
                        p = p
                            .fill_color(nsh_core::color::rgb(span.fg))
                            .font(&format!(
                                "{}{}px {}",
                                if span.bold { "bold " } else { "" },
                                font,
                                current
                                    .appearance
                                    .terminal_font
                                    .resolve(&["Menlo".into()], "Menlo")
                            ))
                            .fill_text(&span.text, x, y + font);
                    }
                }
                for (row, col) in current.cursors {
                    p = p
                        .fill_color(nsh_core::color::rgb(MONOKAI.cursor))
                        .fill_rect(col as f32 * cell, (row + 1) as f32 * line - 2., cell, 2.);
                }
                p
            })
            .flex_1()
            .min_h(px(80.))
            .overflow_hidden()
            .test_id("terminal-grid")
            .claim_touches(TouchClaim::on_start())
            .on_touch(touch)
            .on_layout(move |bounds: &nsh_core::Bounds<nsh_core::Points>| {
                frame.set((bounds.size.width.0, bounds.size.height.0));
                resize(bounds.size.width.0, bounds.size.height.0);
            }),
        )
        .child(
            nsh_core::element::input()
                .test_id("terminal-keyboard")
                .on_key_press(move |key: &str| match key {
                    "ArrowUp" | "Up" => raw(b"\x1b[A"),
                    "ArrowDown" | "Down" => raw(b"\x1b[B"),
                    "ArrowLeft" | "Left" => raw(b"\x1b[D"),
                    "ArrowRight" | "Right" => raw(b"\x1b[C"),
                    "Escape" => raw(&[27]),
                    "Tab" => raw(&[9]),
                    "Backspace" if draft.get().is_empty() => send(vec![0x7f]),
                    _ => {}
                })
                .masked(true)
                .placeholder("Terminal keyboard")
                .auto_correct(false)
                .auto_capitalize(AutoCapitalize::None)
                .value(move || draft.get())
                .on_change(move |value: &str| {
                    let bytes = edit_bytes(&last.get(), value);
                    last.replace(value.to_owned());
                    draft.set(value.to_owned());
                    if !bytes.is_empty() {
                        send(bytes);
                    }
                })
                .on_submit(move |_: &str| {
                    send(b"\r".to_vec());
                    last.replace(String::new());
                    draft.set(String::new());
                }),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Esc", move || raw(b"\x1b")))
                .child(action("Tab", move || raw(b"\t")))
                .child(action("←", move || raw(b"\x1b[D")))
                .child(action("↑", move || raw(b"\x1b[A")))
                .child(action("↓", move || raw(b"\x1b[B")))
                .child(action("→", move || raw(b"\x1b[C")))
                .child(action("Ctrl-C", move || raw(&[3])))
                .child(action("Return", move || {
                    send(b"\r".to_vec());
                    last.replace(String::new());
                    draft.set(String::new());
                })),
        )
        .child(action("Terminal tools", move || tools.update(|v| *v = !*v)))
        .child(dynamic(move || {
            tools.get().then(|| {
                scroll()
                    .h(px(120.))
                    .child(terminal_tools(snap, cwd, confirm))
            })
        }))
        .child(
            text(move || snap.with(|s| s.error.clone().unwrap_or_else(|| s.status.clone())))
                .text_sm(),
        )
}

fn terminal_tools(
    snap: Signal<Snapshot>,
    cwd: Signal<String>,
    confirm: Signal<bool>,
) -> impl IntoElement {
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Refresh terminals", refresh))
                .child(action("Scroll back", || {
                    let mut s = state();
                    let id = s.snap.selected.clone();
                    if let Some(grid) = id.and_then(|id| s.screens.get_mut(&id)) {
                        grid.scroll(10);
                    }
                    publish(&mut s);
                }))
                .child(action("Live output", || {
                    let mut s = state();
                    let id = s.snap.selected.clone();
                    if let Some(grid) = id.and_then(|id| s.screens.get_mut(&id)) {
                        grid.scroll(-10000);
                    }
                    publish(&mut s);
                })),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Select text", || {
                    let mut s = state();
                    s.select = !s.select;
                    if !s.select {
                        let id = s.snap.selected.clone();
                        if let Some(grid) = id.and_then(|id| s.screens.get_mut(&id)) {
                            grid.clear_selection();
                        }
                    }
                    publish(&mut s);
                }))
                .child(action("Select all", || {
                    let mut s = state();
                    let id = s.snap.selected.clone();
                    if let Some(grid) = id.and_then(|id| s.screens.get_mut(&id)) {
                        grid.select_all();
                    }
                    publish(&mut s);
                }))
                .child(action("Copy", || {
                    let text = {
                        let s = state();
                        s.snap
                            .selected
                            .as_ref()
                            .and_then(|id| s.screens.get(id))
                            .and_then(Screen::selected_text)
                    };
                    if let Some(text) = text {
                        set_clipboard(text);
                    }
                }))
                .child(action("Paste", || {
                    spawn(async {
                        let text = clipboard_string().await;
                        let bracketed = {
                            let s = state();
                            s.snap
                                .selected
                                .as_ref()
                                .and_then(|id| s.screens.get(id))
                                .is_some_and(Screen::bracketed_paste)
                        };
                        let text = text.replace("\r\n", "\r").replace('\n', "\r");
                        send(if bracketed {
                            format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
                                .into_bytes()
                        } else {
                            text.into_bytes()
                        });
                    });
                })),
        )
        .child(
            view()
                .flex_row()
                .child(action("Smaller text", || zoom(-1)))
                .child(action("Larger text", || zoom(1))),
        )
        .child(field("New terminal working directory · optional", cwd))
        .child(dynamic(move || {
            let mut out = view().flex_row().flex_wrap();
            for preset in snap.with(|s| s.options.presets.clone()) {
                out = out.child(action(format!("New {preset}"), move || {
                    let cwd = cwd.get();
                    let size = state().size;
                    open(preset.clone(), (!cwd.is_empty()).then_some(cwd), size);
                }));
            }
            out
        }))
        .child(dynamic(move || {
            action(
                if confirm.get() {
                    "Confirm terminate terminal"
                } else {
                    "Terminate terminal"
                },
                move || {
                    if confirm.get() {
                        close();
                        confirm.set(false);
                    } else {
                        confirm.set(true);
                    }
                },
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_edits_preserve_shared_prefix_and_delete_characters() {
        assert_eq!(edit_bytes("café", "café!"), b"!");
        assert_eq!(edit_bytes("café!", "café"), vec![0x7f]);
        assert_eq!(edit_bytes("abcd", "abXY"), b"\x7f\x7fXY");
        assert_eq!(edit_bytes("東京", "東"), vec![0x7f]);
    }
    #[test]
    fn background_lock_clears_the_grid_and_native_unlock_is_explicit() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        {
            let mut s = state();
            s.snap.unlocked = true;
            s.snap.selected = Some("pty-test".into());
            let mut grid = Screen::new(20, 10);
            grid.feed(b"private output");
            s.screens.insert("pty-test".into(), grid);
            publish(&mut s);
            assert!(!s.snap.lines.is_empty());
        }
        set_unlocked(false);
        let s = state();
        assert!(s.screens.is_empty());
        assert!(s.snap.lines.is_empty());
        assert!(s.snap.selected.is_none());
        drop(s);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        set_unlock(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<TerminalScreen>())
        })
        .unwrap();
        app.wait_for("text:Terminal locked").unwrap();
        app.press("text:Unlock terminal").unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!state().snap.unlocked);
        send(b"rm".to_vec());
        assert!(state().snap.selected.is_none());
        {
            let mut s = state();
            s.snap.unlocked = true;
            s.snap.selected = Some("fixture".into());
            let mut grid = Screen::new(20, 10);
            grid.feed(b"\x1b[31mred output\x1b[0m");
            s.screens.insert("fixture".into(), grid);
            publish(&mut s);
        }
        app.wait_for("id:terminal-grid").unwrap();
        app.press("text:Terminal tools").unwrap();
        app.wait_for("text:Terminate terminal").unwrap();
        app.press("text:Terminate terminal").unwrap();
        app.wait_for("text:Confirm terminate terminal").unwrap();
        set_unlocked(false);
        app.wait_for("text:Terminal locked").unwrap();
    }
}
