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
            )
            .child(
                Button::new(
                    "people-network-connected-only",
                    if self.network_connected_only {
                        "Connected only: on"
                    } else {
                        "Connected only"
                    },
                )
                .size(ControlSize::Sm)
                .variant(if self.network_connected_only {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(cx.listener(|page, _, _, cx| {
                    page.network_connected_only = !page.network_connected_only;
                    cx.notify();
                })),
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
                let connected_only = self.network_connected_only;
                let bounds = self.network_bounds.clone();
                let paint_graph = graph.clone();
                let paint_bounds = bounds.clone();
                let canvas = gpui_kit::canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        if paint_bounds.replace(Some(bounds)) != Some(bounds) {
                            window.refresh();
                        }
                        paint(&paint_graph, selected, connected_only, palette, bounds, window);
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
        let mut ego_graph: Option<AnyElement> = None;
        let mut link_summary = String::new();
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
            let direct_count = graph.links_of(i).count();
            let second_deg = graph.second_degree(i);
            let group = graph.nodes[i].group.map(|g| &graph.groups[g]);
            let coworkers_count = group.map_or(0, |g| g.count.saturating_sub(1));
            if direct_count > 0 || coworkers_count > 0 || !second_deg.is_empty() {
                let mut parts = Vec::new();
                if direct_count > 0 {
                    parts.push(format!("{direct_count} direct"));
                }
                if !second_deg.is_empty() {
                    parts.push(format!("{} 2nd-degree", second_deg.len()));
                }
                if coworkers_count > 0 {
                    parts.push(format!("{coworkers_count} coworkers"));
                }
                link_summary = parts.join(" · ");
                if self.show_ego_graph {
                    ego_graph = Some(self.render_ego_graph(graph, i, palette, cx));
                }
            }
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
        let can_introduce = self.writer().is_some() && self.derived.parsed.len() > 1;
        div()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("NETWORK & GRAPH"),
                            )
                            .when(!link_summary.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("· {link_summary}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(ego_graph.is_some() || !link_summary.is_empty(), |el| {
                                el.child(
                                    Button::new(
                                        "people-toggle-ego-graph",
                                        if self.show_ego_graph {
                                            "Hide graph"
                                        } else {
                                            "Show graph"
                                        },
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.show_ego_graph = !page.show_ego_graph;
                                        cx.notify();
                                    })),
                                )
                            })
                            .when(can_introduce, |el| {
                                el.child(
                                    Button::new("people-add-introduction", "+ Introduction")
                                        .size(ControlSize::Sm)
                                        .variant(ButtonVariant::Ghost)
                                        .on_click(cx.listener(move |page, _, window, cx| {
                                            page.open_form(
                                                FormKind::Introduction(to.clone()),
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                            }),
                    ),
            )
            .children(ego_graph)
            .child(rows)
            .into_any_element()
    }

    /// Radial 1-hop + coworker + 2nd-degree ego-graph centered on node `center_ix`.
    fn render_ego_graph(
        &self,
        graph: &Layout,
        center_ix: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let center_name = graph.nodes[center_ix].name.clone();
        let mut neighbors: Vec<(usize, String, bool)> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(center_ix);
        for (other, edge) in graph.links_of(center_ix) {
            if seen.insert(other) {
                let kind = if edge.kind == "introduced" {
                    if edge.from == center_ix {
                        "introduced"
                    } else {
                        "intro by"
                    }
                } else {
                    "chat"
                };
                neighbors.push((other, kind.to_owned(), true));
            }
        }
        if let Some(group_ix) = graph.nodes[center_ix].group {
            for (ix, node) in graph.nodes.iter().enumerate() {
                if node.group == Some(group_ix) && !node.is_self && seen.insert(ix) && neighbors.len() < 10 {
                    neighbors.push((ix, "coworker".to_owned(), false));
                }
            }
        }
        let second_deg: Vec<(usize, usize)> = graph
            .second_degree(center_ix)
            .into_iter()
            .filter(|(_, other)| seen.insert(*other))
            .take(6)
            .collect();

        let width: f32 = 520.0;
        let height: f32 = 220.0;
        let cx_px = width / 2.0;
        let cy_px = height / 2.0;
        let inner_rx: f32 = 148.0;
        let inner_ry: f32 = 68.0;
        let outer_rx: f32 = 215.0;
        let outer_ry: f32 = 94.0;

        let mut node_coords: Vec<(usize, f32, f32, bool, String, bool)> = Vec::new();
        let n_inner = neighbors.len().max(1);
        for (idx, (node_ix, rel, is_direct_edge)) in neighbors.iter().enumerate() {
            let angle = -std::f32::consts::FRAC_PI_2
                + (idx as f32 / n_inner as f32) * std::f32::consts::TAU;
            let x = cx_px + inner_rx * angle.cos();
            let y = cy_px + inner_ry * angle.sin();
            node_coords.push((*node_ix, x, y, *is_direct_edge, rel.clone(), true));
        }

        let mut outer_coords: Vec<(usize, usize, f32, f32)> = Vec::new();
        let n_outer = second_deg.len().max(1);
        for (idx, (via, other)) in second_deg.iter().enumerate() {
            let angle = -std::f32::consts::FRAC_PI_2
                + 0.35
                + (idx as f32 / n_outer as f32) * std::f32::consts::TAU;
            let x = cx_px + outer_rx * angle.cos();
            let y = cy_px + outer_ry * angle.sin();
            outer_coords.push((*via, *other, x, y));
        }

        let paint_inner = node_coords
            .iter()
            .map(|(ix, x, y, direct, _, _)| (*ix, *x, *y, *direct))
            .collect::<Vec<_>>();
        let paint_outer = outer_coords.clone();
        let canvas = gpui_kit::canvas(
            move |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                let ox = f32::from(bounds.origin.x);
                let oy = f32::from(bounds.origin.y);
                let sx = f32::from(bounds.size.width) / width;
                let sy = f32::from(bounds.size.height) / height;
                let scale_pt = |x: f32, y: f32| (ox + x * sx, oy + y * sy);
                let (cx_w, cy_w) = scale_pt(cx_px, cy_px);

                for (ix, x, y, is_direct) in &paint_inner {
                    let (nx, ny) = scale_pt(*x, *y);
                    let mut line = gpui_kit::PathBuilder::stroke(px(if *is_direct { 1.8 } else { 1.0 }));
                    line.move_to(point(px(cx_w), px(cy_w)));
                    line.line_to(point(px(nx), px(ny)));
                    if let Ok(path) = line.build() {
                        let color = if *is_direct {
                            tint(palette.accent, 0xcc)
                        } else {
                            tint(palette.muted, 0x66)
                        };
                        window.paint_path(path, rgba(color));
                    }
                    for (via, _, ox2, oy2) in &paint_outer {
                        if via == ix {
                            let (vx, vy) = scale_pt(*ox2, *oy2);
                            let mut l2 = gpui_kit::PathBuilder::stroke(px(1.0));
                            l2.move_to(point(px(nx), px(ny)));
                            l2.line_to(point(px(vx), px(vy)));
                            if let Ok(p2) = l2.build() {
                                window.paint_path(p2, rgba(tint(palette.amber, 0x88)));
                            }
                        }
                    }
                }
            },
        )
        .absolute()
        .size_full();

        let mut pills: Vec<AnyElement> = Vec::new();
        // Center person pill
        pills.push(
            div()
                .absolute()
                .left(px(cx_px - 56.0))
                .top(px(cy_px - 13.0))
                .w(px(112.0))
                .px_2()
                .py_1()
                .rounded_full()
                .bg(rgb(palette.accent))
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(rgb(0xffffff))
                .truncate()
                .flex()
                .justify_center()
                .child(center_name)
                .into_any_element(),
        );

        for (node_ix, x, y, is_direct, rel, _) in node_coords {
            let node = &graph.nodes[node_ix];
            let target_id = node.id.clone();
            let clickable = !node.is_self;
            let label = format!("{} · {rel}", node.name);
            pills.push(
                div()
                    .id(SharedString::from(format!("ego-node-{node_ix}")))
                    .absolute()
                    .left(px((x - 58.0).clamp(4.0, width - 120.0)))
                    .top(px((y - 11.0).clamp(4.0, height - 26.0)))
                    .max_w(px(118.0))
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(if is_direct {
                        tint(palette.accent, 0x88)
                    } else {
                        palette.border
                    }))
                    .bg(rgba(palette.raised))
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .truncate()
                    .when(clickable, |el| {
                        el.cursor_pointer()
                            .hover(move |s| s.border_color(rgb(palette.accent)))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.select_person(Some(target_id.clone()), cx);
                            }))
                    })
                    .child(label)
                    .into_any_element(),
            );
        }

        for (_, other_ix, x, y) in outer_coords {
            let node = &graph.nodes[other_ix];
            let target_id = node.id.clone();
            pills.push(
                div()
                    .id(SharedString::from(format!("ego-outer-{other_ix}")))
                    .absolute()
                    .left(px((x - 48.0).clamp(4.0, width - 102.0)))
                    .top(px((y - 10.0).clamp(4.0, height - 24.0)))
                    .max_w(px(98.0))
                    .px_1p5()
                    .py(px(1.0))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(tint(palette.amber, 0x66)))
                    .bg(rgba(palette.surface))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .truncate()
                    .cursor_pointer()
                    .hover(move |s| s.text_color(rgb(palette.text)))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.select_person(Some(target_id.clone()), cx);
                    }))
                    .child(node.name.clone())
                    .into_any_element(),
            );
        }

        div()
            .w_full()
            .h(px(height))
            .rounded(px(6.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(tint(palette.raised, 0x55)))
            .relative()
            .overflow_hidden()
            .child(canvas)
            .children(pills)
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
    connected_only: bool,
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
        let has_edge = graph.links_of(i).next().is_some();
        if connected_only && !node.is_self && !has_edge && Some(i) != selected {
            continue;
        }
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
    let mut neighbor_set = std::collections::BTreeSet::new();
    if let Some(sel) = selected {
        for (other, _) in graph.links_of(sel).take(12) {
            neighbor_set.insert(other);
        }
    }
    for (index, node) in graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.is_self || Some(*index) == selected || neighbor_set.contains(index)
        })
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
