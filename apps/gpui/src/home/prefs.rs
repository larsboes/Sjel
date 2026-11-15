use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Situational layout mode for the Today page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeMode {
    /// Adapts to context: expands the map when a trip is active or within 48h.
    #[default]
    Auto,
    /// Distraction-free mode: hides travel/map and spending noise, maximizes tasks & timeline.
    Focus,
    /// Travel mode: keeps the full trip and map card expanded in the right rail.
    Travel,
}

impl HomeMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Focus, Self::Travel];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Focus => "Focus",
            Self::Travel => "Travel",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Auto => "Adapts to schedule and upcoming trips",
            Self::Focus => "Tasks and schedule only",
            Self::Travel => "Keeps trip and map expanded",
        }
    }
}

/// Persisted customization for the Today page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomePreferences {
    pub mode: HomeMode,
    pub show_timeline: bool,
    pub show_tasks: bool,
    pub show_prompts: bool,
    pub show_travel: bool,
    pub show_spending: bool,
    pub large_map: bool,
}

impl Default for HomePreferences {
    fn default() -> Self {
        Self {
            mode: HomeMode::Auto,
            show_timeline: true,
            show_tasks: true,
            show_prompts: true,
            show_travel: true,
            show_spending: false,
            large_map: false,
        }
    }
}

impl HomePreferences {
    pub fn config_path() -> Option<PathBuf> {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("sjel/home.json"));
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home).join(".config/sjel/home.json"));
        }
        None
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<Self>(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("Could not determine config path".into());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create preferences directory: {error}"))?;
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("Serialization error: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("Write error to {path:?}: {e}"))
    }

    pub fn effective_show_spending(&self) -> bool {
        self.show_spending && self.mode != HomeMode::Focus
    }

    pub fn effective_show_travel(&self) -> bool {
        self.show_travel && self.mode != HomeMode::Focus
    }

    pub fn force_expanded_map(&self) -> bool {
        self.mode == HomeMode::Travel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_focus_mode_rules() {
        let mut prefs = HomePreferences::default();
        assert_eq!(prefs.mode, HomeMode::Auto);
        assert!(!prefs.effective_show_spending());
        assert!(prefs.effective_show_travel());
        assert!(!prefs.force_expanded_map());

        prefs.show_spending = true;
        assert!(prefs.effective_show_spending());

        prefs.mode = HomeMode::Focus;
        assert!(!prefs.effective_show_spending());
        assert!(!prefs.effective_show_travel());

        prefs.mode = HomeMode::Travel;
        assert!(prefs.force_expanded_map());
        assert!(prefs.effective_show_travel());
    }

    #[test]
    fn json_round_trip() {
        let prefs = HomePreferences {
            mode: HomeMode::Travel,
            show_timeline: true,
            show_tasks: true,
            show_prompts: false,
            show_travel: true,
            show_spending: false,
            large_map: true,
        };
        let raw = serde_json::to_string(&prefs).expect("serialize");
        let parsed: HomePreferences = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(parsed, prefs);
    }
}
