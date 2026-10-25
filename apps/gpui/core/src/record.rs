//! A typed record and the rows that refer to it or share its dates.

use futures::future::join_all;
use serde::Deserialize;

use crate::shell::{CalendarEntry, CalendarEntryDetail, Shell, ShellError, ShellResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Reference,
    Coincidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CapabilityLinks {
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub unlinkable: usize,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Link {
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub meta: Option<String>,
    #[serde(default)]
    pub via: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelatedRecord {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub meta: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub capability: String,
    pub basis: Basis,
    pub label: String,
    pub records: Vec<RelatedRecord>,
    pub unlinkable: usize,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub properties: Vec<Property>,
    pub groups: Vec<Group>,
}

/// Resolve a typed id. Calendar entries and trips expand to their own fields; other kinds remain
/// a readable reference when their owner has not exposed a standalone read route.
pub async fn read(shell: &Shell, id: &str) -> ShellResult<View> {
    let kind = typed_kind(id).ok_or_else(|| {
        ShellError::Refused(400, Some("This record has no valid typed id.".into()))
    })?;
    let mut event_entry = None;
    let mut trip_plan = None;
    let mut view = match kind.as_str() {
        "cal:entry" => {
            let entry = shell.calendar_entry(id).await?;
            let view = event_view(id, &entry);
            event_entry = Some(entry);
            view
        }
        "trip:plan" => {
            let plan = shell.travel_details(id).await?.plan;
            let view = trip_view(id, &plan);
            trip_plan = Some(plan);
            view
        }
        _ => View {
            id: id.into(),
            kind: kind.clone(),
            title: id.into(),
            properties: vec![Property {
                label: "Kind".into(),
                value: kind.clone(),
            }],
            groups: Vec::new(),
        },
    };

    let registry = shell.capability_panels().await?;
    let reverse = join_all(
        registry
            .into_iter()
            .filter(|capability| capability.links_to.iter().any(|target| target == &kind))
            .map(|capability| async move {
                let name = capability.name;
                match shell.capability_links(&name, id).await {
                    Ok(answer) => Group {
                        capability: name.clone(),
                        basis: Basis::Reference,
                        label: title_case(&name),
                        records: answer.links.into_iter().map(link_record).collect(),
                        unlinkable: answer.unlinkable,
                        error: None,
                    },
                    Err(error) => failed_group(&name, Basis::Reference, title_case(&name), error),
                }
            }),
    )
    .await;
    view.groups.extend(reverse);

    match kind.as_str() {
        "cal:entry" => {
            let entry = event_entry
                .as_ref()
                .expect("calendar entry was loaded above");
            add_event_groups(shell, &mut view, entry).await;
        }
        "trip:plan" => {
            let plan = trip_plan.as_ref().expect("trip plan was loaded above");
            add_trip_groups(shell, &mut view, plan).await;
        }
        _ => {}
    }
    remove_duplicate_coincidences(&mut view.groups);
    Ok(view)
}

fn event_view(id: &str, detail: &CalendarEntryDetail) -> View {
    let entry = &detail.entry;
    let mut properties = vec![Property {
        label: "Starts".into(),
        value: event_time(&entry.starts_at, entry.all_day),
    }];
    push(
        &mut properties,
        "Ends",
        event_time(&entry.ends_at, entry.all_day),
    );
    push_opt(&mut properties, "Where", entry.location.as_deref());
    push(&mut properties, "Commitment", entry.commitment.clone());
    push_opt(&mut properties, "Notes", entry.notes.as_deref());
    View {
        id: id.into(),
        kind: "cal:entry".into(),
        title: entry.title.clone(),
        properties,
        groups: Vec::new(),
    }
}

fn trip_view(id: &str, plan: &crate::travel::Plan) -> View {
    let destination = plan
        .destinations
        .iter()
        .map(|place| place.name.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    let mut properties = vec![Property {
        label: "Dates".into(),
        value: format!("{} – {}", plan.date_start, plan.date_end),
    }];
    push(&mut properties, "Status", plan.status.clone());
    push(&mut properties, "From", plan.origin.name.clone());
    push(&mut properties, "Where", destination);
    View {
        id: id.into(),
        kind: "trip:plan".into(),
        title: plan.title.clone(),
        properties,
        groups: Vec::new(),
    }
}

async fn add_event_groups(shell: &Shell, view: &mut View, detail: &CalendarEntryDetail) {
    let entry = &detail.entry;
    let day = entry.starts_at.get(..10).unwrap_or_default();
    if day.is_empty() {
        return;
    }
    if let Some(plan_id) = detail
        .payload
        .as_ref()
        .and_then(|payload| payload.get("plan_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|_| entry.source == "trips")
    {
        let group = match shell.travel_details(plan_id).await {
            Ok(details) => Group {
                capability: "trips".into(),
                basis: Basis::Reference,
                label: "Its trip".into(),
                records: vec![plan_record(details.plan)],
                unlinkable: 0,
                error: None,
            },
            Err(error) => failed_group("trips", Basis::Reference, "Its trip".into(), error),
        };
        view.groups.push(group);
    }
    let trips = match shell.travel_plans().await {
        Ok(plans) => Group {
            capability: "trips".into(),
            basis: Basis::Coincidence,
            label: "Same days".into(),
            records: plans
                .into_iter()
                .filter(|plan| day >= plan.date_start.as_str() && day <= plan.date_end.as_str())
                .map(plan_record)
                .collect(),
            unlinkable: 0,
            error: None,
        },
        Err(error) => failed_group("trips", Basis::Coincidence, "Same days".into(), error),
    };
    view.groups.push(trips);

    let finance = match shell.finance_dashboard(Some(day), Some(day)).await {
        Ok(projection) => Group {
            capability: "finance".into(),
            basis: Basis::Coincidence,
            label: "Same day".into(),
            records: projection
                .transactions
                .into_iter()
                .take(8)
                .map(|row| RelatedRecord {
                    id: row
                        .source_id
                        .as_deref()
                        .map(|source| format!("fin:tx:{source}"))
                        .unwrap_or(row.id),
                    kind: "fin:tx".into(),
                    title: row.description,
                    meta: format!(
                        "{} · {}",
                        row.date,
                        crate::context::format_money(row.amount_cents, &row.currency)
                    ),
                })
                .collect(),
            unlinkable: 0,
            error: None,
        },
        Err(error) => failed_group("finance", Basis::Coincidence, "Same day".into(), error),
    };
    view.groups.push(finance);
}

async fn add_trip_groups(shell: &Shell, view: &mut View, plan: &crate::travel::Plan) {
    let group = match shell.calendar(&plan.date_start, &plan.date_end).await {
        Ok(entries) => Group {
            capability: "calendar".into(),
            basis: Basis::Coincidence,
            label: "Same days".into(),
            records: entries.into_iter().take(8).map(event_record).collect(),
            unlinkable: 0,
            error: None,
        },
        Err(error) => failed_group("calendar", Basis::Coincidence, "Same days".into(), error),
    };
    view.groups.push(group);
}

fn event_record(entry: CalendarEntry) -> RelatedRecord {
    RelatedRecord {
        id: entry.id,
        kind: "cal:entry".into(),
        title: entry.title,
        meta: event_time(&entry.starts_at, entry.all_day),
    }
}

fn plan_record(plan: crate::travel::Plan) -> RelatedRecord {
    let destination = plan
        .destinations
        .iter()
        .map(|place| place.name.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    RelatedRecord {
        id: plan.id,
        kind: "trip:plan".into(),
        title: plan.title,
        meta: format!("{} – {} · {}", plan.date_start, plan.date_end, destination),
    }
}

fn link_record(link: Link) -> RelatedRecord {
    RelatedRecord {
        id: link.id,
        kind: link.kind,
        title: link.title,
        meta: [link.at, link.meta]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
    }
}

fn failed_group(capability: &str, basis: Basis, label: String, error: impl ToString) -> Group {
    Group {
        capability: capability.into(),
        basis,
        label,
        records: Vec::new(),
        unlinkable: 0,
        error: Some(format!(
            "{capability} did not answer: {}",
            error.to_string()
        )),
    }
}

fn remove_duplicate_coincidences(groups: &mut [Group]) {
    let references: std::collections::HashSet<String> = groups
        .iter()
        .filter(|group| group.basis == Basis::Reference)
        .flat_map(|group| group.records.iter().map(|record| record.id.clone()))
        .collect();
    for group in groups
        .iter_mut()
        .filter(|group| group.basis == Basis::Coincidence)
    {
        group
            .records
            .retain(|record| !references.contains(&record.id));
    }
}

fn typed_kind(id: &str) -> Option<String> {
    if id.len() > 256
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
    {
        return None;
    }
    let cut = id.rfind(':')?;
    if cut == 0 || cut == id.len() - 1 {
        return None;
    }
    let kind = &id[..cut];
    kind.split(':')
        .all(|part| !part.is_empty())
        .then(|| kind.into())
}

fn event_time(value: &str, all_day: bool) -> String {
    if all_day {
        value.get(..10).unwrap_or(value).to_owned()
    } else {
        value.get(..16).unwrap_or(value).replace('T', " ")
    }
}

fn push(properties: &mut Vec<Property>, label: &str, value: String) {
    if !value.trim().is_empty() {
        properties.push(Property {
            label: label.into(),
            value,
        });
    }
}

fn push_opt(properties: &mut Vec<Property>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        push(properties, label, value.into());
    }
}

fn title_case(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
