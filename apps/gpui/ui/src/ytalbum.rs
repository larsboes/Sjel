//! Native album builder for long YouTube recordings.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, Context, Entity, EventEmitter, FontWeight, Window};
use sjel_gpui_core::shell::{self, ShellError, YtAlbumBuild, YtAlbumJob};

use crate::data::Repo;
use crate::theme::Palette;

pub enum YtAlbumEvent {
    Close,
}

pub struct YtAlbum {
    repo: Repo,
    palette: Option<Palette>,
    url: Entity<InputState>,
    album: Entity<InputState>,
    artist: Entity<InputState>,
    year: Entity<InputState>,
    tracklist: Entity<TextareaState>,
    probing: bool,
    starting: bool,
    message: Option<String>,
    error: Option<String>,
    job: Option<YtAlbumJob>,
}

impl EventEmitter<YtAlbumEvent> for YtAlbum {}

impl YtAlbum {
    pub fn new(repo: Repo, url: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("YouTube video URL");
            state.set_value(url.to_owned(), window, cx);
            state
        });
        let album = cx.new(|cx| InputState::new(window, cx).placeholder("Album name"));
        let artist = cx.new(|cx| InputState::new(window, cx).placeholder("Album artist"));
        let year = cx.new(|cx| InputState::new(window, cx).placeholder("Year"));
        let tracklist = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 14));
        Self {
            repo,
            palette: None,
            url: url_input,
            album,
            artist,
            year,
            tracklist,
            probing: false,
            starting: false,
            message: Some("Checking the video for chapters…".into()),
            error: None,
            job: None,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().to_string();
        if url.trim().is_empty() {
            self.error = Some("Enter a YouTube URL first.".into());
            cx.notify();
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.probing = true;
        self.error = None;
        self.message = Some("Checking the video for chapters…".into());
        cx.notify();
        let work = shell::run(async move { shell.ytalbum_probe(&url).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                view.probing = false;
                match result {
                    Ok(probe) => {
                        if view.album.read(cx).value().trim().is_empty() {
                            view.album.update(cx, |input, cx| {
                                input.set_value(probe.title.clone(), window, cx)
                            });
                        }
                        if view.artist.read(cx).value().trim().is_empty() {
                            view.artist.update(cx, |input, cx| {
                                input.set_value(probe.uploader.clone(), window, cx)
                            });
                        }
                        if !probe.tracklist_from_chapters.trim().is_empty()
                            && view.tracklist.read(cx).value().trim().is_empty()
                        {
                            view.tracklist.update(cx, |input, cx| {
                                input.set_value(probe.tracklist_from_chapters.clone(), window, cx)
                            });
                            view.message = Some(format!(
                                "{} · {} tracks filled from chapters",
                                probe.duration_hms,
                                probe.chapters.len()
                            ));
                        } else if probe.tracklist_from_chapters.trim().is_empty() {
                            view.message = Some(format!(
                                "{} · no chapters found; add track starts below",
                                probe.duration_hms
                            ));
                        } else {
                            view.message = Some(format!("{} · video ready", probe.duration_hms));
                        }
                    }
                    Err(error) => {
                        view.error = Some(error.to_string());
                        view.message = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let build = YtAlbumBuild {
            url: self.url.read(cx).value().to_string(),
            album: self.album.read(cx).value().trim().to_owned(),
            artist: self.artist.read(cx).value().trim().to_owned(),
            year: self.year.read(cx).value().trim().to_owned(),
            tracklist: self.tracklist.read(cx).value().to_string(),
        };
        if build.album.is_empty() || build.tracklist.trim().is_empty() {
            self.error = Some("Add an album name and tracklist before building.".into());
            cx.notify();
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.starting = true;
        self.error = None;
        self.message = Some("Starting album build…".into());
        cx.notify();
        let work = shell::run(async move { shell.ytalbum_build(&build).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update_in(cx, |view, window, cx| {
                view.starting = false;
                match result {
                    Ok(job) => {
                        view.message = Some("Album build is running…".into());
                        view.job = Some(job.clone());
                        view.watch_job(job.id, window, cx);
                    }
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn watch_job(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let shell = shell.clone();
            let id = id.clone();
            let result = shell::run(async move { shell.ytalbum_job(&id).await })
                .await
                .unwrap_or(Err(ShellError::Unreachable));
            let mut finished = false;
            let _ = this.update_in(cx, |view, _, cx| {
                match result {
                    Ok(job) => {
                        finished = job.state != "running";
                        if finished {
                            view.message = Some(if job.state == "done" {
                                "Album imported into Apple Music.".into()
                            } else {
                                "Album build failed.".into()
                            });
                        }
                        view.job = Some(job);
                    }
                    Err(error) => {
                        finished = true;
                        view.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
            if finished {
                break;
            }
        })
        .detach();
    }
}

impl Render for YtAlbum {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let tracklist_present = !self.tracklist.read(cx).value().trim().is_empty();
        let job_running = self.job.as_ref().is_some_and(|job| job.state == "running");
        let job = self.job.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("ytalbum-back")
                            .small()
                            .ghost()
                            .label("← Inbox")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(YtAlbumEvent::Close))),
                    )
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Build Apple Music album"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.url)))
                    .child(
                        Button::new("ytalbum-probe")
                            .label(if self.probing {
                                "Checking…"
                            } else {
                                "Probe video"
                            })
                            .disabled(self.probing || self.starting || job_running)
                            .on_click(cx.listener(|view, _, window, cx| view.probe(window, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.album)))
                    .child(div().flex_1().child(Input::new(&self.artist)))
                    .child(div().w(px(110.)).child(Input::new(&self.year))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Tracklist: one start time per line, followed by artist and title."),
            )
            .child(Textarea::new(&self.tracklist).h(px(230.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("ytalbum-build")
                            .label(if self.starting {
                                "Starting…"
                            } else if job_running {
                                "Building…"
                            } else {
                                "Build album"
                            })
                            .disabled(
                                self.probing || self.starting || job_running || !tracklist_present,
                            )
                            .on_click(cx.listener(|view, _, window, cx| view.build(window, cx))),
                    )
                    .when_some(self.message.clone(), |el, message| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(message),
                        )
                    }),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            })
            .when_some(job, |el, job| {
                el.child(
                    div()
                        .id("ytalbum-job")
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
                        ),
                )
            })
    }
}
