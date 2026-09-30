//! Public config. No personal value lives here -- it comes from the private
//! overlay at runtime. Resolution order mirrors scouting's config.rs:
//!   1. `$SJEL_COMMS_CONFIG` (explicit override, full path to a JSON file)
//!   2. `$SJEL_PERSONAL_ROOT/config/comms.json` (the overlay)
//!   3. `capabilities/comms/comms.config.json` next to this source
//!      (local, gitignored -- dev fallback)
//!
//! See `comms.config.example.json` for the shape. Every field is optional;
//! the tool runs zero-config against a local Postgres and no rules.

use serde::Deserialize;
use std::path::PathBuf;

use crate::content_item;
use crate::rules::Rule;
use sjel_inference::{InferenceConfig, ResolvedRole};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RelevanceConfig {
    /// Exact Markdown files or non-recursive directories containing TELOS
    /// focus lenses. Personal paths belong in the private overlay.
    pub profile_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QualityFlagConfig {
    /// Passing corpus fixtures retain 39.7–89.6% of their total raw text.
    /// Round outward so the observed fixtures remain inside the review band.
    pub minimum_total_retention_percent: f64,
    pub maximum_total_retention_percent: f64,
    /// Every passing corpus fixture leaks zero judged boilerplate.
    pub maximum_boilerplate_leakage_percent: f64,
    /// The summary retry ledger caps at three; warn on the attempt before it parks.
    pub summary_attempt_warning: i32,
}

impl Default for QualityFlagConfig {
    fn default() -> Self {
        Self {
            minimum_total_retention_percent: 39.0,
            maximum_total_retention_percent: 90.0,
            maximum_boilerplate_leakage_percent: 0.0,
            summary_attempt_warning: 2,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TravelContextConfig {
    /// Trips owns the plan data. Comms reads its existing HTTP contract and
    /// retains only the last bounded context snapshot used for Feed ranking.
    pub enabled: bool,
    pub base_url: String,
    pub max_plans: usize,
    pub timeout_ms: u64,
}

impl Default for TravelContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "http://127.0.0.1:8086".into(),
            max_plans: 20,
            timeout_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CalendarContextConfig {
    /// Calendar owns its entries. Comms reads one over Calendar's existing
    /// `content-item-v2` route to digest it — the same bounded
    /// cross-capability read it already does against Trips, rather than
    /// reaching into a second capability's database schema.
    pub base_url: String,
    pub timeout_ms: u64,
}

impl Default for CalendarContextConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8087".into(),
            timeout_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VaultLinkSourceConfig {
    /// Stable provenance id, for example `scratchpad-to-read`.
    pub id: String,
    /// Exact Markdown file. Comms never walks the vault recursively.
    pub path: String,
    /// Optional exact Markdown heading text. Only links below that heading and
    /// before the next heading of the same or higher level are candidates.
    /// A missing heading yields zero candidates, never a whole-file fallback.
    pub heading: Option<String>,
    pub enabled: bool,
}

impl Default for VaultLinkSourceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: String::new(),
            heading: None,
            enabled: true,
        }
    }
}

/// A declared collector, and what its content is worth.
///
/// Deliberately **not** `#[serde(default)]` at the container level any more.
/// That attribute made every field optional, which is the right call for a
/// window or a language slug and the wrong one for `data_class`: an operator
/// adding a source would have silently got whatever the Default impl said, and
/// the only classification that can be got by silence is the one nobody chose.
/// So the per-field defaults are spelled out individually and `data_class` is
/// left off the list — a source that does not say what it collects fails to
/// load, loudly, before it fetches anything.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedSourceConfig {
    /// Stable provenance id shown in the Feed.
    #[serde(default)]
    pub id: String,
    /// `github-trending` or `arxiv`.
    #[serde(default)]
    pub adapter: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// arXiv search_query. Empty for GitHub Trending.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional GitHub Trending language slug, for example `rust`.
    #[serde(default)]
    pub language: Option<String>,
    /// GitHub Trending window: daily, weekly or monthly.
    #[serde(default)]
    pub since: Option<String>,
    /// Hard per-run bound. Clamped again by the adapter.
    #[serde(default = "default_source_limit")]
    pub limit: usize,
    /// What this collector's content is: `public`, `personal` or `vault`. The
    /// declaration every item it fetches is stored with, and the only way an
    /// item becomes `public` at all. Required — see the type doc.
    pub data_class: String,
}

fn enabled_by_default() -> bool {
    true
}

fn default_source_limit() -> usize {
    10
}

impl Default for FeedSourceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            adapter: String::new(),
            enabled: true,
            query: None,
            language: None,
            since: None,
            limit: default_source_limit(),
            // Serde no longer reaches this, but a programmatic caller does, and
            // it must fail closed the same way an omission in the file does.
            data_class: "c1".into(),
        }
    }
}

fn default_feed_sources() -> Vec<FeedSourceConfig> {
    vec![
        FeedSourceConfig {
            id: "github-trending-daily".into(),
            adapter: "github-trending".into(),
            enabled: true,
            query: None,
            language: None,
            since: Some("daily".into()),
            limit: 12,
            // The Trending page is world-readable and fetched anonymously; no
            // session of the operator's is involved in what it returns.
            data_class: "c0".into(),
        },
        FeedSourceConfig {
            id: "arxiv-ai-recent".into(),
            adapter: "arxiv".into(),
            enabled: true,
            query: Some("cat:cs.AI OR cat:cs.LG OR cat:cs.CL".into()),
            language: None,
            since: None,
            limit: 12,
            // Published preprints. The query itself can be the operator's,
            // which is why a saved private query belongs in the overlay and can
            // declare itself otherwise; the abstracts it returns are not.
            data_class: "c0".into(),
        },
    ]
}

/// The vault root the library projection writes into. Same JSON shape as
/// `trips.json`'s `obsidian` block, deliberately: an operator configuring the
/// second bridge should be writing the block they already wrote for the first.
#[derive(Debug, Clone, Deserialize)]
struct FileObsidian {
    root: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    google_env_path: Option<String>,
    port: Option<u16>,
    api_secret_file: Option<String>,
    dashboard_origin: Option<String>,
    enrichment_drain_minutes: Option<u64>,
    digest_drain_minutes: Option<u64>,
    capacity_alert_after: Option<i32>,
    gmail_maintenance_minutes: Option<u64>,
    inbox_sweep_minutes: Option<u64>,
    inbox_sweep_max_threads: Option<usize>,
    inbox_sweep_quiet_hours: Option<String>,
    #[serde(default)]
    rules: Vec<Rule>,
    keeper_export_dir: Option<String>,
    obsidian: Option<FileObsidian>,
    relevance: Option<RelevanceConfig>,
    travel_context: Option<TravelContextConfig>,
    calendar_context: Option<CalendarContextConfig>,
    #[serde(default)]
    vault_link_sources: Vec<VaultLinkSourceConfig>,
    feed_sources: Option<Vec<FeedSourceConfig>>,
    quality_flags: Option<QualityFlagConfig>,
    #[serde(default)]
    ingest_allowed_origins: Vec<String>,
    /// Absent in the overlay today, and absent means the model rung stays in
    /// shadow. See [`MailModelConfig`].
    mail_model: Option<MailModelConfig>,
}

/// What the local model rung is allowed to do on this machine.
///
/// Absent in the overlay means shadow only: the pass writes verdicts and
/// changes no category. The flip to live is one key a human sets after reading
/// the report, not a code deploy and not a request flag a script could set by
/// accident.
///
/// Read through `mail_model::apply_allowed`, which takes this section rather
/// than the whole `Config`, so the refusal is a pure function and its test does
/// not depend on what this machine's overlay happens to hold.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MailModelConfig {
    /// Whether a verdict may move a category at all.
    pub apply: bool,
    /// The blunt floor below which a disagreement is not written. The model's
    /// confidence is self-reported and uncalibrated: this is a policy
    /// threshold, not a probability, and the corpus is what sets it.
    ///
    /// Zero is not a floor, so `apply = true` beside a zero here is refused by
    /// `mail_model::apply_allowed` rather than silently writing everything.
    pub min_confidence_bp: u32,
    /// How many threads one pass may act on when the caller names no limit.
    /// Read through `mail_model::pass_limit`, which both the CLI and
    /// `POST /triage/classify/refresh` go through.
    pub limit: usize,
}

impl Default for MailModelConfig {
    fn default() -> Self {
        Self {
            apply: false,
            min_confidence_bp: 0,
            limit: 200,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The one shared SQLite file, under the table prefix `comms` (PRD Q45).
    /// Resolved by `sjel_config::database_path`: `SJEL_DB_PATH`, else
    /// `<overlay>/data/axon/axon.db`. A deployment fact, not a capability one:
    /// a file per capability would drop the cross-capability joins the shared
    /// instance existed for, so `SJEL_COMMS_DATABASE_URL` and a `database_url`
    /// in `comms.json` are both gone.
    pub database_path: PathBuf,
    /// Path to the `KEY=value` env file holding GOOGLE_CLIENT_ID /
    /// GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN. Default
    /// `$SJEL_PERSONAL_ROOT/config/comms.env`.
    pub google_env_path: PathBuf,
    /// HTTP port for `comms-server`.
    pub port: u16,
    /// Resolved shared secret token for mutating HTTP endpoints (`POST /ingest` etc).
    pub api_secret: Option<String>,
    /// Allowed CORS origin for cross-origin requests (defaults to `http://127.0.0.1:47117`).
    pub dashboard_origin: String,
    /// How often comms-server drains the summary backlog, in minutes. `0`
    /// disables the pass entirely, which is the setting for a machine that runs
    /// `comms summarize --pending` from somewhere else. Without a drain, items
    /// ingested while the inference server was unreachable stay empty forever —
    /// that is how 36 of 39 items ended up without a summary (#74).
    pub enrichment_drain_minutes: u64,
    /// How often comms-server retries digests that failed retryably, in
    /// minutes. `0` disables the pass, leaving digests manual-press-only.
    ///
    /// Separate from `enrichment_drain_minutes` because the two write different
    /// things — that one fills `feed_items.summary`, this one fills
    /// `content_digests` — and a machine may reasonably want one without the
    /// other. Bounded by the same ledger: three attempts, then the row rests.
    pub digest_drain_minutes: u64,
    /// How many *consecutive* capacity aborts from the local inference server
    /// before the automatic passes raise it rather than absorbing it. `0`
    /// disables the alert.
    ///
    /// Three because that is a full period of the shipped 15-minute drains plus
    /// change: one abort is another prefill in flight and self-heals, two is
    /// a busy half-hour, three in a row means the row is not going to be
    /// written by waiting. Owned here rather than in `capacity.rs` so the
    /// number lives with every other cadence this server runs on.
    pub capacity_alert_after: i32,
    /// Retry durable Gmail actions and reconcile labels on this interval. `0`
    /// disables automatic maintenance; manual reconciliation remains available.
    pub gmail_maintenance_minutes: u64,
    /// Pull new inbox proposals on this interval. **`0`, disabled, is the
    /// default and the shipped value.** An unattended job that reads a mailbox
    /// is opt-in per machine, never something a fresh clone starts doing.
    pub inbox_sweep_minutes: u64,
    /// Newest-N threads per scheduled pass. A bound, not a cursor: a cursor
    /// that advances every run walks backwards through the whole mailbox over
    /// time, which is the unbounded rescan this schedule exists to avoid.
    /// Re-reading the newest page is free because upserts key on thread id.
    pub inbox_sweep_max_threads: usize,
    /// `"22-7"` — local hours `[start, end)` in which the schedule holds off.
    /// `None` means no quiet window. Manual sweeps ignore it: the point is to
    /// stop unattended traffic, not to lock the operator out of their own tool.
    pub inbox_sweep_quiet_hours: Option<(u32, u32)>,
    /// Shared, machine-resolved inference roles. Backend URLs, models and key
    /// references never belong to Comms configuration.
    pub inference: InferenceConfig,
    /// Config-driven classification rules (first match wins, before built-in
    /// heuristics). Empty = built-in heuristics only.
    pub rules: Vec<Rule>,
    /// Optional directory to export distilled keeper notes into on `comms keep`.
    pub keeper_export_dir: Option<PathBuf>,
    /// The Obsidian vault the feed library projects into (PRD Q49). `None`
    /// leaves the bridge off, which is what an unconfigured host wants: no
    /// vault, no writes, and the rows are still the record.
    ///
    /// This *is* a vault root, unlike `vault_link_sources` two fields down, and
    /// the difference is direction. That field refuses a root because comms
    /// reads there and a recursive read would sweep up Scratchpad credentials.
    /// This one is a write destination, and what bounds it is not the root but
    /// `projection::DIR`: one folder, files carrying comms' own header, and
    /// `MarkdownRoot` proving every path stays inside the root before a byte is
    /// written.
    pub obsidian_root: Option<PathBuf>,
    pub relevance: RelevanceConfig,
    pub travel_context: TravelContextConfig,
    pub calendar_context: CalendarContextConfig,
    /// Explicit Markdown link sources. This is intentionally not a vault root:
    /// Scratchpad and other notes can contain credentials and admin URLs.
    pub vault_link_sources: Vec<VaultLinkSourceConfig>,
    /// General awareness sources. `None` in a file uses the public, non-personal
    /// defaults; an explicit empty array disables them all.
    pub feed_sources: Vec<FeedSourceConfig>,
    pub quality_flags: QualityFlagConfig,
    /// `None` until the overlay declares it, and `None` refuses every apply.
    pub mail_model: Option<MailModelConfig>,
}

// One implementation, in libs/sjel-config, re-exported under the name this
// module's call sites already use. comms was the last capability still carrying
// its own copies of these helpers.
pub(crate) use sjel_config::expand_tilde;

/// Resolve an API-key reference without ever storing or logging its value.
/// JSON files use `.auth.api_key` (the oMLX settings shape); non-JSON files use
/// their trimmed contents. Parsed JSON without that field deliberately has no
/// raw-content fallback.
///
/// The reader itself moved to `libs/sjel-server` when the inbound gate did:
/// both this file and `<overlay>/config/deployment.env`'s
/// `SJEL_INBOUND_TOKEN_FILE` name a file holding the same kind of value, and two
/// readers of one shape is the drift the move removes.
pub(crate) fn api_key_from_file(path: Option<&str>) -> Option<String> {
    sjel_server::token_from_file(&expand_tilde(path?))
}

/// `"22-7"` -> `(22, 7)`. Returns `None` for anything it does not fully
/// understand, and `None` means "no quiet window" — a misconfigured string
/// must not silently become a window that suppresses every run instead.
fn parse_quiet_hours(value: &str) -> Option<(u32, u32)> {
    let (start, end) = value.trim().split_once('-')?;
    let start: u32 = start.trim().parse().ok()?;
    let end: u32 = end.trim().parse().ok()?;
    (start < 24 && end < 24 && start != end).then_some((start, end))
}

fn config_path() -> PathBuf {
    if let Ok(p) = sjel_config::env_var("SJEL_COMMS_CONFIG") {
        return expand_tilde(&p);
    }
    if let Some(p) = sjel_config::overlay_config("comms.json") {
        return p;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("comms.config.json")
}

fn default_google_env_path() -> PathBuf {
    sjel_config::overlay_config("comms.env").unwrap_or_else(|| PathBuf::from("comms.env"))
}

/// The one normaliser, and it is the guard's own: the configured entries and the
/// URL being checked have to be compared as the same shape or the comparison is a
/// coin toss. [`ingest_allowed_origins`] calls it on every entry;
/// `sjel_http::guard::origin_is_allowed` calls it on the URL.
pub(crate) use sjel_http::guard::normalize_origin;

/// Origins `POST /ingest` may fetch even though they resolve to an address
/// inside this machine or this network (Q74).
///
/// **Empty by default, and that is the shipped value.** No real machine needs
/// an entry: the guard exists because an ingested link could otherwise drive a
/// loopback-bound Axon API. The demo is the one caller that legitimately points
/// Comms at loopback — `tools/demo-up` writes `demo/demo.toml`'s `[demo] origin`
/// into `demo/overlay/config/comms.json` before starting the stack, and that
/// origin is a synthetic HTTP server on a port no capability occupies.
///
/// An entry is an origin, not a host: `http://127.0.0.1:8099` clears the demo
/// origin and still refuses `http://127.0.0.1:8086/api/plans`. A host-only
/// allowlist would hand back the whole loopback surface.
///
/// A free function rather than a [`Config`] field because its one reader,
/// `media::check_destination`, holds no `Config` and must not build one:
/// `Config::load` reads the API-secret file, and a URL that is about to be
/// refused has no business touching a secret. It is called only after the
/// address check has already failed, so an ordinary fetch never reads it.
pub fn ingest_allowed_origins() -> Vec<String> {
    load_file_config()
        .ingest_allowed_origins
        .iter()
        .filter_map(|raw| {
            let origin = normalize_origin(raw);
            if origin.is_none() {
                eprintln!(
                    "comms: ingest_allowed_origins entry {raw:?} is not an absolute \
                     http(s) origin -- ignoring it"
                );
            }
            origin
        })
        .collect()
}

fn load_file_config() -> FileConfig {
    let path = config_path();
    if !path.is_file() {
        return FileConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_else(|e| {
            eprintln!("warning: could not parse {path:?}: {e} -- using defaults");
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

impl Config {
    pub fn load() -> Self {
        let file = load_file_config();

        let google_env_path = file
            .google_env_path
            .map(|p| expand_tilde(&p))
            .unwrap_or_else(default_google_env_path);

        // The runner's port contract, resolved in one place for every capability.
        // No capability-specific escape-hatch env var here: comms never had one.
        let port = sjel_config::resolve_port(None, file.port, 8083);
        let api_secret = api_key_from_file(file.api_secret_file.as_deref());
        let dashboard_origin = file
            .dashboard_origin
            .unwrap_or_else(|| "http://127.0.0.1:47117".into());
        let enrichment_drain_minutes = file.enrichment_drain_minutes.unwrap_or(15);
        let digest_drain_minutes = file.digest_drain_minutes.unwrap_or(15);
        // Clamped at zero rather than rejected: a negative threshold is a typo,
        // and the safe reading of a typo is "off", not "alert on everything".
        let capacity_alert_after = file.capacity_alert_after.unwrap_or(3).max(0);
        let gmail_maintenance_minutes = file.gmail_maintenance_minutes.unwrap_or(15);
        let inbox_sweep_minutes = file.inbox_sweep_minutes.unwrap_or(0);
        let inbox_sweep_max_threads = file.inbox_sweep_max_threads.unwrap_or(25).clamp(1, 100);
        let inbox_sweep_quiet_hours = file
            .inbox_sweep_quiet_hours
            .as_deref()
            .and_then(parse_quiet_hours);
        let inference = InferenceConfig::load(sjel_config::overlay_config);
        let keeper_export_dir = file.keeper_export_dir.map(|p| expand_tilde(&p));
        // An empty root reads as "not configured", not as the current directory.
        // `comms.config.example.json` ships every optional string blank, so a copied
        // template must leave the bridge off rather than log a failed export after
        // every mutation.
        let obsidian_root = file
            .obsidian
            .map(|o| o.root)
            .filter(|root| !root.trim().is_empty())
            .map(|root| expand_tilde(&root));
        let mut relevance = file.relevance.unwrap_or_default();
        relevance.profile_paths = relevance
            .profile_paths
            .into_iter()
            .map(|path| expand_tilde(&path).to_string_lossy().into_owned())
            .collect();
        let vault_link_sources = file
            .vault_link_sources
            .into_iter()
            .map(|mut source| {
                source.path = expand_tilde(&source.path).to_string_lossy().into_owned();
                source
            })
            .collect();
        // A declared class outside the vocabulary is a typo, and a typo must not
        // be more permissive than saying nothing. Refused down to `c1` and
        // reported, rather than passed through to a CHECK constraint that would
        // reject the item hours later at ingest.
        let feed_sources = file
            .feed_sources
            .unwrap_or_else(default_feed_sources)
            .into_iter()
            .map(|mut source| {
                if !content_item::valid(&source.data_class) {
                    eprintln!(
                        "comms: feed source '{}' declares an unknown data_class '{}'; \
                         treating it as c1",
                        source.id, source.data_class
                    );
                    source.data_class = "c1".into();
                }
                source
            })
            .collect();
        let travel_context = file.travel_context.unwrap_or_default();
        let calendar_context = file.calendar_context.unwrap_or_default();
        let quality_flags = file.quality_flags.unwrap_or_default();
        // Deliberately not `unwrap_or_default()`: "the operator has not decided"
        // and "the operator decided no" have to stay distinguishable, because
        // the first is what the 409 names.
        let mail_model = file.mail_model;

        Self {
            database_path: sjel_config::database_path(),
            google_env_path,
            port,
            api_secret,
            dashboard_origin,
            enrichment_drain_minutes,
            digest_drain_minutes,
            capacity_alert_after,
            gmail_maintenance_minutes,
            inbox_sweep_minutes,
            inbox_sweep_max_threads,
            inbox_sweep_quiet_hours,
            inference,
            rules: file.rules,
            keeper_export_dir,
            obsidian_root,
            relevance,
            travel_context,
            calendar_context,
            vault_link_sources,
            feed_sources,
            quality_flags,
            mail_model,
        }
    }

    pub fn embedding_role(&self) -> Option<ResolvedRole> {
        self.inference.role("embedding")
    }

    pub fn reranking_role(&self) -> Option<ResolvedRole> {
        self.inference.role("reranking")
    }

    pub fn summarization_role(&self) -> Option<ResolvedRole> {
        self.inference.role("summarization")
    }

    /// A smaller, faster local model for the cheap rungs, if this machine has
    /// one configured.
    ///
    /// Optional by design: a machine with only `summarization` behaves exactly
    /// as it did before this existed. Where it earns its place is a host with a
    /// second local model that is quick but small — Apple's on-device model is
    /// 4,096 tokens and answers a Brief digest in about two seconds against
    /// twelve to twenty for the 26B — because moving the short rungs onto it
    /// takes them off the shared GPU budget entirely.
    ///
    /// Resolved by `crate::quiet`, which is where "the light rung" is defined,
    /// because every unattended pass on this machine is now pinned to it. This
    /// method is the convenience view for a caller that already holds a Config.
    pub fn light_summarization_role(&self) -> Option<ResolvedRole> {
        crate::quiet::light_role(&self.inference)
    }

    /// This machine's config with a chosen inference roster substituted in.
    ///
    /// Built by overriding the loaded one rather than by listing twenty
    /// defaults, so a new field cannot silently arrive here as a zero value
    /// that no real machine would have.
    #[cfg(test)]
    pub(crate) fn with_inference(inference: InferenceConfig) -> Self {
        Self {
            inference,
            ..Self::load()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fail-closed rule as a type error rather than a runtime default. A
    /// source that does not say what it collects must not load at all: the
    /// alternative is a collector quietly inheriting whatever the Default impl
    /// happens to say, which is exactly how the feed ended up with a `public`
    /// nobody had chosen.
    #[test]
    fn a_feed_source_that_declares_no_data_class_fails_to_deserialize() {
        let without = r#"{"id":"x","adapter":"arxiv","limit":5}"#;
        let error = serde_json::from_str::<FeedSourceConfig>(without)
            .expect_err("a source with no declared class must not deserialize");
        assert!(
            error.to_string().contains("data_class"),
            "the error must name the missing field, got: {error}"
        );

        let with = r#"{"id":"x","adapter":"arxiv","limit":5,"data_class":"c0"}"#;
        let parsed: FeedSourceConfig =
            serde_json::from_str(with).expect("a declared source loads normally");
        assert_eq!(parsed.data_class, "c0");
        assert!(parsed.enabled, "the other per-field defaults still apply");
    }

    /// The shipped defaults are declarations, not omissions — if this ever
    /// reads `c1` it means someone dropped the declaration and the
    /// general-awareness feed silently stopped being cloud-eligible.
    #[test]
    fn every_shipped_collector_declares_its_class() {
        for source in default_feed_sources() {
            assert!(
                content_item::valid(&source.data_class),
                "{} declares an unknown class",
                source.id
            );
        }
    }

    #[test]
    fn expand_tilde_uses_home() {
        std::env::set_var("HOME", "/tmp/fake-home");
        assert_eq!(expand_tilde("~/foo"), PathBuf::from("/tmp/fake-home/foo"));
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[test]
    fn quiet_hours_parse_or_decline() {
        assert_eq!(parse_quiet_hours("22-7"), Some((22, 7)));
        assert_eq!(parse_quiet_hours(" 0 - 6 "), Some((0, 6)));
        // Anything unparseable means no window, never an all-day one — the
        // failure mode of the opposite choice is a schedule that silently
        // never runs and looks identical to a broken connector.
        for bad in ["", "22", "22-24", "9-9", "night", "22-7-3"] {
            assert_eq!(parse_quiet_hours(bad), None, "{bad} should not parse");
        }
    }

    /// The store path is a deployment fact, not a capability one: a
    /// `database_url` left in `comms.json` must not move comms off the shared
    /// file, because places joins mail against it.
    #[test]
    fn the_store_path_comes_from_the_deployment_not_from_comms_json() {
        // Cleared first: with the host's overlay vars in scope, the path both
        // sides resolve is the deployed axon.db. No test may name the live
        // store, even to compare a `PathBuf`.
        let _config = EnvGuard::take("SJEL_COMMS_CONFIG");
        let _overlay = EnvGuard::take("SJEL_PERSONAL_ROOT");
        let _explicit = EnvGuard::take("SJEL_DB_PATH");
        assert_eq!(Config::load().database_path, sjel_config::database_path());
    }

    #[test]
    fn relevance_config_only_owns_profile_paths() {
        let relevance = RelevanceConfig::default();
        assert!(relevance.profile_paths.is_empty());
    }

    /// Restores an env var on drop. Rust runs a crate's tests as threads of ONE
    /// process, so `remove_var` here is not local to this test: unrestored, it
    /// left every later store test resolving the fallback connection string
    /// instead of the overlay's real one, and eight of them failed against a
    /// perfectly healthy Postgres.
    struct EnvGuard(Vec<(String, Option<String>)>);

    impl EnvGuard {
        /// Clears the setting, so a value the operator's shell still exports cannot stand in
        /// for it.
        fn take(key: &'static str) -> Self {
            let names = std::iter::once(key.to_string());
            Self(
                names
                    .map(|name| {
                        let previous = std::env::var(&name).ok();
                        std::env::remove_var(&name);
                        (name, previous)
                    })
                    .collect(),
            )
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, previous) in self.0.drain(..).rev() {
                match previous {
                    Some(value) => std::env::set_var(&name, value),
                    None => std::env::remove_var(&name),
                }
            }
        }
    }

    #[test]
    fn load_with_no_env_and_no_file_uses_defaults() {
        let _config = EnvGuard::take("SJEL_COMMS_CONFIG");
        let _overlay = EnvGuard::take("SJEL_PERSONAL_ROOT");
        let cfg = Config::load();
        assert_eq!(cfg.port, 8083);
        assert!(cfg.rules.is_empty());
        assert!(cfg.inference.roles.is_empty());
        assert!(cfg.relevance.profile_paths.is_empty());
        assert!(cfg.vault_link_sources.is_empty());
        assert_eq!(cfg.feed_sources.len(), 2);
        assert_eq!(cfg.quality_flags.minimum_total_retention_percent, 39.0);
        assert_eq!(cfg.quality_flags.maximum_total_retention_percent, 90.0);
        assert_eq!(cfg.quality_flags.maximum_boilerplate_leakage_percent, 0.0);
        assert_eq!(cfg.quality_flags.summary_attempt_warning, 2);
    }
}
