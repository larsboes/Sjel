//! Posts: drafts with text revisions, ordered Photos media, collages and upload exports.
//!
//! Backed by `crates/posts/posts` via sjel-status. Photos are added from the current
//! selection in the Photos app, so the page never asks for a media item ID.

use std::path::PathBuf;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Input, TextInput};
use crate::ely::theme::ControlSize;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    div, img, prelude::*, px, AnyElement, Context, Entity, FontWeight, IntoElement, ObjectFit,
    Render, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::{Palette, ThemeContext};
use sjel_gpui_core::posts::{
    next_accent, slot_label, PostDetail, PostMediaItem, PostsView, PostsWriteRequest, STATES,
};

pub struct PostsPage {
    pub repo: Repo,
    pub list: Remote<PostsView>,
    pub detail: Remote<PostDetail>,
    pub busy: Busy,
    pub selected: Option<String>,
    /// The slug whose text the editor holds, so a reload never overwrites typing.
    loaded_text: Option<String>,
    text: Option<Entity<TextareaState>>,
    new_slug: Option<Entity<TextInput>>,
    new_title: Option<Entity<TextInput>>,
    url: Option<Entity<TextInput>>,
}

impl PostsPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            list: Remote::default(),
            detail: Remote::default(),
            busy: Busy::default(),
            selected: None,
            loaded_text: None,
            text: None,
            new_slug: None,
            new_title: None,
            url: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.list,
            &repo,
            |shell| async move { shell.posts().await },
        );
        self.load_detail(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn load_detail(&mut self, cx: &mut Context<Self>) {
        let Some(slug) = self.selected.clone() else {
            return;
        };
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.detail,
            &repo,
            move |shell| async move { shell.post_detail(&slug).await },
        );
    }

    fn select(&mut self, slug: String, cx: &mut Context<Self>) {
        self.selected = Some(slug);
        self.detail = Remote::default();
        self.loaded_text = None;
        self.load_detail(cx);
    }

    fn act(
        &mut self,
        req: PostsWriteRequest,
        done: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("posts:{}:{}:{}", req.action, req.slug, req.index);
        let created = (req.action == "new").then(|| req.slug.clone());
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, done),
            move |shell| async move { shell.write_posts(&req).await },
            move |this, result, cx| {
                if result.is_some() {
                    if let Some(slug) = created {
                        this.selected = Some(slug);
                        this.loaded_text = None;
                    }
                }
                this.list = Remote::default();
                this.detail = Remote::default();
                this.load(cx);
            },
        );
    }

    fn request(&self, action: &str) -> PostsWriteRequest {
        PostsWriteRequest {
            action: action.into(),
            slug: self.selected.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text.is_none() {
            self.text = Some(cx.new(|cx| TextareaState::new(window, cx).auto_grow(12, 40)));
        }
        if self.new_slug.is_none() {
            self.new_slug =
                Some(cx.new(|cx| {
                    TextInput::new(window, cx).placeholder("slug, e.g. event-recap-2026")
                }));
            self.new_title = Some(cx.new(|cx| TextInput::new(window, cx).placeholder("Title")));
            self.url = Some(cx.new(|cx| TextInput::new(window, cx).placeholder("Published URL")));
        }
        if let (Some(detail), Some(text)) = (&self.detail.value, &self.text) {
            if self.loaded_text.as_deref() != Some(detail.summary.slug.as_str()) {
                let body = detail.text.clone();
                text.update(cx, |t, cx| t.set_value(body, window, cx));
                let url = detail.summary.url.clone().unwrap_or_default();
                if let Some(input) = &self.url {
                    input.update(cx, |i, cx| i.set_text(url, cx));
                }
                self.loaded_text = Some(detail.summary.slug.clone());
            }
        }
    }

    fn render_list(
        &self,
        view: &PostsView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = view.posts.iter().map(|post| {
            let slug = post.slug.clone();
            let label = format!("{} · {} · {} photos", post.title, post.state, post.media);
            Button::new(SharedString::from(format!("post-{slug}")), label)
                .size(ControlSize::Sm)
                .when(self.selected.as_deref() != Some(slug.as_str()), |b| {
                    b.variant(ButtonVariant::Ghost)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.select(slug.clone(), cx)))
        });
        let mut col = div().flex().flex_col().gap(px(4.)).children(rows);
        if view.posts.is_empty() {
            col = col.child(note("No posts yet. Create one below.", palette.muted));
        }
        col.into_any_element()
    }

    fn render_new(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(slug), Some(title)) = (&self.new_slug, &self.new_title) else {
            return div().into_any_element();
        };
        let (slug_in, title_in) = (slug.clone(), title.clone());
        div()
            .flex()
            .gap(px(8.))
            .child(div().w(px(220.)).child(Input::new(slug)))
            .child(div().flex_1().child(Input::new(title)))
            .child(
                Button::new("post-new", "New post")
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let req = PostsWriteRequest {
                            action: "new".into(),
                            slug: slug_in.read(cx).text().trim().to_string(),
                            title: title_in.read(cx).text().trim().to_string(),
                            ..Default::default()
                        };
                        this.act(req, "Post created", window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_detail(
        &self,
        detail: &PostDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = detail.summary.state.clone();
        let states = STATES.iter().map(|s| {
            let name = s.to_string();
            Button::new(SharedString::from(format!("state-{s}")), *s)
                .size(ControlSize::Sm)
                .when(state != *s, |b| b.variant(ButtonVariant::Ghost))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let url = this
                        .url
                        .as_ref()
                        .map(|u| u.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    let req = PostsWriteRequest {
                        state: name.clone(),
                        url,
                        ..this.request("state")
                    };
                    this.act(req, "State saved", window, cx);
                }))
        });
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(detail.summary.title.clone()),
            )
            .child(div().flex().gap(px(6.)).children(states));
        if let Some(url) = &self.url {
            col = col.child(div().w(px(420.)).child(Input::new(url)));
        }
        if let Some(text) = &self.text {
            let area = text.clone();
            col = col.child(Textarea::new(text)).child(
                div()
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        Button::new("post-save-text", "Save text")
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let req = PostsWriteRequest {
                                    text: area.read(cx).value().to_string(),
                                    ..this.request("text")
                                };
                                this.loaded_text = None;
                                this.act(req, "Text saved as a new revision", window, cx);
                            })),
                    )
                    .child(note(
                        format!("{} earlier revisions", detail.revisions),
                        palette.muted,
                    )),
            );
        }
        col.child(self.render_media(detail, palette, cx))
            .into_any_element()
    }

    fn render_media(
        &self,
        detail: &PostDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let out_dir = detail
            .media
            .iter()
            .find_map(|m| m.file.clone())
            .and_then(|f| PathBuf::from(f).parent().map(|p| p.display().to_string()));
        let mut toolbar = div()
            .flex()
            .gap(px(8.))
            .child(
                Button::new("post-add-selected", "Add selected from Photos")
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let req = this.request("add-selected");
                        this.act(req, "Added the Photos selection", window, cx);
                    })),
            )
            .child(
                Button::new("post-export", "Export for upload")
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let req = this.request("export");
                        this.act(req, "Exported in post order", window, cx);
                    })),
            );
        if let Some(dir) = out_dir {
            toolbar = toolbar.child(
                Button::new("post-reveal", "Show in Finder")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| cx.open_url(&format!("file://{dir}"))),
            );
        }
        let count = detail.media.len();
        let slots = detail
            .media
            .iter()
            .enumerate()
            .map(|(i, item)| self.render_slot(i, item, count, detail, palette, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(toolbar)
            .child(div().flex().flex_wrap().gap(px(12.)).children(slots))
            .into_any_element()
    }

    fn render_slot(
        &self,
        i: usize,
        item: &PostMediaItem,
        count: usize,
        detail: &PostDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let thumb: AnyElement = match &item.file {
            Some(file) => img(PathBuf::from(file))
                .object_fit(ObjectFit::Contain)
                .size_full()
                .into_any_element(),
            None => note("Not exported yet", palette.muted),
        };
        let button = |id: &str, label: &'static str| {
            Button::new(SharedString::from(format!("slot-{id}-{i}")), label)
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
        };
        let mut controls = div().flex().flex_wrap().gap(px(2.));
        if i > 0 {
            controls =
                controls.child(
                    button("up", "↑")
                        .on_click(slot_action(cx, i, "move", |r| r.direction = "up".into())),
                );
        }
        if i + 1 < count {
            controls =
                controls.child(
                    button("down", "↓")
                        .on_click(slot_action(cx, i, "move", |r| r.direction = "down".into())),
                );
        }
        controls = controls.child(button("remove", "Remove").on_click(slot_action(
            cx,
            i,
            "remove",
            |_| {},
        )));
        match &item.collage {
            Some(collage) => {
                let next = next_accent(&collage.accent).to_string();
                controls = controls
                    .child(button("split", "Split").on_click(slot_action(cx, i, "split", |_| {})))
                    .child(button("accent", "Accent").on_click(cx.listener(
                        move |this, _, window, cx| {
                            let req = PostsWriteRequest {
                                index: i,
                                accent: next.clone(),
                                ..this.request("accent")
                            };
                            this.act(req, "Accent changed", window, cx);
                        },
                    )));
            }
            None => {
                let next_is_photo = detail.media.get(i + 1).is_some_and(|n| n.collage.is_none());
                if next_is_photo {
                    controls = controls
                        .child(
                            button("pair", "Pair →")
                                .on_click(slot_action(cx, i, "pair", |r| r.layout = "pair".into())),
                        )
                        .child(button("stack", "Stack →").on_click(slot_action(
                            cx,
                            i,
                            "pair",
                            |r| r.layout = "stack".into(),
                        )));
                }
            }
        }
        div()
            .w(px(220.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .h(px(160.))
                    .w_full()
                    .bg(gpui_kit::rgb(palette.surface))
                    .child(thumb),
            )
            .child(div().text_xs().child(slot_label(i, item)))
            .child(controls)
            .into_any_element()
    }
}

/// A button's click callback.
type ClickHandler = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static>;

/// A click handler that sends one media action for slot `i`.
fn slot_action(
    cx: &mut Context<PostsPage>,
    i: usize,
    name: &'static str,
    extra: fn(&mut PostsWriteRequest),
) -> ClickHandler {
    Box::new(cx.listener(move |this: &mut PostsPage, _, window, cx| {
        let mut req = PostsWriteRequest {
            index: i,
            ..this.request(name)
        };
        extra(&mut req);
        this.act(req, "Media updated", window, cx);
    }))
}

impl Render for PostsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        self.ensure_inputs(window, cx);
        let Some(view) = self.list.value.clone() else {
            let body = match &self.list.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Posts", body, palette).into_any_element();
        };
        let detail: AnyElement = match (&self.selected, &self.detail.value, &self.detail.error) {
            (None, _, _) => note("Select a post, or create one.", palette.muted),
            (Some(_), Some(detail), _) => self.render_detail(detail, palette, cx),
            (Some(_), None, Some(error)) => note(error.to_string(), palette.red),
            (Some(_), None, None) => placeholder(320.),
        };
        div()
            .id("posts")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(16.))
            .child(self.render_new(cx))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .w(px(280.))
                            .child(self.render_list(&view, palette, cx)),
                    )
                    .child(div().flex_1().child(detail)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_starts_with_nothing_selected() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = PostsPage::new(repo);
        assert!(page.selected.is_none());
        assert!(page.list.value.is_none());
    }
}
