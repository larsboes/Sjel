//! Network canvas and the selected contact's sourced links.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, Bounds, Context, Corners, Edges, Entity,
    MouseButton, MouseDownEvent, Pixels, SharedString,
};
use sjel_gpui_core::people::graph::{layout, Layout};
use sjel_gpui_core::shell::{self, ShellError};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Input, TextInput};
use crate::ely::theme::ControlSize;
use crate::theme::{tint, Palette};

use super::forms::{FormKind, Work};
use super::PeoplePage;

const LABEL_MIN_PX: f32 = 46.0;

impl PeoplePage {
    pub(super) fn load_graph(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.graph.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.graph.loading = true;
        self.graph_request = self.graph_request.wrapping_add(1);
        let request = self.graph_request;
        let work = shell::run(async move {
            let graph = shell.people_graph().await?;
            tokio::task::spawn_blocking(move || layout(&graph))
                .await
                .map_err(|_| ShellError::Unreachable)
        });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if page.graph_request == request {
                    page.graph.settle(res.map(Rc::new));
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_network(
        &self,
        search: &Entity<TextInput>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let help = |text: String| div().text_xs().text_color(rgb(palette.muted)).child(text);
        let found = self.derived.filtered_indices.len();
        let toolbar = div()
            .flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(search).size(ControlSize::Sm)),
            )
            .when(!self.search_query.is_empty(), |bar| {
                bar.child(help(format!("{found} matches")))
            })
            .child(
                Button::new("people-network-previous", "Previous")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(found == 0)
                    .on_click(cx.listener(|page, _, _, cx| page.step_selection(false, cx))),
            )
            .child(
                Button::new("people-network-next", "Next")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(found == 0)
                    .on_click(cx.listener(|page, _, _, cx| page.step_selection(true, cx))),
            );

        let frame = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .overflow_hidden();
        let graph = self.graph.value.as_ref();
        let legend = graph.map(|graph| {
            let contacts = graph.nodes.iter().filter(|n| !n.is_self).count();
            format!(
                "{contacts} contacts · {} company groups · {} links. Circles group coworkers; arrows are introductions; plain lines are group conversations.",
                graph.groups.len(),
                graph.edges.len(),
            )
        });
        let status = if let Some(error) = &self.graph.error {
            Some(if graph.is_some() {
                format!("Showing the previous graph. Refresh failed: {error}")
            } else {
                format!("Could not read the people graph: {error}")
            })
        } else if self.graph.loading || graph.is_none() {
            Some("Loading the people graph…".to_owned())
        } else if graph.is_some_and(|g| g.nodes.iter().all(|n| n.is_self)) {
            Some("No contacts yet. Add a person to start the network.".to_owned())
        } else {
            None
        };
        let surface = match graph {
            Some(graph) if graph.nodes.iter().any(|n| !n.is_self) => {
                let selected = self
                    .selected_id
                    .as_deref()
                    .and_then(|id| graph.nodes.iter().position(|n| n.id == id));
                let bounds = self.network_bounds.clone();
                let paint_graph = graph.clone();
                let paint_bounds = bounds.clone();
                let canvas = gpui_kit::canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        if paint_bounds.replace(Some(bounds)) != Some(bounds) {
                            window.refresh();
                        }
                        paint(&paint_graph, selected, palette, bounds, window);
                    },
                )
                .absolute()
                .size_full();

                let click_graph = graph.clone();
                let click_bounds = bounds.clone();
                frame
                    .id("people-network-canvas")
                    .relative()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |page, event: &MouseDownEvent, window, cx| {
                            let Some(bounds) = click_bounds.get() else {
                                return;
                            };
                            let Some((x, y)) = to_unit(&bounds, event.position.x, event.position.y)
                            else {
                                return;
                            };
                            window.focus(&page.focus, cx);
                            let hit = click_graph.hit(x, y, 2.0);
                            if !hit.is_some_and(|i| click_graph.nodes[i].is_self) {
                                page.select_person(
                                    hit.map(|i| click_graph.nodes[i].id.clone()),
                                    cx,
                                );
                            }
                        }),
                    )
                    .child(canvas)
                    .children(labels(graph, selected, &bounds, palette))
                    .into_any_element()
            }
            _ => frame.into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(toolbar)
            .when_some(status, |el, message| el.child(help(message)))
            .child(surface)
            .when_some(legend, |el, legend| el.child(help(legend)))
            .into_any_element()
    }

    pub(super) fn render_links(
        &self,
        id: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows = div().flex().flex_col().gap_2();
        let text = |label: String| div().text_sm().text_color(rgb(palette.muted)).child(label);
        if let Some(error) = &self.graph.error {
            rows = rows.child(text(format!("Could not refresh links: {error}")));
        }
        if let Some((graph, i)) = self.graph.value.as_ref().and_then(|graph| {
            graph
                .nodes
                .iter()
                .position(|n| n.id == id)
                .map(|i| (graph, i))
        }) {
            let group = graph.nodes[i].group.map(|g| &graph.groups[g]);
            if let Some(group) = group {
                rows = rows.child(text(format!(
                    "Shares {} with {} others",
                    group.label,
                    group.count - 1
                )));
            }
            for (index, (other, edge)) in graph.links_of(i).enumerate() {
                let node = &graph.nodes[other];
                let label = match (edge.kind.as_str(), edge.from == i) {
                    ("introduced", true) => format!("Introduced {}", node.name),
                    ("introduced", false) => format!("Introduced by {}", node.name),
                    _ => format!("Group conversation with {}", node.name),
                };
                let source = match edge.source.as_str() {
                    "operator" => "Added by you",
                    "linkedin" => "LinkedIn",
                    source => source,
                };
                let evidence = [Some(source), edge.at.as_deref(), edge.note.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                let target = node.id.clone();
                let mut link = div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(
                        Button::new(SharedString::from(format!("people-link-{index}")), label)
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(node.is_self)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.select_person(Some(target.clone()), cx);
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(evidence),
                    );
                if edge.kind == "introduced"
                    && edge.source == "operator"
                    && edge.to == i
                    && self.writer().is_some()
                    && self.graph.error.is_none()
                {
                    let from = graph.nodes[edge.from].id.clone();
                    let to = graph.nodes[edge.to].id.clone();
                    let armed = self
                        .confirm_unlink
                        .as_ref()
                        .is_some_and(|(a, b)| a == &from && b == &to);
                    link = link.child(
                        Button::new(
                            SharedString::from(format!("people-unlink-{index}")),
                            if armed {
                                "Confirm removal"
                            } else {
                                "Remove introduction"
                            },
                        )
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                if armed {
                                    page.confirm_unlink = None;
                                    if let Some(shell) = page.writer() {
                                        page.write(
                                            shell,
                                            Work::UnlinkIntroduction {
                                                from: from.clone(),
                                                to: to.clone(),
                                            },
                                            window,
                                            cx,
                                        );
                                    }
                                } else {
                                    page.confirm_unlink = Some((from.clone(), to.clone()));
                                }
                                cx.notify();
                            },
                        )),
                    );
                }
                rows = rows.child(link);
            }
            for (via, other) in graph.second_degree(i) {
                let target = graph.nodes[other].id.clone();
                rows = rows.child(
                    Button::new(
                        SharedString::from(format!("people-second-degree-{other}")),
                        format!("{} via {}", graph.nodes[other].name, graph.nodes[via].name),
                    )
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.select_person(Some(target.clone()), cx);
                    })),
                );
            }
            if graph.links_of(i).next().is_none() && group.is_none() {
                rows = rows.child(text("No links yet.".to_owned()));
            }
        } else if self.graph.loading {
            rows = rows.child(text("Loading links…".to_owned()));
        }
        let to = id.to_owned();
        if self.writer().is_some() && self.derived.parsed.len() > 1 {
            rows = rows.child(
                Button::new("people-add-introduction", "Record introduction")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(move |page, _, window, cx| {
                        page.open_form(FormKind::Introduction(to.clone()), window, cx);
                    })),
            );
        }
        div()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("NETWORK"),
            )
            .child(rows)
            .into_any_element()
    }
}

/// The square that unit space maps onto: centred in `bounds`, with a margin.
pub(super) fn frame_of(bounds: &Bounds<Pixels>) -> (f32, f32, f32) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let cx = f32::from(bounds.origin.x) + w / 2.;
    let cy = f32::from(bounds.origin.y) + h / 2.;
    (cx, cy, w.min(h) * 0.46)
}

fn to_unit(bounds: &Bounds<Pixels>, x: Pixels, y: Pixels) -> Option<(f32, f32)> {
    let (cx, cy, side) = frame_of(bounds);
    (side >= 8.).then(|| ((f32::from(x) - cx) / side, (f32::from(y) - cy) / side))
}

fn circle(window: &mut gpui_kit::Window, x: f32, y: f32, r: f32, fill: u32, border: Option<u32>) {
    let bounds = Bounds::new(
        point(px(x - r), px(y - r)),
        gpui_kit::size(px(2. * r), px(2. * r)),
    );
    window.paint_quad(gpui_kit::PaintQuad {
        bounds,
        corner_radii: Corners::all(px(r)),
        background: rgba(fill).into(),
        border_widths: Edges::all(px(if border.is_some() { 1. } else { 0. })),
        border_color: rgba(border.unwrap_or(0)).into(),
        border_style: gpui_kit::BorderStyle::default(),
    });
}

fn paint(
    graph: &Layout,
    selected: Option<usize>,
    palette: Palette,
    bounds: Bounds<Pixels>,
    window: &mut gpui_kit::Window,
) {
    let (cx, cy, side) = frame_of(&bounds);
    if side < 8. {
        return;
    }
    let at = |x: f32, y: f32| (cx + x * side, cy + y * side);
    for group in &graph.groups {
        let (x, y) = at(group.x, group.y);
        circle(
            window,
            x,
            y,
            group.r * side,
            tint(palette.accent, 0x10),
            Some(tint(palette.accent, 0x40)),
        );
    }
    let mut linked = vec![false; graph.nodes.len()];
    if let Some(selected) = selected {
        for (other, _) in graph.links_of(selected) {
            linked[other] = true;
        }
    }
    let r = (graph.node_r * side).clamp(1.5, 7.0);
    for edge in &graph.edges {
        let (a, b) = (&graph.nodes[edge.from], &graph.nodes[edge.to]);
        let ((x1, y1), (x2, y2)) = (at(a.x, a.y), at(b.x, b.y));
        let hot = selected.is_some_and(|s| s == edge.from || s == edge.to);
        let colour = match (edge.directed, hot) {
            (_, true) => tint(palette.accent, 0xee),
            (true, false) => tint(palette.amber, 0xaa),
            (false, false) => tint(palette.muted, 0x88),
        };
        let mut line = gpui_kit::PathBuilder::stroke(px(if hot { 2.0 } else { 1.2 }));
        line.move_to(point(px(x1), px(y1)));
        if edge.directed {
            let (dx, dy) = (x2 - x1, y2 - y1);
            let length = dx.hypot(dy);
            let gap = if b.is_self || Some(edge.to) == selected {
                2.3 * r
            } else {
                1.5 * r
            };
            if length > gap + 4. {
                let (ux, uy) = (dx / length, dy / length);
                let (x, y) = (x2 - gap * ux, y2 - gap * uy);
                line.line_to(point(px(x), px(y)));
                line.move_to(point(px(x - 4. * ux - 3. * uy), px(y - 4. * uy + 3. * ux)));
                line.line_to(point(px(x), px(y)));
                line.line_to(point(px(x - 4. * ux + 3. * uy), px(y - 4. * uy - 3. * ux)));
            } else {
                line.line_to(point(px(x2), px(y2)));
            }
        } else {
            line.line_to(point(px(x2), px(y2)));
        }
        if let Ok(path) = line.build() {
            window.paint_path(path, rgba(colour));
        }
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        let (x, y) = at(node.x, node.y);
        if node.is_self {
            circle(window, x, y, r * 2.2, tint(palette.accent, 0xff), None);
        } else if Some(i) == selected {
            circle(
                window,
                x,
                y,
                r * 1.8,
                tint(palette.accent, 0xff),
                Some(tint(palette.text, 0xff)),
            );
        } else if linked[i] {
            circle(window, x, y, r * 1.4, tint(palette.amber, 0xff), None);
        } else {
            let fill = if node.group.is_some() {
                palette.text
            } else {
                palette.muted
            };
            circle(window, x, y, r, tint(fill, 0xb0), None);
        }
    }
}

/// Company labels for circles large enough to hold one, and the selected person's name.
fn labels(
    graph: &Layout,
    selected: Option<usize>,
    bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
    palette: Palette,
) -> Vec<AnyElement> {
    // The first frame has no bounds yet; the canvas records them and asks for the next frame.
    let Some(b) = bounds.get() else {
        return Vec::new();
    };
    let (cx, cy, side) = frame_of(&b);
    let (ox, oy) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let mut out = Vec::new();
    for (i, group) in graph.groups.iter().enumerate() {
        let width = 2. * group.r * side;
        if width < LABEL_MIN_PX {
            continue;
        }
        let (x, y) = (
            cx + group.x * side - ox,
            cy + (group.y - group.r) * side - oy,
        );
        out.push(
            div()
                .id(SharedString::from(format!("people-network-group-{i}")))
                .absolute()
                .left(px(x - width / 2.))
                .top(px(y - 14.))
                .w(px(width))
                .flex()
                .justify_center()
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} · {}", group.label, group.count)),
                )
                .into_any_element(),
        );
    }
    for (index, node) in graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| node.is_self || Some(*index) == selected)
    {
        let (x, y) = (cx + node.x * side - ox, cy + node.y * side - oy);
        out.push(
            div()
                .absolute()
                .id(SharedString::from(format!("people-network-label-{index}")))
                .left(px((x + 10.).min((f32::from(b.size.width) - 140.).max(0.))))
                .top(px((y - 9.).max(0.)))
                .max_w(px(132.))
                .truncate()
                .px_1p5()
                .rounded(px(4.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.text))
                .child(if node.is_self {
                    "You".to_owned()
                } else {
                    node.name.clone()
                })
                .into_any_element(),
        );
    }
    out
}
