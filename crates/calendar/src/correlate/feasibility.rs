use super::*;

/// The three verdicts of the Calendar correlation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feasibility {
    Free,
    NeedsTravelDay,
    Conflicts,
}

/// The most an entry at this commitment is allowed to cost, whatever it is.
///
/// This is the whole reason the axis exists. An Impact Lab in Atlanta that
/// scouting merely *found* used to reach `Conflicts` through `kind = "event"`
/// and quietly delete two days from August. A thing you have not decided on
/// cannot cost you anything, so `Possible` caps at `Free`.
pub(super) fn commitment_ceiling(commitment: Commitment) -> Feasibility {
    match commitment {
        Commitment::Possible => Feasibility::Free,
        Commitment::Planned => Feasibility::NeedsTravelDay,
        Commitment::Committed => Feasibility::Conflicts,
    }
}

/// What one entry does to a candidate that overlaps it.
///
/// Two axes, and the cheaper one wins: `kind` says how much this *could* cost,
/// `commitment` caps how much it is *allowed* to. Taking the min is what keeps
/// the enabling kinds honest — a planned `travel_ok` is still free, because
/// planning to be up for a trip was never a cost.
///
/// Deliberately total on the kind side: a kind this layer has never seen
/// returns `Free`. That is the other half of "kinds are data, not a
/// constraint" — a day-planning kind added later lands without a migration
/// *and* without silently blocking travel on every day it covers.
pub fn impact(kind: &str, commitment: Commitment) -> Feasibility {
    kind_ceiling(kind).min(commitment_ceiling(commitment))
}

/// What this kind costs when it is actually happening.
pub(super) fn kind_ceiling(kind: &str) -> Feasibility {
    match kind {
        // You are somewhere else, or already booked into something concrete
        // on that day. The contract's free clause ("or only travel_ok/event")
        // is the candidate's *own* already-promoted entry, which never reaches
        // this function — see `Verdict::already_in_calendar`. Any *other*
        // committed event is a real clash: you cannot attend both.
        "away" | "event" | "nightlife" => Feasibility::Conflicts,
        // Movable, so still offered with the cost named: on-site work can go
        // remote, a busy block can be dropped.
        "work_onsite" | "busy" => Feasibility::NeedsTravelDay,
        // work_remote is location-flexible by definition, travel_ok is the
        // explicit yes. Both stay in the evidence so the UI can mention them;
        // neither raises the verdict. Unknown kinds land here too — including
        // the imports that used to be spelled `kind = "draft"`, which are now
        // ordinary kinds at `Commitment::Possible`.
        _ => Feasibility::Free,
    }
}
