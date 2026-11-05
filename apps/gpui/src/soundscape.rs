//! Desktop soundscape slideover adapter re-exporting [`sjel_gpui_ui::soundscape::SoundscapeSlideover`].

use std::sync::Arc;

use gpui_kit::{App, Context};
use sjel_gpui_core::data::Repo;
use sjel_gpui_core::shell;

pub use sjel_gpui_ui::soundscape::SoundscapeSlideover as Soundscape;

pub fn new(repo: Repo, cx: &mut Context<Soundscape>) -> Soundscape {
    let audio = crate::audio::audio(cx);
    let device = audio.device_name().map(|name| match audio.sample_rate() {
        Some(rate) => format!("{name} · {rate:.0} Hz"),
        None => name.to_owned(),
    });
    let device_error = audio.error().map(str::to_owned);
    Soundscape::new_with_handles(
        repo,
        audio.control.clone(),
        audio.visual.clone(),
        device,
        device_error,
        cx,
    )
}

/// Start playing through the conductor, for `SJEL_GPUI_SOUNDSCAPE=play`.
pub fn play_through_conductor(cx: &mut App) {
    let audio = crate::audio::audio(cx);
    audio.control.set_playing(true);
    let endpoint = &sjel_gpui_core::environment::current().endpoint;
    let Ok(shell) = sjel_gpui_core::shell::Shell::new(endpoint) else {
        return;
    };
    let shell = Arc::new(shell);
    cx.spawn(async move |cx| {
        if let Some(Ok(state)) =
            shell::run(async move { shell.set_soundscape_playing(true).await }).await
        {
            cx.update(|cx| {
                let audio = crate::audio::audio(cx);
                audio.control.set_playing(state.playing);
            });
        }
    })
    .detach();
}
