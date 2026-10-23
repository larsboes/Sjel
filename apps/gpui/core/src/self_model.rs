//! Data models and pure reductions for the Self-model (`/self`).
//!
//! Visualizes Sjel's architectural units (capabilities, libraries, packs, spines),
//! compile-time couplings, runtime dependencies, code sizes, and live service status.

use std::collections::{BTreeMap, HashSet};
use serde::{Deserialize, Serialize};

/// Code stats for an architectural unit.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfCode {
    #[serde(default)]
    pub files: usize,
    #[serde(default)]
    pub nodes: usize,
}

/// Service definition if the unit is a process or container.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfService {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub image: Option<String>,
}

/// An architectural unit in Sjel.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfUnit {
    pub name: String,
    /// `capability`, `lib`, `spine`, `pack`, `unknown`
    pub kind: String,
    #[serde(default)]
    pub service: Option<SelfService>,
    #[serde(default)]
    pub code: Option<SelfCode>,
}

/// Compile-time coupling between two units.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfCoupling {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// An upstream dependency verdict.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfUpstream {
    pub name: String,
    pub verdict: String,
}

/// Knowledge graph telemetry fused into the self-model.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfGraphMeta {
    #[serde(default)]
    pub present: bool,
    #[serde(default)]
    pub nodes: usize,
    #[serde(default)]
    pub external: usize,
    #[serde(default)]
    pub stale: Vec<String>,
    #[serde(default)]
    pub unmatched: Vec<String>,
}

/// Committed self-model from `self.json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelfModel {
    #[serde(default)]
    pub schema: usize,
    #[serde(default)]
    pub generator: String,
    #[serde(default)]
    pub units: Vec<SelfUnit>,
    #[serde(default)]
    pub coupling: Vec<SelfCoupling>,
    #[serde(default)]
    pub upstreams: Vec<SelfUpstream>,
    #[serde(default)]
    pub graph: Option<SelfGraphMeta>,
}

/// Response returned by `GET /api/sjel-status/self`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct SelfModelResponse {
    pub model: SelfModel,
    /// Live health status per capability: `Some(true)` running, `Some(false)` off, `None` unpolled.
    #[serde(default)]
    pub live: BTreeMap<String, Option<bool>>,
}

/// Return all unit names that participate in at least one coupling.
pub fn coupled_names(model: &SelfModel) -> HashSet<String> {
    let mut names = HashSet::new();
    for c in &model.coupling {
        names.insert(c.from.clone());
        names.insert(c.to.clone());
    }
    names
}

/// Return units that couple to nothing and are compiled into nothing.
pub fn isolated_units<'a>(model: &'a SelfModel) -> Vec<&'a SelfUnit> {
    let coupled = coupled_names(model);
    model
        .units
        .iter()
        .filter(|u| !coupled.contains(&u.name))
        .collect()
}

/// Return units that participate in compile-time coupling.
pub fn coupled_units<'a>(model: &'a SelfModel) -> Vec<&'a SelfUnit> {
    let coupled = coupled_names(model);
    model
        .units
        .iter()
        .filter(|u| coupled.contains(&u.name))
        .collect()
}

/// Filter units by architectural kind.
pub fn units_by_kind<'a>(model: &'a SelfModel, kind: &str) -> Vec<&'a SelfUnit> {
    model.units.iter().filter(|u| u.kind == kind).collect()
}

/// Find outgoing compile-time couplings (what this unit pulls in).
pub fn outgoing_couplings<'a>(model: &'a SelfModel, unit: &str) -> Vec<&'a SelfCoupling> {
    model.coupling.iter().filter(|c| c.from == unit).collect()
}

/// Find incoming compile-time couplings (what pulls this unit in).
pub fn incoming_couplings<'a>(model: &'a SelfModel, unit: &str) -> Vec<&'a SelfCoupling> {
    model.coupling.iter().filter(|c| c.to == unit).collect()
}

/// Total coupling degree (incoming + outgoing).
pub fn unit_degree(model: &SelfModel, unit: &str) -> usize {
    model
        .coupling
        .iter()
        .filter(|c| c.from == unit || c.to == unit)
        .count()
}

/// Live health state of a unit: `Some(true)` for running, `Some(false)` for down, `None` for unpolled.
pub fn unit_liveness(res: &SelfModelResponse, unit: &str) -> Option<bool> {
    res.live.get(unit).copied().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> SelfModelResponse {
        let mut live = BTreeMap::new();
        live.insert("calendar".to_string(), Some(true));
        live.insert("assistant".to_string(), Some(false));
        live.insert("civil-date".to_string(), None);

        SelfModelResponse {
            model: SelfModel {
                schema: 2,
                generator: "tools/self".into(),
                units: vec![
                    SelfUnit {
                        name: "calendar".into(),
                        kind: "capability".into(),
                        service: Some(SelfService {
                            kind: "process".into(),
                            port: Some("8087".into()),
                            requires: vec![],
                            image: None,
                        }),
                        code: Some(SelfCode {
                            files: 12,
                            nodes: 140,
                        }),
                    },
                    SelfUnit {
                        name: "assistant".into(),
                        kind: "capability".into(),
                        service: Some(SelfService {
                            kind: "process".into(),
                            port: Some("8100".into()),
                            requires: vec!["calendar".into()],
                            image: None,
                        }),
                        code: Some(SelfCode {
                            files: 8,
                            nodes: 95,
                        }),
                    },
                    SelfUnit {
                        name: "civil-date".into(),
                        kind: "lib".into(),
                        service: None,
                        code: Some(SelfCode {
                            files: 3,
                            nodes: 30,
                        }),
                    },
                    SelfUnit {
                        name: "backup".into(),
                        kind: "capability".into(),
                        service: None,
                        code: Some(SelfCode {
                            files: 5,
                            nodes: 50,
                        }),
                    },
                ],
                coupling: vec![SelfCoupling {
                    from: "calendar".into(),
                    to: "civil-date".into(),
                    kinds: vec!["cargo-dep".into()],
                    evidence: vec!["Cargo.toml".into()],
                }],
                upstreams: vec![SelfUpstream {
                    name: "chrono".into(),
                    verdict: "pass".into(),
                }],
                graph: Some(SelfGraphMeta {
                    present: true,
                    nodes: 7500,
                    external: 120,
                    stale: vec![],
                    unmatched: vec![],
                }),
            },
            live,
        }
    }

    #[test]
    fn test_coupled_and_isolated_units() {
        let res = sample_response();
        let coupled = coupled_units(&res.model);
        assert_eq!(coupled.len(), 2);
        let coupled_names: Vec<&str> = coupled.iter().map(|u| u.name.as_str()).collect();
        assert!(coupled_names.contains(&"calendar"));
        assert!(coupled_names.contains(&"civil-date"));

        let isolated = isolated_units(&res.model);
        assert_eq!(isolated.len(), 2);
        let isolated_names: Vec<&str> = isolated.iter().map(|u| u.name.as_str()).collect();
        assert!(isolated_names.contains(&"assistant"));
        assert!(isolated_names.contains(&"backup"));
    }

    #[test]
    fn test_couplings_direction_and_degree() {
        let res = sample_response();
        let out = outgoing_couplings(&res.model, "calendar");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to, "civil-date");

        let inc = incoming_couplings(&res.model, "civil-date");
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].from, "calendar");

        assert_eq!(unit_degree(&res.model, "calendar"), 1);
        assert_eq!(unit_degree(&res.model, "civil-date"), 1);
        assert_eq!(unit_degree(&res.model, "assistant"), 0);
    }

    #[test]
    fn test_unit_liveness() {
        let res = sample_response();
        assert_eq!(unit_liveness(&res, "calendar"), Some(true));
        assert_eq!(unit_liveness(&res, "assistant"), Some(false));
        assert_eq!(unit_liveness(&res, "civil-date"), None);
        assert_eq!(unit_liveness(&res, "unknown"), None);
    }

    #[test]
    fn test_units_by_kind() {
        let res = sample_response();
        let caps = units_by_kind(&res.model, "capability");
        assert_eq!(caps.len(), 3);
        let libs = units_by_kind(&res.model, "lib");
        assert_eq!(libs.len(), 1);
    }

    #[test]
    fn test_deserialization_from_self_json() {
        let raw = include_str!("../../../../self.json");
        let model: SelfModel = serde_json::from_str(raw).expect("Valid self.json");
        assert!(model.units.len() >= 30);
        assert!(model.coupling.len() >= 10);
    }
}
