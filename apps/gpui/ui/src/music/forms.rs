//! Inputs, mode-specific form sections, and background job log renderer.

use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, FontWeight};
use sjel_gpui_core::shell::MusicJob;

use super::Music;
use crate::theme::Palette;

pub fn render_song_inputs(music: &Music) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&music.title)))
                .child(div().flex_1().child(Input::new(&music.artist)))
                .child(div().w(px(110.)).child(Input::new(&music.year))),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&music.start)))
                .child(div().flex_1().child(Input::new(&music.end))),
        )
        .into_any_element()
}

pub fn render_album_inputs(music: &Music, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&music.album)))
                .child(div().flex_1().child(Input::new(&music.artist)))
                .child(div().w(px(110.)).child(Input::new(&music.year))),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Tracklist: one start time per line, followed by artist and title."),
        )
        .child(Textarea::new(&music.tracklist).h(px(230.)))
        .into_any_element()
}

pub fn render_job_card(job: MusicJob, palette: Palette) -> AnyElement {
    div()
        .id("music-job")
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{} · {}", job.album, job.state)),
        )
        .when_some(job.outdir.clone(), |el, path| {
            el.child(div().text_xs().text_color(rgb(palette.muted)).child(path))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(job.log.into_iter().map(|line| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(line)
                        .into_any_element()
                })),
        )
        .into_any_element()
}
