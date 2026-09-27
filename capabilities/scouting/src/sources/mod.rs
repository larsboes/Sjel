//! Source registry — config-driven opportunity source discovery and adapter instantiation.
//!
//! "Where do opportunities come from?" is answered by a `sources` array in `scouting.json`,
//! not by hardcoded match arms in `main.rs` + `server.rs`. Each source entry declares an
//! adapter type, a location (path or URL), and what it provides (events, profiles, or both).
//! The factory below creates the right `SourceAdapter` for each declared type.
//!
//! See `capabilities/scouting/README.md` for the architecture call.
//! See `scouting.config.example.json` for the config shape.
//!
//! Currently shipped adapters:
//!   - `obsidian-markdown` — reads markdown event files from a vault directory
//!   - `rss` — any RSS/Atom feed
//!   - `luma-calendar` — one Luma calendar's future events, by `cal-…` api id
//!   - `splash-hub` — one Splash That brand event hub's upcoming events, by
//!     `<host>/<hub_id>`; white-label, so one adapter covers every brand there
//!
//! Adding a new adapter type: add an arm to `create_adapter()`, implement `SourceAdapter`,
//! register the module. That's it — no changes to `main.rs`, `server.rs`, or the pipeline.

pub mod obsidian_md;
pub mod rss;

use std::path::PathBuf;

use serde::Deserialize;

use crate::opportunity::OpportunityType;
use crate::source::SourceAdapter;

// ---------------------------------------------------------------------------
// Config-side: deserialized from `scouting.json`'s `sources[]` array.
// See `scouting.config.example.json` for the documented shape.
// ---------------------------------------------------------------------------

/// Declared opportunity source — the "what" half. A `SourceManifest` is the resolved runtime form.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceEntry {
    /// Unique id for this source, used as both the adapter name (for `--adapter <id>`)
    /// and the display name in `--list-sources`.
    pub id: String,

    /// Adapter type: `"obsidian-markdown"`, `"rss"` or `"luma-calendar"` —
    /// the `create_adapter()` arms; anything else is `UnknownAdapter`.
    /// Must match an arm in `create_adapter()`.
    pub adapter: String,

    /// Root path — for file-based sources (vaults, JSON dumps).
    /// Supports `~/` expansion.
    #[serde(default)]
    pub path: Option<String>,

    /// URL — for network-based sources (RSS feeds, APIs).
    #[serde(default)]
    pub url: Option<String>,

    /// Glob pattern for event files, relative to `path`. Defaults to
    /// adapter-specific convention if absent.
    #[serde(default)]
    pub events_glob: Option<String>,

    /// Glob or exact markdown file for opportunities, relative to `path`.
    /// Prefer this field for new sources. `events_glob` remains as a
    /// backwards-compatible alias for event registries.
    #[serde(default)]
    pub opportunities_glob: Option<String>,

    /// Opportunity type emitted by a generic source adapter. Defaults to
    /// `event`, preserving existing source entries.
    #[serde(default)]
    pub opportunity_type: Option<OpportunityType>,

    /// Glob pattern for interest-profile files, relative to `profile_path`
    /// when one is declared and to `path` otherwise.
    #[serde(default)]
    pub profiles_glob: Option<String>,

    /// Root the interest profile resolves under, when that is not the store the
    /// opportunities come from. Supports `~/` expansion.
    ///
    /// A matching profile is a consumer input — an operator-curated or
    /// TELOS-derived predicate about what is worth surfacing. It is not a
    /// required resident of whichever knowledge store happens to hold the
    /// opportunity notes, and the two have separate sync lifecycles. Absent,
    /// the profile resolves under `path`, exactly as it did before this field.
    #[serde(default)]
    pub profile_path: Option<String>,

    /// Path to human-readable documentation about this source, relative to `path`.
    /// Displayed by `--list-sources --verbose`.
    #[serde(default)]
    pub doc: Option<String>,

    /// Whether this source is active. Disabled sources are listed but not polled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// What this source's opportunities are worth protecting: `c0`, `c1`, `c2`
    /// or `c3`. Absent means undeclared, which is c1.
    ///
    /// The declaration is here rather than in Rust for the reason comms already
    /// gives for the identical field on `FeedSourceConfig`: the operator decides
    /// what a source collects, and the only class that can be reached by silence
    /// is the one nobody chose. `c0` is reachable only through a positive
    /// declaration.
    ///
    /// Unlike comms', it is OPTIONAL, and the difference is deliberate: comms
    /// refuses to load a feed source that omits the key, which it can afford
    /// because it shipped the key with the sources. Scouting's `sources[]` are
    /// already deployed and would all fail to load. Absent is therefore treated
    /// as undeclared and answered with `DataClass::undeclared()` -- the strict
    /// direction, never `c0` -- so an unmigrated config loses nothing but the
    /// ability to say "public".
    #[serde(default)]
    pub data_class: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// The class a source declares, refusing anything outside the vocabulary.
///
/// Three inputs, one direction. A valid literal is honoured. A literal that is
/// not a class is a typo, and a typo must not be more permissive than saying
/// nothing, so it is refused down to the undeclared default and reported --
/// exactly the rule `comms::config` applies to the same field, and the reason
/// it is reported at load rather than dropped at ingest. Nothing at all is the
/// undeclared default too, and that default is `c1`: never `c0`.
fn declared_class(declared: Option<&str>, id: &str) -> String {
    let fallback = content_item::DataClass::undeclared().value;
    match declared.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if content_item::valid(value) => value.to_string(),
        Some(value) => {
            eprintln!(
                "scouting: source '{id}' declares an unknown data_class '{value}'; \
                 treating it as {fallback}"
            );
            fallback
        }
        None => fallback,
    }
}

/// What an opportunity from `source` is worth protecting.
///
/// The serve-side half of the declaration above, and the one that fails closed:
/// a stored row whose source is not in `sources[]` gets `DataClass::undeclared`.
/// That covers every hardcoded adapter (luma, meetup, cfp, euro_hackathons,
/// transit_fare), a source an operator has since removed from the config, and a
/// typo'd source name — none of which anybody has declared anything about, so
/// none of them may read as `c0`.
///
/// Resolved at read time rather than stored on the row, which is the ONE place
/// this departs from comms' feed rule ("the class lands with the INSERT, and
/// every later pass may only raise it"). Comms protects a per-ITEM class a human
/// can edit; scouting has no per-item class and no surface to edit one, so the
/// class here is a property of the source and the current declaration is the
/// truthful answer for every row that came from it. The consequence is stated
/// rather than hidden: editing `data_class` in `scouting.json` re-labels rows
/// already stored, downwards as well as upwards. That edit is the operator's own
/// written declaration, which is the same authority `set_feed_data_class`
/// requires to lower a class in comms.
pub fn class_for_source(sources: &[SourceManifest], source: &str) -> String {
    sources
        .iter()
        .find(|manifest| manifest.id == source)
        .map(|manifest| manifest.data_class.clone())
        .unwrap_or_else(|| content_item::DataClass::undeclared().value)
}

// ---------------------------------------------------------------------------
// Runtime-side: resolved paths, ready for adapter construction.
// ---------------------------------------------------------------------------

/// A fully resolved source manifest — paths expanded, defaults filled.
/// Created by `SourceEntry::resolve()` and consumed by `create_adapter()`.
#[derive(Debug, Clone)]
pub struct SourceManifest {
    pub id: String,
    pub adapter: String,
    pub root_path: Option<PathBuf>,
    pub url: Option<String>,
    pub events_glob: Option<String>,
    pub opportunities_glob: Option<String>,
    pub opportunity_type: OpportunityType,
    pub profiles_glob: Option<String>,
    /// Declared profile root, when it differs from `root_path`. Resolution is
    /// `profile_root().unwrap_or(root_path)` — see `profile_location()`.
    pub profile_root: Option<PathBuf>,
    pub doc_path: Option<PathBuf>,
    pub enabled: bool,
    /// Resolved and validated by `SourceEntry::resolve` -- always one of
    /// `DATA_CLASSES`, never empty. See `declared_class`.
    pub data_class: String,
}

use markdown_root::MarkdownRoot;
use sjel_config::expand_tilde;

impl SourceEntry {
    /// Resolve into a runtime manifest: expand `~`, compute absolute doc path.
    pub fn resolve(&self) -> SourceManifest {
        let root_path = self.path.as_ref().map(|p| expand_tilde(p));
        let doc_path = self
            .doc
            .as_ref()
            .and_then(|d| root_path.as_ref().map(|root| root.join(d)));
        SourceManifest {
            id: self.id.clone(),
            adapter: self.adapter.clone(),
            root_path,
            url: self.url.clone(),
            events_glob: self.events_glob.clone(),
            opportunities_glob: self.opportunities_glob.clone(),
            opportunity_type: self.opportunity_type.unwrap_or(OpportunityType::Event),
            profiles_glob: self.profiles_glob.clone(),
            profile_root: self.profile_path.as_ref().map(|p| expand_tilde(p)),
            doc_path,
            enabled: self.enabled,
            data_class: declared_class(self.data_class.as_deref(), &self.id),
        }
    }
}

impl SourceManifest {
    /// Where this source's interest profile lives, if it declares one.
    ///
    /// `Ok(None)` means the source declares no profile, which is ordinary — the
    /// built-in network adapters score against profiles other sources declare.
    /// An `Err` is always an operator config error and says which: a glob with
    /// nothing to resolve against, or a root that is not there. Both used to be
    /// silent skips, which is how a profile could stop being applied without
    /// anything anywhere saying so.
    pub fn profile_location(&self) -> Result<Option<(MarkdownRoot, &str)>, String> {
        let Some(ref glob) = self.profiles_glob else {
            return Ok(None);
        };
        let root = self
            .profile_root
            .as_ref()
            .or(self.root_path.as_ref())
            .ok_or_else(|| {
                format!(
                    "source '{}' declares profiles_glob '{glob}' but neither \
                     'profile_path' nor 'path' to resolve it against",
                    self.id
                )
            })?;
        let declared = MarkdownRoot::declare(root.clone())
            .map_err(|e| format!("source '{}' profile root: {e}", self.id))?;
        Ok(Some((declared, glob.as_str())))
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Errors that can occur when constructing a source adapter from a manifest.
#[derive(Debug, thiserror::Error)]
pub enum SourceFactoryError {
    #[error(
        "unknown adapter type '{0}' — valid types: obsidian-markdown, rss, luma-calendar, splash-hub"
    )]
    UnknownAdapter(String),
    #[error("obsidian-markdown adapter: {0}")]
    ObsidianMd(String),
    #[error("source '{id}': {detail}")]
    Config { id: String, detail: String },
    #[error("{0}")]
    Other(String),
}

impl From<String> for SourceFactoryError {
    fn from(s: String) -> Self {
        SourceFactoryError::Other(s)
    }
}

/// Create a `SourceAdapter` from a resolved manifest.
///
/// Each arm corresponds to one adapter type. Adding a new source type means adding
/// one arm here — `main.rs` and `server.rs` both call this, no changes needed there.
pub fn create_adapter(
    manifest: &SourceManifest,
) -> Result<Box<dyn SourceAdapter>, SourceFactoryError> {
    match manifest.adapter.as_str() {
        "obsidian-markdown" => {
            let root = manifest
                .root_path
                .as_ref()
                .ok_or_else(|| SourceFactoryError::Config {
                    id: manifest.id.clone(),
                    detail: "obsidian-markdown adapter requires a 'path'".into(),
                })?;
            let opportunities_glob = manifest
                .opportunities_glob
                .clone()
                .or_else(|| manifest.events_glob.clone())
                .unwrap_or_else(|| "*.md".into());
            Ok(Box::new(obsidian_md::ObsidianMarkdownSource::new(
                manifest.id.clone(),
                root.clone(),
                opportunities_glob,
                manifest.opportunity_type,
            )?))
        }
        "rss" => {
            let url = manifest
                .url
                .clone()
                .ok_or_else(|| SourceFactoryError::Config {
                    id: manifest.id.clone(),
                    detail: "rss adapter requires a 'url'".into(),
                })?;
            Ok(Box::new(rss::RssFeedSource::new(manifest.id.clone(), url)))
        }
        // Which Luma calendars get tracked is a declaration, not a discovery
        // (see README § "sources are declared, never discovered"). The `url`
        // field carries the calendar's `cal-…` api id: Luma publishes no
        // public slug→id lookup, so the id is the only stable handle.
        "luma-calendar" => {
            let api_id = manifest
                .url
                .clone()
                .ok_or_else(|| SourceFactoryError::Config {
                    id: manifest.id.clone(),
                    detail:
                        "luma-calendar adapter requires 'url' set to the calendar's 'cal-…' api id"
                            .into(),
                })?;
            let adapter =
                crate::adapters::luma::LumaAdapter::for_calendar(api_id).map_err(|e| {
                    SourceFactoryError::Config {
                        id: manifest.id.clone(),
                        detail: e.to_string(),
                    }
                })?;
            Ok(Box::new(adapter.with_source_id(manifest.id.clone())))
        }
        // One adapter covers every brand on Splash That, because the platform
        // is white-label and the hub id is the whole address. `url` carries
        // `<host>/<hub_id>`: the query is same-origin, so the id alone does not
        // say which host answers for it.
        "splash-hub" => {
            let locator = manifest
                .url
                .clone()
                .ok_or_else(|| SourceFactoryError::Config {
                    id: manifest.id.clone(),
                    detail: "splash-hub adapter requires 'url' set to '<host>/<hub_id>'".into(),
                })?;
            let adapter = crate::adapters::splash_hub::SplashHubAdapter::for_hub(&locator)
                .map_err(|detail| SourceFactoryError::Config {
                    id: manifest.id.clone(),
                    detail,
                })?;
            Ok(Box::new(adapter.with_source_id(manifest.id.clone())))
        }
        other => Err(SourceFactoryError::UnknownAdapter(other.into())),
    }
}

/// Pretty-print all configured sources for `--list-sources`.
pub fn print_sources(sources: &[SourceManifest]) {
    if sources.is_empty() {
        println!("  (no sources configured — see sources[] in scouting.json)");
        return;
    }
    for src in sources {
        let status = if src.enabled { "enabled" } else { "disabled" };
        let adapter = &src.adapter;
        let location = src
            .root_path
            .as_ref()
            .map(|p| p.display().to_string())
            .or_else(|| src.url.clone())
            .unwrap_or_default();
        println!(
            "  {:<20}  {:<18}  {}  {}",
            src.id, adapter, status, location
        );
        if let Some(ref doc) = src.doc_path {
            println!("  {:>20}  docs: {}", "", doc.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, adapter: &str, url: Option<&str>) -> SourceManifest {
        SourceManifest {
            id: id.into(),
            adapter: adapter.into(),
            root_path: None,
            url: url.map(Into::into),
            events_glob: None,
            opportunities_glob: None,
            opportunity_type: OpportunityType::Event,
            profiles_glob: None,
            profile_root: None,
            doc_path: None,
            enabled: true,
            data_class: content_item::DataClass::undeclared().value,
        }
    }

    /// The regression behind the `&'static str` -> `&str` change on
    /// `SourceAdapter::name()`: `store::record_run` keys `source_state` on this
    /// value, so two feeds declared as separate sources used to share one cursor
    /// row because both answered "rss".
    #[test]
    fn two_sources_of_one_adapter_type_have_distinct_names() {
        let a = create_adapter(&manifest(
            "conference-rss",
            "rss",
            Some("https://example.test/a"),
        ))
        .expect("rss adapter builds from a url");
        let b = create_adapter(&manifest(
            "hackathon-rss",
            "rss",
            Some("https://example.test/b"),
        ))
        .expect("rss adapter builds from a url");

        assert_eq!(a.name(), "conference-rss");
        assert_eq!(b.name(), "hackathon-rss");
        assert_ne!(
            a.name(),
            b.name(),
            "one cursor row per source, not per adapter type"
        );
    }

    /// Same property for the other config-built network adapter: two tracked
    /// calendars are two sources.
    #[test]
    fn two_luma_calendars_have_distinct_names() {
        let a = create_adapter(&manifest(
            "berlin-events",
            "luma-calendar",
            Some("cal-aaa111"),
        ))
        .expect("luma adapter builds from a cal- id");
        let b = create_adapter(&manifest(
            "bonn-events",
            "luma-calendar",
            Some("cal-bbb222"),
        ))
        .expect("luma adapter builds from a cal- id");

        assert_eq!(a.name(), "berlin-events");
        assert_eq!(b.name(), "bonn-events");
    }

    /// A hand-constructed adapter carries no source id and keeps its type name,
    /// which is what the hardcoded pipeline adapters rely on.
    #[test]
    fn an_unconfigured_adapter_keeps_its_type_name() {
        use crate::adapters::luma::LumaAdapter;
        assert_eq!(LumaAdapter::new().name(), "luma");
    }

    fn entry(id: &str, data_class: Option<&str>) -> SourceEntry {
        SourceEntry {
            id: id.into(),
            adapter: "rss".into(),
            path: None,
            url: Some("https://example.test/feed".into()),
            events_glob: None,
            opportunities_glob: None,
            opportunity_type: None,
            profiles_glob: None,
            profile_path: None,
            doc: None,
            enabled: true,
            data_class: data_class.map(Into::into),
        }
    }

    /// The three ways a source declares a class, and the one direction they all
    /// fail in.
    ///
    /// The middle two are the planted inputs. `"public"` is the vocabulary Q72
    /// retired -- deliberately disjoint from `c0`..`c3` so a stale reader fails
    /// loudly rather than passing as the wrong class -- and `"C0"` is the same
    /// literal an operator would most plausibly typo. Neither may be honoured,
    /// and neither may be MORE permissive than saying nothing: both land on the
    /// undeclared default, which is Mine.
    #[test]
    fn a_source_that_declares_nothing_or_nonsense_is_mine_and_never_public() {
        assert_eq!(entry("declared", Some("c0")).resolve().data_class, "c0");
        assert_eq!(entry("strict", Some("c2")).resolve().data_class, "c2");

        assert_eq!(entry("silent", None).resolve().data_class, "c1");
        assert_eq!(entry("blank", Some("   ")).resolve().data_class, "c1");
        assert_eq!(
            entry("retired", Some("public")).resolve().data_class,
            "c1",
            "the pre-Q72 vocabulary is not a class and must not read as one"
        );
        assert_eq!(
            entry("typo", Some("C0")).resolve().data_class,
            "c1",
            "a typo must not be more permissive than silence"
        );
    }

    /// A stored row whose source is not in `sources[]` at all.
    ///
    /// This is the live case, not a hypothetical: a scout measured 310 rows from
    /// four adapters, and the hardcoded ones (luma, meetup, cfp,
    /// euro_hackathons, transit_fare) have no config entry to declare anything.
    /// Every one of them is undeclared, so every one of them is c1.
    #[test]
    fn a_row_whose_source_declared_nothing_fails_closed() {
        let declared = vec![entry("bonn-events", Some("c0")).resolve()];

        assert_eq!(class_for_source(&declared, "bonn-events"), "c0");
        assert_eq!(class_for_source(&declared, "luma"), "c1");
        assert_eq!(class_for_source(&declared, "bonn-event"), "c1");
        assert_eq!(class_for_source(&[], "bonn-events"), "c1");
    }
}
