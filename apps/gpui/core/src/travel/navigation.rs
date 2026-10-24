//! Routes within the shared travel workspace.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TripSection {
    #[default]
    Overview,
    Itinerary,
    Map,
    Items,
}

impl TripSection {
    pub const ALL: [Self; 4] = [Self::Overview, Self::Itinerary, Self::Map, Self::Items];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Itinerary => "Itinerary",
            Self::Map => "Map",
            Self::Items => "Saved items",
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Itinerary => "itinerary",
            Self::Map => "map",
            Self::Items => "items",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TravelRoute {
    #[default]
    Trips,
    /// A map of the current trip, also reached from the app's Map destination.
    Map,
    /// Train connections and split tickets between two stations (`/travel/connections`).
    Connections,
    Trip {
        id: String,
        section: TripSection,
    },
}

impl TravelRoute {
    /// Recognizes native trip paths. Unsupported subpaths stay with the dashboard.
    pub fn parse(path: &str) -> Option<Self> {
        if path == "/map" {
            return Some(Self::Map);
        }
        if path == "/travel" {
            return Some(Self::Trips);
        }
        if path == "/travel/connections" {
            return Some(Self::Connections);
        }
        let mut parts = path.strip_prefix("/travel/")?.split('/');
        let id = parts.next()?;
        if !valid_id(id) {
            return None;
        }
        let section = match parts.next() {
            None | Some("overview") => TripSection::Overview,
            Some("itinerary") => TripSection::Itinerary,
            Some("map") => TripSection::Map,
            Some("items") => TripSection::Items,
            _ => return None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self::Trip {
            id: id.to_owned(),
            section,
        })
    }

    pub fn href(&self) -> String {
        match self {
            Self::Trips => "/travel".into(),
            Self::Map => "/map".into(),
            Self::Connections => "/travel/connections".into(),
            Self::Trip { id, section } => format!("/travel/{id}/{}", section.path()),
        }
    }
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_round_trip() {
        for section in TripSection::ALL {
            let route = TravelRoute::Trip {
                id: "trip:plan:1234abcd".into(),
                section,
            };
            assert_eq!(TravelRoute::parse(&route.href()), Some(route));
        }
        assert_eq!(
            TravelRoute::parse("/travel/connections"),
            Some(TravelRoute::Connections)
        );
        assert_eq!(
            TravelRoute::parse("/travel/plan_42"),
            Some(TravelRoute::Trip {
                id: "plan_42".into(),
                section: TripSection::Overview
            })
        );
        for path in [
            "/travel/../map",
            "/travel/a/unknown",
            "/travel/a/map/extra",
            "/travel//map",
            "/travel/a?x=y",
        ] {
            assert_eq!(TravelRoute::parse(path), None);
        }
    }
}
