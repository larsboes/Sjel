//! Data models and pure reductions for the Self-model (`/self`).
//!
//! Visualizes Sjel's architectural units (capabilities, libraries, packs, spines),
//! compile-time couplings, runtime dependencies, code sizes, and live service status.

#[cfg(test)]
use std::collections::BTreeMap;

pub use status_api::{
    coupled_names, coupled_units, incoming_couplings, isolated_units, outgoing_couplings,
    unit_degree, unit_liveness, units_by_kind, SelfCode, SelfCoupling, SelfModel,
    SelfModelResponse, SelfService, SelfUnit, SelfUpstream,
};

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
