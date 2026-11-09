//! Shared layout candidate projection and capability-owned validation findings.
#[derive(Clone, Copy)]
pub enum PlanningKind {
    Search,
    Compose,
}

#[derive(Clone)]
pub struct PlanningCandidate {
    pub title: String,
    pub details: String,
    pub items: Vec<serde_json::Value>,
}

pub fn planning_candidates(
    kind: PlanningKind,
    outcome: &serde_json::Value,
    base: &[super::RoomPlanSavedLayoutItem],
) -> Vec<PlanningCandidate> {
    match kind {
        PlanningKind::Search => outcome
            .get("hits")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, hit)| {
                let places = hit.get("places")?.as_object()?;
                let mut items = base
                    .iter()
                    .filter_map(|item| serde_json::to_value(item).ok())
                    .collect::<Vec<_>>();
                for item in &mut items {
                    let reference = item.get("ref")?.as_str()?.to_owned();
                    if let Some(position) =
                        places.get(&reference).and_then(serde_json::Value::as_array)
                    {
                        if position.len() == 2 {
                            item["x"] = position[0].clone();
                            item["y"] = position[1].clone();
                        }
                    }
                }
                let reserve = hit
                    .get("tightest_reserve_cm")
                    .and_then(serde_json::Value::as_i64)
                    .map_or_else(
                        || "reserve not reported".to_owned(),
                        |cm| format!("{cm} cm reserve"),
                    );
                let details = format!(
                    "{} soft findings · {} cm narrowest route · {} cm wall contact · {}{}",
                    hit.get("soft")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    hit.get("bottleneck_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    hit.get("wall_contact_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    reserve,
                    if hit
                        .get("pareto")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        " · Pareto choice"
                    } else {
                        ""
                    }
                );
                Some(PlanningCandidate {
                    title: format!("RoomPlan search proposal {}", index + 1),
                    details,
                    items,
                })
            })
            .collect(),
        PlanningKind::Compose => outcome
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, result)| {
                let places = result.get("places")?.as_array()?;
                let items = places
                    .iter()
                    .filter_map(|place| {
                        let tuple = place.as_array()?;
                        Some(serde_json::json!({
                            "ref": tuple.first()?.as_str()?,
                            "x": tuple.get(1)?.get(0)?.as_i64()?,
                            "y": tuple.get(1)?.get(1)?.as_i64()?,
                            "rot": tuple.get(2)?.as_i64()?,
                        }))
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    return None;
                }
                let hard = result
                    .get("hard")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let soft = result
                    .get("soft")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let hard_rules = result
                    .get("hard")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ");
                let soft_rules = result
                    .get("soft")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ");
                let details = format!(
                    "{} · {} hard ({}) · {} soft ({}) · {} cm narrowest route · {:.1} m² free",
                    if result
                        .get("pass")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        "Passes checks"
                    } else {
                        "Needs review"
                    },
                    hard,
                    if hard_rules.is_empty() {
                        "none"
                    } else {
                        hard_rules.as_str()
                    },
                    soft,
                    if soft_rules.is_empty() {
                        "none"
                    } else {
                        soft_rules.as_str()
                    },
                    result
                        .get("bottleneck_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    result
                        .get("free_m2")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0),
                );
                Some(PlanningCandidate {
                    title: format!("RoomPlan composition {}", index + 1),
                    details,
                    items,
                })
            })
            .collect(),
    }
}

pub fn add_planning_analysis(candidate: &mut PlanningCandidate, review: &serde_json::Value) {
    let Some(check) = review.get("check") else {
        candidate.details.push_str(" · Detailed review unavailable");
        return;
    };
    let violations = |severity: &str| {
        check
            .get(severity)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|finding| {
                let rule = finding.get("rule")?.as_str()?;
                let message = finding
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("details unavailable");
                Some(format!("{rule}: {message}"))
            })
            .collect::<Vec<_>>()
    };
    let hard = violations("hard");
    let soft = violations("soft");
    let review_gaps = check
        .get("unchecked")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|gap| {
            Some(format!(
                "{}: {}",
                gap.get("rule")?.as_str()?,
                gap.get("reason")?.as_str()?
            ))
        })
        .collect::<Vec<_>>();
    let uncertainties = check
        .get("uncertainties")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|uncertainty| {
            Some(format!(
                "{}: {}",
                uncertainty.get("label")?.as_str()?,
                uncertainty
                    .get("fields")?
                    .as_array()?
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
        .collect::<Vec<_>>();
    let mut corridors = check
        .get("metrics")
        .and_then(|metrics| metrics.get("corridors"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|route| {
            let from = route.get("from")?.as_str()?;
            let to = route.get("to")?.as_str()?;
            let width = route.get("width_cm").and_then(serde_json::Value::as_i64);
            Some((
                width.unwrap_or(i64::MAX),
                format!(
                    "{from}–{to} {}",
                    width.map_or_else(|| "unmeasured".into(), |cm| format!("{cm} cm"))
                ),
            ))
        })
        .collect::<Vec<_>>();
    corridors.sort_by_key(|(width, _)| *width);
    let routes = corridors
        .iter()
        .take(3)
        .map(|(_, route)| route.as_str())
        .collect::<Vec<_>>();
    let sun = review
        .get("sun")
        .and_then(|sun| sun.get("hits_per_piece"))
        .and_then(serde_json::Value::as_object)
        .map(|hits| {
            let checked = review
                .get("sun")
                .and_then(|sun| sun.get("hours"))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let mut pieces = hits
                .iter()
                .filter_map(|(reference, count)| Some((reference.as_str(), count.as_u64()?)))
                .collect::<Vec<_>>();
            pieces.sort_by_key(|piece| std::cmp::Reverse(piece.1));
            if pieces.is_empty() {
                format!("no direct sun across {checked} checked hours")
            } else {
                format!(
                    "direct sun in {checked} checked hours: {}",
                    pieces
                        .iter()
                        .take(3)
                        .map(|(reference, count)| format!("{reference} {count}h"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        })
        .or_else(|| {
            review
                .get("sun_note")
                .and_then(serde_json::Value::as_str)
                .map(|note| format!("sunlight unavailable: {note}"))
        })
        .unwrap_or_else(|| "sunlight unavailable".into());
    candidate.details = format!(
        "{} · {} hard findings{} · {} soft findings{} · unchecked: {} · uncertain: {} · access paths: {} · {}",
        candidate.details,
        hard.len(),
        if hard.is_empty() {
            String::new()
        } else {
            format!(" [{}]", hard.join("; "))
        },
        soft.len(),
        if soft.is_empty() {
            String::new()
        } else {
            format!(" [{}]", soft.join("; "))
        },
        if review_gaps.is_empty() {
            "none".into()
        } else {
            review_gaps.join("; ")
        },
        if uncertainties.is_empty() {
            "none".into()
        } else {
            uncertainties.join("; ")
        },
        if routes.is_empty() {
            "none reported".into()
        } else {
            routes.join(", ")
        },
        sun,
    );
}
