//! The people graph as the screen draws it: a deterministic layout of `/entities/api/graph`.
//!
//! The operator sits at the centre. People who share a current company form a circle labelled
//! with it; a shared company is a coincidence (`docs/capabilities/links.md`), so the circle is a
//! grouping, not a line. Exact links (`introduced`, `conversation`) are the lines. Everyone else is
//! placed around the groups. Coordinates are unit space, centred on 0, scaled to fit radius 1.

use std::collections::{BTreeMap, BTreeSet};

pub use entities_api::{EntityEdge, GraphNode, PeopleGraph};

/// The angle between two successive spiral positions, which spreads points without rings.
const GOLDEN: f32 = 2.399_963;
/// A node's radius before the layout is scaled to fit.
const NODE: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedNode {
    pub id: String,
    pub name: String,
    pub is_self: bool,
    /// Index into `Layout::groups`, when the node belongs to a company group.
    pub group: Option<usize>,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub label: String,
    pub count: usize,
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEdge {
    pub from: usize,
    pub to: usize,
    pub kind: String,
    pub directed: bool,
    pub source: String,
    pub at: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub nodes: Vec<PlacedNode>,
    pub groups: Vec<Group>,
    pub edges: Vec<PlacedEdge>,
    /// A node's drawn radius in unit space.
    pub node_r: f32,
}

/// A company name as one key: trimmed, case folded, inner spaces collapsed.
pub fn company_key(company: &str) -> String {
    company
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Places circles of radius `r` one after another on an outward spiral, each at the first spot
/// that overlaps no earlier circle.
struct Packer {
    placed: Vec<(f32, f32, f32)>,
    gap: f32,
    next_single: usize,
}

impl Packer {
    fn place(&mut self, r: f32) -> (f32, f32) {
        let free = |x: f32, y: f32, placed: &[(f32, f32, f32)]| {
            placed.iter().all(|(px, py, pr)| {
                let (dx, dy) = (x - px, y - py);
                dx * dx + dy * dy >= (r + pr + self.gap).powi(2)
            })
        };
        let mut step = if r == NODE { self.next_single } else { 0 };
        loop {
            let t = step as f32;
            let radius = 0.9 * t.sqrt() * (NODE + self.gap);
            let (x, y) = (radius * (t * GOLDEN).cos(), radius * (t * GOLDEN).sin());
            if free(x, y, &self.placed) {
                self.placed.push((x, y, r));
                if r == NODE {
                    self.next_single = step + 1;
                }
                return (x, y);
            }
            step += 1;
        }
    }
}

/// The layout of `graph`. The same graph always gives the same layout.
pub fn layout(graph: &PeopleGraph) -> Layout {
    let mut members: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        if node.kind == "self" {
            continue;
        }
        if let Some(company) = node.company.as_deref().filter(|c| !c.trim().is_empty()) {
            members
                .entry(company_key(company))
                .or_insert_with(|| (company.trim().to_string(), Vec::new()))
                .1
                .push(i);
        }
    }
    let mut groups: Vec<(String, Vec<usize>)> = members
        .into_values()
        .filter(|(_, m)| m.len() >= 2)
        .collect();
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));

    let mut position: Vec<Option<(f32, f32, Option<usize>)>> = vec![None; graph.nodes.len()];
    let mut packer = Packer {
        placed: Vec::new(),
        gap: NODE * 0.6,
        next_single: 0,
    };
    // The operator first, so the spiral starts at the centre.
    let centre_r = NODE * 3.0;
    packer.place(centre_r);
    for (i, node) in graph.nodes.iter().enumerate() {
        if node.kind == "self" {
            position[i] = Some((0.0, 0.0, None));
        }
    }
    let mut placed_groups = Vec::new();
    for (g, (label, ids)) in groups.iter().enumerate() {
        let n = ids.len() as f32;
        let r = NODE * (1.9 * n.sqrt() + 1.2);
        let (cx, cy) = packer.place(r);
        let inner = r - NODE * 1.4;
        for (j, &i) in ids.iter().enumerate() {
            let t = j as f32 + 0.5;
            let d = inner * (t / n).sqrt();
            position[i] = Some((
                cx + d * (t * GOLDEN).cos(),
                cy + d * (t * GOLDEN).sin(),
                Some(g),
            ));
        }
        placed_groups.push(Group {
            label: label.clone(),
            count: ids.len(),
            x: cx,
            y: cy,
            r,
        });
    }
    for slot in position.iter_mut().filter(|p| p.is_none()) {
        let (x, y) = packer.place(NODE);
        *slot = Some((x, y, None));
    }

    let extent = packer
        .placed
        .iter()
        .map(|(x, y, r)| (x * x + y * y).sqrt() + r)
        .fold(centre_r, f32::max);
    let scale = 1.0 / extent;
    let nodes = graph
        .nodes
        .iter()
        .zip(position)
        .map(|(node, p)| {
            let (x, y, group) = p.unwrap_or_default();
            PlacedNode {
                id: node.id.clone(),
                name: node.name.clone(),
                is_self: node.kind == "self",
                group,
                x: x * scale,
                y: y * scale,
            }
        })
        .collect();
    for group in &mut placed_groups {
        group.x *= scale;
        group.y *= scale;
        group.r *= scale;
    }
    let index: BTreeMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let edges = graph
        .edges
        .iter()
        .filter_map(|e| {
            Some(PlacedEdge {
                from: *index.get(e.a.as_str())?,
                to: *index.get(e.b.as_str())?,
                kind: e.kind.clone(),
                directed: entities_api::edge_is_directed(&e.kind),
                source: e.source.clone(),
                at: e.at.clone(),
                note: e.note.clone(),
            })
        })
        .collect();
    Layout {
        nodes,
        groups: placed_groups,
        edges,
        node_r: NODE * scale,
    }
}

impl Layout {
    /// The node under unit point (`x`, `y`), within `slack` node radii.
    pub fn hit(&self, x: f32, y: f32, slack: f32) -> Option<usize> {
        let reach = self.node_r * slack;
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i, (n.x - x).hypot(n.y - y)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The links of node `i`, paired with the other endpoint.
    pub fn links_of(&self, i: usize) -> impl Iterator<Item = (usize, &PlacedEdge)> {
        self.edges.iter().filter_map(move |edge| {
            if edge.from == i {
                Some((edge.to, edge))
            } else if edge.to == i {
                Some((edge.from, edge))
            } else {
                None
            }
        })
    }

    /// One two-edge path per non-adjacent person, as `(via, destination)`. A path
    /// describes reachability, not a new link or an inferred introduction.
    pub fn second_degree(&self, i: usize) -> Vec<(usize, usize)> {
        let neighbors: BTreeSet<_> = self.links_of(i).map(|(other, _)| other).collect();
        let mut via_by_other = BTreeMap::new();
        for edge in &self.edges {
            for (via, other) in [(edge.from, edge.to), (edge.to, edge.from)] {
                if neighbors.contains(&via)
                    && other != i
                    && !neighbors.contains(&other)
                    && !self.nodes[other].is_self
                {
                    via_by_other.entry(other).or_insert(via);
                }
            }
        }
        via_by_other
            .into_iter()
            .map(|(other, via)| (via, other))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str, company: Option<&str>) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: kind.into(),
            name: id.to_uppercase(),
            company: company.map(str::to_string),
        }
    }

    fn graph() -> PeopleGraph {
        PeopleGraph {
            nodes: vec![
                node("me", "self", Some("Acme")),
                node("a", "person", Some("Acme")),
                node("b", "person", Some(" acme ")),
                node("c", "person", Some("Globex")),
                node("d", "person", None),
            ],
            edges: vec![
                EntityEdge {
                    a: "c".into(),
                    b: "a".into(),
                    kind: "introduced".into(),
                    source: "operator".into(),
                    at: None,
                    note: None,
                },
                EntityEdge {
                    a: "a".into(),
                    b: "gone".into(),
                    kind: "conversation".into(),
                    source: "linkedin".into(),
                    at: None,
                    note: None,
                },
            ],
        }
    }

    #[test]
    fn a_shared_company_groups_people_and_the_operator_is_the_centre() {
        let l = layout(&graph());
        assert_eq!(l.groups.len(), 1);
        assert_eq!((l.groups[0].label.as_str(), l.groups[0].count), ("Acme", 2));
        assert_eq!(l.nodes[1].group, Some(0));
        assert_eq!(l.nodes[2].group, Some(0));
        assert_eq!(l.nodes[3].group, None, "a company of one is no group");
        assert_eq!((l.nodes[0].x, l.nodes[0].y), (0.0, 0.0));
        assert!(l.nodes.iter().all(|n| n.x.hypot(n.y) <= 1.0));
    }

    #[test]
    fn nodes_do_not_overlap_and_the_layout_repeats() {
        let l = layout(&graph());
        for (i, a) in l.nodes.iter().enumerate() {
            for b in &l.nodes[i + 1..] {
                assert!(
                    (a.x - b.x).hypot(a.y - b.y) >= l.node_r * 1.5,
                    "{} and {}",
                    a.id,
                    b.id
                );
            }
        }
        assert_eq!(l, layout(&graph()));
    }

    #[test]
    fn five_hundred_people_fit_without_overlap() {
        let nodes = (0..500)
            .map(|i| {
                let company = (i % 3 != 0).then(|| format!("Company {}", i % 70));
                node(&format!("p{i}"), "person", company.as_deref())
            })
            .collect();
        let started = std::time::Instant::now();
        let l = layout(&PeopleGraph {
            nodes,
            edges: Vec::new(),
        });
        let took = started.elapsed();
        assert!(took.as_millis() < 2000, "layout took {took:?}");
        assert!(l.nodes.iter().all(|n| n.x.hypot(n.y) <= 1.0 + 1e-4));
        for (i, a) in l.nodes.iter().enumerate() {
            for b in &l.nodes[i + 1..] {
                assert!((a.x - b.x).hypot(a.y - b.y) >= l.node_r * 1.5);
            }
        }
    }

    #[test]
    fn second_degree_uses_two_exact_links_and_excludes_direct_contacts() {
        let mut graph = graph();
        graph.edges.push(EntityEdge {
            a: "c".into(),
            b: "d".into(),
            kind: "conversation".into(),
            source: "linkedin".into(),
            at: Some("2026-11-18".into()),
            note: None,
        });
        let l = layout(&graph);
        assert_eq!(l.second_degree(1), vec![(3, 4)]);
        assert_eq!(l.edges[1].at.as_deref(), Some("2026-11-18"));
        assert!(!l.edges[1].directed);
        assert!(
            l.second_degree(2).is_empty(),
            "a coworker has no implied links"
        );
        assert!(l.second_degree(3).is_empty(), "both neighbors are direct");
    }

    #[test]
    fn an_edge_to_a_missing_node_is_dropped_and_direction_is_kept() {
        let l = layout(&graph());
        assert_eq!(l.edges.len(), 1);
        assert_eq!(
            (l.edges[0].from, l.edges[0].to, l.edges[0].directed),
            (3, 1, true)
        );
        let (other, edge) = l.links_of(1).next().unwrap();
        assert_eq!(
            (other, edge.kind.as_str(), edge.source.as_str()),
            (3, "introduced", "operator")
        );
        assert!(
            l.second_degree(1).is_empty(),
            "company groups are not edges"
        );
        let (x, y) = (l.nodes[3].x, l.nodes[3].y);
        assert_eq!(l.hit(x, y, 1.5), Some(3));
        assert_eq!(l.hit(5.0, 5.0, 1.5), None);
    }
}
