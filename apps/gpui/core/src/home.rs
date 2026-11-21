//! Deterministic adaptive Home/Today layout planner shared across desktop and mobile surfaces.

/// High-level layout mode selected by the operator or left on [`HomeContextMode::Auto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HomeContextMode {
    #[default]
    Auto,
    Focus,
    Travel,
}

/// Right-rail / secondary sections available on Home/Today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeRailTab {
    Scratchpad,
    Prompts,
    Inbox,
    Projects,
    Travel,
}

/// Live signals evaluated by [`plan_home_layout`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HomeSignals {
    pub mode: HomeContextMode,
    pub urgent_tasks: usize,
    pub approvals: usize,
    pub degraded_capabilities: usize,
    pub prompt_drafts: usize,
    pub trip_days_away: Option<i64>,
}

/// Ordered Home layout plan and a short human-readable explanation of why it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeAdaptivePlan {
    pub rail_order: Vec<HomeRailTab>,
    pub reason: &'static str,
    pub show_ambient_cards: bool,
}

/// Computes the adaptive Home layout order and explainability label from live signals.
pub fn plan_home_layout(signals: &HomeSignals) -> HomeAdaptivePlan {
    match signals.mode {
        HomeContextMode::Focus => HomeAdaptivePlan {
            rail_order: vec![HomeRailTab::Scratchpad],
            reason: "Focus · Distraction-free",
            show_ambient_cards: false,
        },
        HomeContextMode::Travel => HomeAdaptivePlan {
            rail_order: vec![
                HomeRailTab::Travel,
                HomeRailTab::Scratchpad,
                HomeRailTab::Prompts,
                HomeRailTab::Inbox,
                HomeRailTab::Projects,
            ],
            reason: "Travel · Trip & Map pinned",
            show_ambient_cards: true,
        },
        HomeContextMode::Auto => {
            if signals.trip_days_away.is_some_and(|days| days <= 2) {
                HomeAdaptivePlan {
                    rail_order: vec![
                        HomeRailTab::Travel,
                        HomeRailTab::Prompts,
                        HomeRailTab::Scratchpad,
                        HomeRailTab::Inbox,
                        HomeRailTab::Projects,
                    ],
                    reason: "Auto · Trip within 48h",
                    show_ambient_cards: true,
                }
            } else if signals.approvals > 0
                || signals.urgent_tasks > 0
                || signals.degraded_capabilities > 0
            {
                HomeAdaptivePlan {
                    rail_order: vec![
                        HomeRailTab::Prompts,
                        HomeRailTab::Inbox,
                        HomeRailTab::Projects,
                        HomeRailTab::Scratchpad,
                        HomeRailTab::Travel,
                    ],
                    reason: "Auto · Triage & urgent work",
                    show_ambient_cards: true,
                }
            } else if signals.prompt_drafts > 0 {
                HomeAdaptivePlan {
                    rail_order: vec![
                        HomeRailTab::Prompts,
                        HomeRailTab::Scratchpad,
                        HomeRailTab::Inbox,
                        HomeRailTab::Projects,
                        HomeRailTab::Travel,
                    ],
                    reason: "Auto · Prompt drafts ready",
                    show_ambient_cards: true,
                }
            } else {
                HomeAdaptivePlan {
                    rail_order: vec![
                        HomeRailTab::Scratchpad,
                        HomeRailTab::Prompts,
                        HomeRailTab::Inbox,
                        HomeRailTab::Projects,
                        HomeRailTab::Travel,
                    ],
                    reason: "Auto · Balanced day",
                    show_ambient_cards: true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_planner_promotes_imminent_travel_and_focus_filters_noise() {
        let travel_plan = plan_home_layout(&HomeSignals {
            mode: HomeContextMode::Auto,
            urgent_tasks: 2,
            trip_days_away: Some(1),
            ..Default::default()
        });
        assert_eq!(travel_plan.rail_order[0], HomeRailTab::Travel);
        assert_eq!(travel_plan.reason, "Auto · Trip within 48h");
        assert!(travel_plan.show_ambient_cards);

        let triage_plan = plan_home_layout(&HomeSignals {
            mode: HomeContextMode::Auto,
            approvals: 1,
            trip_days_away: Some(10),
            ..Default::default()
        });
        assert_eq!(triage_plan.rail_order[0], HomeRailTab::Prompts);
        assert_eq!(triage_plan.reason, "Auto · Triage & urgent work");

        let focus_plan = plan_home_layout(&HomeSignals {
            mode: HomeContextMode::Focus,
            trip_days_away: Some(0),
            approvals: 3,
            ..Default::default()
        });
        assert_eq!(focus_plan.rail_order, vec![HomeRailTab::Scratchpad]);
        assert!(!focus_plan.show_ambient_cards);
    }
}
