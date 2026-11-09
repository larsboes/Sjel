//! GPUI detail slots projected into the shared travel map model.
use super::details::DetailState;
pub use markers::{LocationMarker, PlaceCategory};
use sjel_gpui_core::travel::{markers, Plan};
use std::collections::HashMap;

pub(crate) fn build_location_markers(
    plans: &[Plan],
    details: &HashMap<String, DetailState>,
    category: Option<PlaceCategory>,
) -> Vec<LocationMarker> {
    let saved = details
        .iter()
        .filter_map(|(id, slot)| slot.value.as_ref().map(|v| (id.clone(), v)))
        .collect();
    markers::build_location_markers(plans, &saved, category)
}

pub(crate) fn build_current_plan_markers(
    plan: &Plan,
    details: Option<&DetailState>,
    category: Option<PlaceCategory>,
) -> Vec<LocationMarker> {
    markers::build_current_plan_markers(plan, details.and_then(|s| s.value.as_ref()), category)
}
