//! Sort a folder into Sjel through the `import` capability. Choose a folder, see where its files
//! would go, start the import, and read what the ledger holds for that folder afterwards.
//! Source files are only read; the capability moves and removes nothing.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, IntoElement, PathPromptOptions, SharedString,
    Window,
};

use crate::backup::size;
use crate::data::{self, Busy, Remote, Repo};
use crate::shell::{ImportJob, ImportPlan, ImportStatus};
use crate::theme::Palette;
use crate::ui::{dot, list, note, row, section};

const RUN: &str = "import:run";

pub struct ImportPage {
    repo: Repo,
    folder: Option<String>,
    plan: Remote<ImportPlan>,
    status: Remote<ImportStatus>,
    job: Remote<Option<ImportJob>>,
    busy: Busy,
    polling: bool,
}

impl ImportPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            folder: None,
            plan: Remote::default(),
            status: Remote::default(),
            job: Remote::default(),
            busy: Busy::default(),
            polling: false,
        }
    }

    /// Opening the page picks up a run started earlier, and its folder when none is chosen.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.job,
            &repo,
            |shell| async move { shell.import_current().await },
        );
        self.poll(cx);
    }

    fn running(&self) -> bool {
        self.job
            .value
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|job| !job.finished)
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.folder = Some(folder.display().to_string());
                view.plan = Remote::default();
                view.status = Remote::default();
                view.sort(cx);
                view.load_status(cx);
            });
        })
        .detach();
    }

    fn sort(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.plan,
            &repo,
            |shell| async move { shell.import_plan(&folder).await },
        );
    }

    fn load_status(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.status,
            &repo,
            |shell| async move { shell.import_status(&folder).await },
        );
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, RUN, "Import started."),
            move |shell| async move { shell.import_start(&folder).await },
            |this, started, cx| {
                if started.is_some() {
                    this.load(cx);
                }
            },
        );
    }

    /// Reads the run once a second while it is active, then the ledger once at the end.
    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let active = this.update(cx, |view, cx| {
                    if view.folder.is_none() {
                        view.folder = view.job.value.clone().flatten().map(|job| job.folder);
                    }
                    if view.running() && !view.job.loading {
                        let repo = view.repo.clone();
                        data::read(
                            cx,
                            view,
                            |s| &mut s.job,
                            &repo,
                            |shell| async move { shell.import_current().await },
                        );
                    }
                    view.running() || view.job.loading
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
            let _ = this.update(cx, |view, cx| {
                view.polling = false;
                view.load_status(cx);
            });
        })
        .detach();
    }

    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.folder.is_some();
        let running = self.running();
        div()
            .flex()
            .gap_2()
            .child(
                Button::new("import-choose")
                    .small()
                    .ghost()
                    .label("Choose folder…")
                    .disabled(running)
                    .on_click(cx.listener(|view, _, _, cx| view.choose(cx))),
            )
            .child(
                Button::new("import-run")
                    .small()
                    .label("Import")
                    .disabled(!chosen || running)
                    .loading(self.busy.contains(RUN) || running)
                    .on_click(cx.listener(|view, _, window, cx| view.start(window, cx))),
            )
            .into_any_element()
    }

    fn progress(&self, palette: Palette) -> Option<AnyElement> {
        let job = self.job.value.clone().flatten()?;
        if Some(&job.folder) != self.folder.as_ref() {
            return None;
        }
        let counts = job
            .outcomes
            .iter()
            .map(|(outcome, files)| format!("{files} {outcome}"))
            .chain((job.unchanged > 0).then(|| format!("{} unchanged", job.unchanged)))
            .collect::<Vec<_>>()
            .join(" · ");
        let (tone, text) = match (&job.error, job.finished) {
            (Some(error), _) => (palette.red, format!("Stopped: {error}")),
            (None, true) => (palette.green, format!("Finished. {counts}")),
            (None, false) => (
                palette.amber,
                format!("Importing {} of {} files. {counts}", job.done, job.total),
            ),
        };
        Some(note(text, tone))
    }
}

fn plan_rows(plan: &ImportPlan, palette: Palette) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = plan
        .routes
        .iter()
        .map(|route| {
            let tone = if route.route == "skip" {
                palette.muted
            } else {
                palette.accent
            };
            line(
                dot(tone),
                &route.route,
                route.files,
                Some(size(route.bytes)),
                palette,
            )
        })
        .collect();
    rows.extend(plan.skipped.iter().map(|skip| {
        line(
            div(),
            &format!("skip: {}", skip.reason),
            skip.files,
            None,
            palette,
        )
    }));
    rows
}

fn status_rows(status: &ImportStatus, palette: Palette) -> Vec<AnyElement> {
    status
        .rows
        .iter()
        .map(|row| {
            let tone = match row.outcome.as_str() {
                "imported" | "present" => palette.green,
                "pending" => palette.amber,
                "failed" => palette.red,
                _ => palette.muted,
            };
            let label = match &row.detail {
                Some(detail) => format!("{} {} · {detail}", row.route, row.outcome),
                None => format!("{} {}", row.route, row.outcome),
            };
            line(dot(tone), &label, row.files, None, palette)
        })
        .collect()
}

fn line(
    lead: impl IntoElement,
    label: &str,
    files: usize,
    bytes: Option<String>,
    palette: Palette,
) -> AnyElement {
    row()
        .child(div().w(px(7.)).child(lead))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(label.to_owned()),
        )
        .child(
            div()
                .w(px(80.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{files} files")),
        )
        .child(
            div()
                .w(px(72.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(bytes.unwrap_or_default()),
        )
        .into_any_element()
}

impl Render for ImportPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let folder: SharedString = self
            .folder
            .clone()
            .unwrap_or_else(|| "Choose a folder. Sjel sorts every file to where it belongs.".into())
            .into();
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(folder),
            )
            .child(self.controls(cx));
        let plan = match (&self.plan.value, &self.plan.error) {
            (Some(plan), _) => list(plan_rows(plan, palette), palette),
            (None, Some(error)) => note(error.to_string(), palette.red),
            (None, None) if self.plan.loading => note("Sorting…", palette.muted),
            (None, None) => div().into_any_element(),
        };
        let status = match (&self.status.value, &self.status.error) {
            (Some(status), _) if !status.rows.is_empty() => {
                list(status_rows(status, palette), palette)
            }
            (Some(_), _) => note("Nothing from this folder is recorded yet.", palette.muted),
            (None, Some(error)) => note(error.to_string(), palette.red),
            (None, None) => div().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(head)
            .children(self.progress(palette))
            .child(section("Where the files go", plan, palette))
            .child(section("In Sjel", status, palette))
    }
}
