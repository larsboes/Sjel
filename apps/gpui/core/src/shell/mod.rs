//! Client for the Sjel shell (`capabilities/sjel-status`), authenticated with the inbound token.
//!
//! The same contracts and token as the menu-bar app (`apps/mac/Sources/SjelRelay`): the token
//! comes from the login Keychain item that `tools/setup-inbound-auth.sh` writes, through
//! `/usr/bin/security`, which that script puts on the item's access list.

use std::fmt;
use std::future::Future;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use reqwest::{header, Method, StatusCode, Url};

use crate::capture::{self, Capture, Route};
use crate::mail::{BulkResult, MailAction, MailProposal};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use futures::channel::mpsc::UnboundedReceiver;

/// The default shell endpoint, from `capabilities/sjel-status/service.toml`.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8082";
const KEYCHAIN_SERVICE: &str = "sjel-inbound-token";
const EVENTS_PATH: &str = "/api/sjel-status/events";

/// The ceiling on one original, matching `MAX_BYTES` in `capabilities/documents/src/server.rs`.
/// A reader holds one document at a time, and this is the most it can be asked to hold; the two
/// numbers are one edit apart on purpose.
const MAX_DOCUMENT_BYTES: usize = 128 * 1024 * 1024;

pub use status_api::{
    default_kept_running, AgentCall, AgentGate, AgentView, Approval, CapabilityHealth, Liveness,
    NodeHealth, RegistryEntry, ROUTE_STATUS_AGENT, ROUTE_STATUS_AGENT_MODE, ROUTE_STATUS_BACKUPS,
    ROUTE_STATUS_BACKUP_POLICY, ROUTE_STATUS_BACKUP_RUNS, ROUTE_STATUS_BACKUP_TARGETS,
    ROUTE_STATUS_BACKUP_VERIFY, ROUTE_STATUS_CAPABILITIES, ROUTE_STATUS_CREDENTIALS,
    ROUTE_STATUS_CV, ROUTE_STATUS_EVENTS, ROUTE_STATUS_HEALTH, ROUTE_STATUS_HOST_WATCH,
    ROUTE_STATUS_LAN, ROUTE_STATUS_LINKS, ROUTE_STATUS_NOW, ROUTE_STATUS_PACKS, ROUTE_STATUS_REPOS,
    ROUTE_STATUS_ROUTES, ROUTE_STATUS_RUNTIME, ROUTE_STATUS_SEARCH, ROUTE_STATUS_SELF,
    ROUTE_STATUS_SETUP, ROUTE_STATUS_SETUP_APPLY, ROUTE_STATUS_SETUP_VERIFY, ROUTE_STATUS_STORAGE,
    ROUTE_STATUS_STORAGE_HISTORY, ROUTE_STATUS_UPDATES, ROUTE_STATUS_UPDATES_APPLY,
};

/// `GET /calendar/api/entries` (`Entry` in `capabilities/calendar/src/store.rs`). Home's
pub use calendar_api::{
    CalendarEntry, CalendarEntryDetail, ExportOptIn as CalendarGoogleExportOptIn,
    ExportOutcome as CalendarGoogleExportOutcome, ExportReport as CalendarGoogleExportReport,
    ExportSkipped as CalendarGoogleExportSkipped,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct CalendarGoogleExportOptOut {
    opted_out: bool,
}

pub use finance_api::{
    AllocateExpenseRequest, BalanceCoverage, BalanceKind, CandidateState, FinanceCandidate,
    FinanceCsvMappingProfile, FinanceCsvPreview, FinanceDashboardProjection,
    FinanceDashboardSummary, FinanceDecimalValue, FinancePortfolio, FinancePosition,
    FinanceTransaction, FinanceTrendPoint, LinkReimbursementRequest, ManualBalance,
    ManualBalanceSnapshot, ManualBalanceUpdate, PricePointRecord, ReclassifyBatchRequest,
    ReclassifyItem, ReconcileTransferRequest, ReviewCandidateRequest, SharedExpenseSummary,
    StateChangeRecord, SubscriptionRecord, SubscriptionState, TrackedNetWorth, TransactionKind,
};

pub use entities_api::{DuplicatePair, DuplicateSide, EntityFact, EntityField, EntityRecord};

pub use import_api::{
    ImportJob, ImportPlan, ImportRoute, ImportSkip, ImportStatus, ImportStatusRow,
};

#[derive(Deserialize)]
struct ImportJobEnvelope {
    job: Option<ImportJob>,
}

pub use software_api::{SoftwareApp, SoftwareApps};

pub use status_api::{BackupRun, BackupStatus, Backups};

/// `GET /projects/api/projects` (`ProjectSummary` in `capabilities/projects/src/server.rs`).
pub use projects_api::{
    decode_item_ref, inventory_reference, project_ref, status_closes, GlobalTask,
    InventoryIdentity, ProjectInventoryLink, ProjectKind, ProjectLinkRecord, ProjectMailDecision,
    ProjectMailPage, ProjectMailProposal, ProjectMailTarget, ProjectRecord, ProjectSummary,
    RenamedRecord, TASK_STATUSES,
};

pub use status_api::ServerHit;

/// One note across projects (`GET /projects/api/records`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GlobalNote {
    pub path: String,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
}

pub use documents_api::{
    ClassificationProposal, DocumentBoundaryEvidence, DocumentDetail, DocumentExtractedPage,
    DocumentPage, DocumentPageRange, DocumentProposalRecord, DocumentSplitChildInput,
    DocumentSplitRequest, DocumentSplitSuggestion, DocumentSummary, DocumentUpload,
    DuplicateCandidate, DuplicateCandidateKind, DuplicateCandidatePage, DuplicateDecision,
    DuplicateDecisionAction, DuplicateDecisionResponse, DuplicatePageRange, DuplicatePartRelation,
    DuplicateSourceLocation, ProposalAbstentionReason, ProposalEvidence, ProposalEvidenceSource,
    ProposalInvalidReason, ProposalOutcome, ProposalReview, ProposalReviewEvent, ProposalSource,
};

pub use feed_api::{
    data_class_lowering_needs_rationale, data_class_rank, CloudDerivativePreview,
    CloudProviderOption, CloudRedactionFinding, FeedCalendarContent, FeedCloudProcessingState,
    FeedContentDetail, FeedDataClass, FeedDetail, FeedDigest, FeedEntry, FeedEvaluation, FeedList,
    FeedQualityFlag, FeedQualityRefresh, FeedRelevance, FeedSurface,
    ROUTE_COMMS_CONTENT_CLOUD_PROVIDERS, ROUTE_COMMS_FEED, ROUTE_COMMS_FEED_QUALITY,
    ROUTE_COMMS_FEED_QUALITY_REFRESH,
};

/// A feed or document id: 64 hex characters, a SHA-256. Anything else never reaches a URL.
fn is_sha256(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The reader route for one document's original, or `None` for an id that is not a digest. The
/// check runs before any request, so a malformed id never reaches the capability.
fn document_original_path(id: &str) -> Option<String> {
    is_sha256(id).then(|| format!("/documents/api/documents/{id}/original"))
}

fn too_large() -> ShellError {
    ShellError::Refused(413, Some("That document is too large to open here.".into()))
}

/// The body of one shell request. An upload is raw bytes and must not carry a JSON content type:
/// the capability takes the filename from the query and reads the body as the file itself.
enum Payload {
    Json(serde_json::Value),
    Bytes(Vec<u8>),
}

/// Metadata returned by `POST /music/api/probe`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct MusicProbe {
    pub title: String,
    pub uploader: String,
    pub duration_hms: String,
    #[serde(default)]
    pub chapters: Vec<MusicChapter>,
    #[serde(default)]
    pub tracklist_from_chapters: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MusicChapter {
    pub start: u64,
    pub title: String,
}

/// Request and response contract for a music build.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicJob {
    pub id: String,
    pub album: String,
    pub state: String,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub outdir: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MusicBuild {
    pub url: String,
    /// `"song"` imports one track; empty or `"album"` cuts a tracklist.
    pub mode: String,
    pub album: String,
    pub artist: String,
    pub year: String,
    pub tracklist: String,
    /// Song mode only: the track title and an optional `M:SS` range.
    pub title: String,
    pub start: String,
    pub end: String,
}

/// The soundscape playback state (`GET /soundscape/api/soundscape/state`).
pub use soundscape_api::{
    default_energy, default_volume, deserialize_session_patch, Change as SoundscapeChange,
    Claim as SoundscapeClaim, Host as SoundscapeHost, LayerName as SoundscapeLayerName,
    Layers as SoundscapeLayers, ParamName as SoundscapeParamName, Params as SoundscapeParams,
    Patch as SoundscapePatch, Preset as SoundscapePreset, Release as SoundscapeRelease,
    Scale as SoundscapeScale, Session as SoundscapeSession, SoundscapeState,
    StateView as SoundscapeStateView, HOST_TTL, MAX_SESSION_MS, PRESETS as SOUNDSCAPE_PRESETS,
    PROXY_ROUTE_SOUNDSCAPE_STATE, ROUTE_SOUNDSCAPE_HEALTH, ROUTE_SOUNDSCAPE_HOST_CLAIM,
    ROUTE_SOUNDSCAPE_HOST_RELEASE, ROUTE_SOUNDSCAPE_STATE, ROUTE_SOUNDSCAPE_STREAM,
    SCENARIOS as SOUNDSCAPE_SCENARIOS,
};

pub use assistant_api::{
    AskEvent, AskProvider, AskReply, AskRequest, AskResponse, AskTurn, GenerateRequest,
    GenerateResponse, ProviderKeyStatus, SteerRequest, PROXY_ROUTE_ASSISTANT_ASK,
    PROXY_ROUTE_ASSISTANT_ASK_STREAM, PROXY_ROUTE_ASSISTANT_PROVIDERS, ROUTE_ASK,
    ROUTE_ASK_PROVIDERS, ROUTE_ASK_STREAM, ROUTE_GENERATE,
};

pub use transit_api::{
    Confidence as TransitConfidence, ContractBoundary as TransitContractBoundary,
    Journey as TransitJourney, JourneyRanking as TransitJourneyRanking,
    JourneyReliability as TransitJourneyReliability, JourneyWeights as TransitJourneyWeights,
    Leg as TransitLeg, RankFactor as TransitRankFactor, Ranking as TransitRanking,
    Reliability as TransitReliability, SplitConfidence as TransitSplitConfidence,
    SplitResult as TransitSplitResult, SplitSegment as TransitSplitSegment,
    Station as TransitStation, TrainMatch as TransitTrainMatch, PRIORITIES as TRANSIT_PRIORITIES,
    PRIORITY_WEIGHTS as TRANSIT_PRIORITY_WEIGHTS, PROXY_ROUTE_TRANSIT_HEALTH,
    PROXY_ROUTE_TRANSIT_SEARCH, PROXY_ROUTE_TRANSIT_SPLIT, PROXY_ROUTE_TRANSIT_SUGGEST,
    PROXY_ROUTE_TRANSIT_TRIPS, ROUTE_SEARCH as ROUTE_TRANSIT_SEARCH,
    ROUTE_SPLIT as ROUTE_TRANSIT_SPLIT, ROUTE_SUGGEST as ROUTE_TRANSIT_SUGGEST,
    ROUTE_TRIPS as ROUTE_TRANSIT_TRIPS,
};

pub use distribution_api::{
    host_platform, valid_feature_metadata, valid_package_identifier, valid_release_version,
    valid_sha256, AvailableFeature, FeatureMetadata as InstalledFeatureMetadata, GithubRelease,
    GithubReleaseAsset, InstalledFeature, InstalledPackage, Package, PackageFile, PackageReceipt,
    ReleaseAsset, ReleaseCapability, ReleaseCatalogue, ReleaseSource, Runtime as RuntimeIdentity,
    CATALOGUE_LIMIT, MAX_MANIFEST, MAX_PACKAGE, OFFICIAL_RELEASE_URL as RELEASE_URL,
};

pub use devices_api::{DevicesResp, JoinsResp};

/// Reads the official latest release and checks its catalogue before exposing feature entries.
pub async fn feature_catalogue() -> ShellResult<Vec<AvailableFeature>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| ShellError::PackageManager("could not start catalog client".into()))?;
    let release_response = client
        .get(RELEASE_URL)
        .header(header::USER_AGENT, "Sjel-Desktop")
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| {
            ShellError::PackageManager(format!("official catalog request failed: {error}"))
        })?;
    if !release_response.status().is_success() {
        return Err(ShellError::PackageManager(format!(
            "official catalog request returned HTTP {}",
            release_response.status()
        )));
    }
    let release: GithubRelease = release_response
        .json()
        .await
        .map_err(|_| ShellError::Unreadable)?;
    if !valid_release_version(&release.tag_name) {
        return Err(ShellError::PackageManager(
            "latest official release has an invalid tag".into(),
        ));
    }
    let catalogue_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "release.json")
        .ok_or_else(|| {
            ShellError::PackageManager("latest release has no package catalog".into())
        })?;
    validate_release_asset_url(
        &catalogue_asset.browser_download_url,
        &release.tag_name,
        "release.json",
    )?;
    let catalogue_response = client
        .get(&catalogue_asset.browser_download_url)
        .header(header::USER_AGENT, "Sjel-Desktop")
        .send()
        .await
        .map_err(|error| {
            ShellError::PackageManager(format!("official catalog download failed: {error}"))
        })?;
    if !catalogue_response.status().is_success()
        || catalogue_response
            .content_length()
            .is_some_and(|size| size > CATALOGUE_LIMIT as u64)
    {
        return Err(ShellError::PackageManager(
            "official catalog download is unavailable or too large".into(),
        ));
    }
    use futures::StreamExt as _;
    let mut stream = catalogue_response.bytes_stream();
    let mut catalogue_bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ShellError::Unreadable)?;
        if catalogue_bytes.len().saturating_add(chunk.len()) > CATALOGUE_LIMIT {
            return Err(ShellError::PackageManager(
                "official catalog exceeds the size limit".into(),
            ));
        }
        catalogue_bytes.extend_from_slice(&chunk);
    }
    let catalogue: ReleaseCatalogue =
        serde_json::from_slice(&catalogue_bytes).map_err(|_| ShellError::Unreadable)?;
    if catalogue.schema != 1
        || catalogue.version != release.tag_name
        || catalogue.source.repository != "https://github.com/larsboes/Sjel"
        || catalogue.source.commit.len() != 40
        || !catalogue
            .source
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || catalogue.source.dirty
        || !catalogue.source.rebuilt
    {
        return Err(ShellError::PackageManager(
            "official catalog failed its release identity checks".into(),
        ));
    }

    let host_platform = host_platform();
    let runtime = runtime_identity();
    let mut features = Vec::new();
    let mut package_names = std::collections::BTreeSet::new();
    for package in catalogue
        .capabilities
        .into_iter()
        .filter(|package| package.platform == host_platform)
    {
        if package.version != catalogue.version
            || !valid_package_identifier(&package.name)
            || !package_names.insert(package.name.clone())
            || package
                .requires
                .iter()
                .any(|name| !valid_package_identifier(name) || name == &package.name)
            || package
                .requires
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != package.requires.len()
            || !valid_feature_metadata(&package.feature)
            || package.bytes == 0
            || package.bytes > 1_073_741_824
            || !valid_sha256(&package.sha256)
            || package.asset
                != format!(
                    "sjel-{}-{}-{}.tar.gz",
                    package.name, package.version, package.platform
                )
        {
            return Err(ShellError::PackageManager(format!(
                "invalid catalog entry for {}",
                package.name
            )));
        }
        let matching_assets: Vec<_> = catalogue
            .assets
            .iter()
            .filter(|asset| asset.name == package.asset)
            .collect();
        let Some(asset) = matching_assets.first() else {
            return Err(ShellError::PackageManager(format!(
                "catalog asset missing for {}",
                package.name
            )));
        };
        if matching_assets.len() != 1
            || asset.bytes != package.bytes
            || asset.sha256 != package.sha256
            || asset
                .platform
                .as_deref()
                .is_some_and(|platform| platform != package.platform)
        {
            return Err(ShellError::PackageManager(format!(
                "catalog asset does not match {}",
                package.name
            )));
        }
        let matching_release_assets: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| asset.name == package.asset)
            .collect();
        let release_asset = matching_release_assets.first().copied().ok_or_else(|| {
            ShellError::PackageManager(format!("release download missing for {}", package.name))
        })?;
        if matching_release_assets.len() != 1 {
            return Err(ShellError::PackageManager(format!(
                "release has ambiguous downloads for {}",
                package.name
            )));
        }
        validate_release_asset_url(
            &release_asset.browser_download_url,
            &release.tag_name,
            &package.asset,
        )?;
        let reason = if !catalogue.friend_ready {
            Some("This release is not cleared for feature installation.".to_owned())
        } else if runtime.as_ref().map(|identity| identity.0.as_str())
            != Some(catalogue.version.as_str())
        {
            Some("Place the matching Sjel runtime before installing this feature.".to_owned())
        } else if runtime.as_ref().map(|identity| identity.1.as_str())
            != Some(host_platform.as_str())
        {
            Some("The placed Sjel runtime does not match this device.".to_owned())
        } else if !installer_available() {
            Some("The placed runtime package installer is unavailable.".to_owned())
        } else {
            None
        };
        features.push(AvailableFeature {
            name: package.name,
            version: package.version,
            platform: package.platform,
            requires: package.requires,
            feature: package.feature,
            asset: package.asset,
            bytes: package.bytes,
            sha256: package.sha256,
            download_url: release_asset.browser_download_url.clone(),
            installable: reason.is_none(),
            unavailable_reason: reason,
        });
    }
    features.sort_by(|left, right| {
        left.feature
            .category
            .cmp(&right.feature.category)
            .then_with(|| left.feature.title.cmp(&right.feature.title))
    });
    Ok(features)
}

/// Downloads and installs one feature from a freshly validated official catalogue.
pub async fn install_feature(
    name: &str,
    expected_version: &str,
    expected_sha256: &str,
) -> ShellResult<()> {
    if !valid_package_identifier(name) {
        return Err(ShellError::PackageManager("invalid feature name".into()));
    }
    let package = feature_catalogue()
        .await?
        .into_iter()
        .find(|package| package.name == name)
        .ok_or_else(|| {
            ShellError::PackageManager("feature is not in the official catalog".into())
        })?;
    if !package.installable {
        return Err(ShellError::PackageManager(
            package
                .unavailable_reason
                .unwrap_or_else(|| "feature is not ready for installation".into()),
        ));
    }
    if package.version != expected_version || package.sha256 != expected_sha256 {
        return Err(ShellError::PackageManager(
            "the release catalog changed; refresh feature details before installing".into(),
        ));
    }
    let executable = std::env::var_os("SJEL_CLI_BIN")
        .or_else(|| {
            std::env::var_os("SJEL_ROOT")
                .map(std::path::PathBuf::from)
                .map(|root| root.join("target/release/sjel-cli").into_os_string())
        })
        .ok_or_else(|| {
            ShellError::PackageManager("placed runtime installer is unavailable".into())
        })?;

    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        use std::sync::atomic::{AtomicU64, Ordering};

        static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|error| ShellError::PackageManager(error.to_string()))?;
        let response = client
            .get(&package.download_url)
            .header(header::USER_AGENT, "Sjel-Desktop")
            .send()
            .map_err(|error| {
                ShellError::PackageManager(format!("feature download failed: {error}"))
            })?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length != package.bytes)
        {
            return Err(ShellError::PackageManager(
                "feature download does not match the catalog size".into(),
            ));
        }
        let archive = std::env::temp_dir().join(format!(
            "sjel-feature-{}-{}.tar.gz",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut created = false;
        let result = (|| {
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&archive)
                .map_err(|error| {
                    ShellError::PackageManager(format!("cannot stage feature download: {error}"))
                })?;
            created = true;
            let copied = std::io::copy(
                &mut response.take(package.bytes.saturating_add(1)),
                &mut output,
            )
            .map_err(|error| {
                ShellError::PackageManager(format!("feature download failed: {error}"))
            })?;
            if copied != package.bytes {
                return Err(ShellError::PackageManager(
                    "feature download size differs from the catalog".into(),
                ));
            }
            output.sync_all().map_err(|error| {
                ShellError::PackageManager(format!("cannot sync feature download: {error}"))
            })?;
            drop(output);
            let output = Command::new(executable)
                .arg("distribution")
                .arg("install-package")
                .arg(&archive)
                .arg("--sha256")
                .arg(&package.sha256)
                .output()
                .map_err(|error| {
                    ShellError::PackageManager(format!("cannot run package installer: {error}"))
                })?;
            if output.status.success() {
                Ok(())
            } else {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                Err(ShellError::PackageManager(if detail.is_empty() {
                    "runtime refused the feature package".into()
                } else {
                    detail
                }))
            }
        })();
        if created {
            let _ = std::fs::remove_file(archive);
        }
        result
    })
    .await
    .map_err(|error| ShellError::PackageManager(error.to_string()))?
}

fn validate_release_asset_url(value: &str, version: &str, asset: &str) -> ShellResult<()> {
    let url = Url::parse(value).map_err(|_| ShellError::Unreadable)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.path() != format!("/larsboes/Sjel/releases/download/{version}/{asset}")
    {
        return Err(ShellError::PackageManager(
            "catalog asset URL is not from the official release".into(),
        ));
    }
    Ok(())
}

fn runtime_identity() -> Option<(String, String)> {
    let root = std::env::var_os("SJEL_ROOT")?;
    let identity: RuntimeIdentity = serde_json::from_slice(
        &std::fs::read(std::path::PathBuf::from(root).join("distribution.json")).ok()?,
    )
    .ok()?;
    Some((identity.version, identity.platform))
}

fn installer_available() -> bool {
    std::env::var_os("SJEL_CLI_BIN").is_some_and(|path| std::path::Path::new(&path).is_file())
        || std::env::var_os("SJEL_ROOT")
            .map(std::path::PathBuf::from)
            .is_some_and(|root| root.join("target/release/sjel-cli").is_file())
}

/// One frame of `GET /api/sjel-status/events`: both answers, pushed when either changes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Snapshot {
    pub health: Option<NodeHealth>,
    pub agent: Option<AgentView>,
    /// Why the shell could not read one of the two answers.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// The endpoint is not on loopback.
    NotLoopback,
    /// No token in the Keychain.
    NotSetUp,
    /// Nothing answered.
    Unreachable,
    /// The shell answered with an error status, and its `error` text when it sent one.
    Refused(u16, Option<String>),
    /// The shell answered 200 with a body this client cannot read.
    Unreadable,
    /// A paired-device request failed or was refused.
    Transport(String),
    /// A provider credential could not be written to the OS keychain.
    CredentialUnavailable,
    /// The local packaged runtime cannot report its installed feature packages.
    PackageManager(String),
    /// The phone's own store failed, and why. No shell was asked.
    Store(String),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback => {
                f.write_str("SJEL_STATUS_URL must point to 127.0.0.1 or localhost.")
            }
            Self::NotSetUp => {
                f.write_str("No Sjel token on this Mac. Run tools/setup-inbound-auth.sh.")
            }
            Self::Unreachable => f.write_str("Sjel is not running on this Mac."),
            Self::Refused(_, Some(error)) => f.write_str(error),
            Self::Refused(status, None) => write!(f, "Sjel refused the request ({status})."),
            Self::Unreadable => f.write_str("Sjel answered with something this app cannot read."),
            Self::Transport(why) => f.write_str(why),
            Self::CredentialUnavailable => {
                f.write_str("Could not save the provider API key to the OS keychain.")
            }
            Self::PackageManager(why) => write!(f, "Feature packages are unavailable: {why}"),
            Self::Store(why) => write!(f, "This device could not save it: {why}"),
        }
    }
}

pub type ShellResult<T> = Result<T, ShellError>;

pub struct Shell {
    base: Url,
    client: reqwest::Client,
    token: Arc<Mutex<Option<String>>>,
}

impl Shell {
    /// # Errors
    /// `NotLoopback` when `endpoint` is not an `http` URL on 127.0.0.1 or localhost.
    pub fn new(endpoint: &str) -> ShellResult<Self> {
        let base = Url::parse(endpoint).map_err(|_| ShellError::NotLoopback)?;
        let loopback =
            base.scheme() == "http" && matches!(base.host_str(), Some("127.0.0.1" | "localhost"));
        if !loopback {
            return Err(ShellError::NotLoopback);
        }
        Ok(Self {
            base,
            client: reqwest::Client::new(),
            token: Arc::new(Mutex::new(None)),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    pub async fn health(&self) -> ShellResult<NodeHealth> {
        self.json(Method::GET, "/api/sjel-status/health", None, 10)
            .await
    }

    pub async fn capability_registry(&self) -> ShellResult<Vec<RegistryEntry>> {
        self.json(Method::GET, "/api/sjel-status/capabilities", None, 10)
            .await
    }

    pub async fn capability_links(
        &self,
        capability: &str,
        id: &str,
    ) -> ShellResult<crate::record::CapabilityLinks> {
        if !is_registry_name(capability) || !valid_typed_id(id) {
            return Err(ShellError::Refused(
                400,
                Some("Not a capability or typed record id.".into()),
            ));
        }
        let path = with_query(&format!("/{capability}/api/links"), &[("to", id)]);
        self.json(Method::GET, &path, None, 10).await
    }

    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<T> {
        let response = self.send(method, path, body, timeout_secs).await?;
        response.json().await.map_err(|_| ShellError::Unreadable)
    }

    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        self.send_payload(method, path, body.map(Payload::Json), timeout_secs)
            .await
    }

    /// One request path for every body shape: JSON, raw bytes for an upload, or none. Keeping the
    /// credential read, the 401 recovery and the error text in one place is what stops the upload
    /// route from growing its own idea of what a refusal looks like.
    async fn send_payload(
        &self,
        method: Method,
        path: &str,
        payload: Option<Payload>,
        timeout_secs: u64,
    ) -> ShellResult<reqwest::Response> {
        let url = self.base.join(path).map_err(|_| ShellError::Unreadable)?;
        // A stale Keychain token cannot have changed a GET's state. Refresh it and retry that
        // read once; never replay writes or uploads after a refusal.
        let has_payload = payload.is_some();
        let mut payload = payload;
        let mut attempt = 0;
        loop {
            let token = self.token()?;
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(token)
                .timeout(Duration::from_secs(timeout_secs));
            request = match payload.take() {
                None => request,
                Some(Payload::Json(body)) => request.json(&body),
                Some(Payload::Bytes(bytes)) => request
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(bytes),
            };
            let response = request.send().await.map_err(|_| ShellError::Unreachable)?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED {
                let body = response.bytes().await.unwrap_or_default();
                let rejected_token = authentication_rejection(&body);
                *self.token.lock().expect("token lock") = None;
                if should_retry_auth(&method, has_payload, rejected_token, attempt) {
                    attempt += 1;
                    continue;
                }
                return Err(ShellError::Refused(status.as_u16(), error_text(&body)));
            }
            if status.is_success() {
                return Ok(response);
            }
            let body = response.bytes().await.unwrap_or_default();
            return Err(ShellError::Refused(status.as_u16(), error_text(&body)));
        }
    }

    fn token(&self) -> ShellResult<String> {
        cached_token(&self.token)
    }
}

fn cached_token(cache: &Mutex<Option<String>>) -> ShellResult<String> {
    let mut cached = cache.lock().expect("token lock");
    if let Some(token) = cached.as_ref() {
        return Ok(token.clone());
    }
    // A demo node's throwaway token comes from a file (`tools/demo-shots`), never the Keychain.
    if let Some(path) = std::env::var_os("SJEL_INBOUND_TOKEN_FILE") {
        let token = std::fs::read_to_string(path).map_err(|_| ShellError::NotSetUp)?;
        let token = token.trim();
        if token.is_empty() {
            return Err(ShellError::NotSetUp);
        }
        *cached = Some(token.to_owned());
        return Ok(token.to_owned());
    }
    let user = std::env::var("USER").map_err(|_| ShellError::NotSetUp)?;
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            &user,
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .output()
        .map_err(|_| ShellError::NotSetUp)?;
    let token = String::from_utf8(output.stdout).map_err(|_| ShellError::NotSetUp)?;
    let token = token.trim();
    if !output.status.success() || token.is_empty() {
        return Err(ShellError::NotSetUp);
    }
    *cached = Some(token.to_owned());
    Ok(token.to_owned())
}

fn authentication_rejection(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| {
            body.get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|error| error == "invalid or missing authentication token")
}

fn should_retry_auth(
    method: &Method,
    has_payload: bool,
    auth_rejection: bool,
    attempt: usize,
) -> bool {
    attempt == 0
        && !has_payload
        && auth_rejection
        && (method == Method::GET || method == Method::HEAD)
}

fn is_capability_status_question(prompt: &str) -> bool {
    let words: Vec<String> = prompt
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let asks_about_capabilities = words
        .iter()
        .any(|word| word.starts_with("capabilit") || word == "services");
    let asks_about_status = ["up", "running", "status", "health", "available"]
        .iter()
        .any(|term| words.iter().any(|word| word == term));
    asks_about_capabilities && asks_about_status
}

/// A path with its query, each pair encoded once. The capability owns the names; this only joins
/// them, so a project name with a `/` or a space reaches it intact.
fn with_query(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse("http://127.0.0.1").expect("a static origin parses");
    url.set_path(path);
    url.query_pairs_mut().extend_pairs(pairs);
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

fn valid_calendar_entry_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

fn valid_cloud_content_id(source: &str, id: &str) -> bool {
    matches!(source, "feed" | "mail")
        && !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && (source != "feed" || is_sha256(id))
}

fn valid_typed_id(id: &str) -> bool {
    id.len() <= 256
        && id.split(':').count() >= 2
        && id.split(':').all(|part| !part.is_empty())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

fn valid_scouting_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

/// Why one connection to the change stream ended.
enum Followed {
    /// The shell answered, but does not serve the stream: a build from before it existed.
    Unsupported,
    /// The saved token expired; the owner loop reloads it before reconnecting.
    Unauthorized,
    /// The shell or the network dropped it. Reconnecting is worth a try.
    Lost,
}

async fn follow(
    client: &reqwest::Client,
    url: &Url,
    token: &str,
    to: &UnboundedSender<Snapshot>,
) -> Followed {
    let response = client
        .get(url.clone())
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await;
    let Ok(response) = response else {
        return Followed::Lost;
    };
    let status = response.status();
    if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
        return Followed::Unsupported;
    }
    if status == StatusCode::UNAUTHORIZED {
        return Followed::Unauthorized;
    }
    if !status.is_success() {
        return Followed::Lost;
    }
    let mut response = response;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return Followed::Lost,
            Err(_) => return Followed::Lost,
        };
        buffer.extend_from_slice(&chunk);
        while let Some(data) = next_frame(&mut buffer) {
            if let Ok(snapshot) = serde_json::from_str::<Snapshot>(&data) {
                if to.unbounded_send(snapshot).is_err() {
                    return Followed::Unsupported;
                }
            }
        }
    }
}

/// One event's data, if a whole event has arrived. A comment line carries no data and yields an
/// empty string, which no snapshot parses as, so it passes through unremarked.
fn next_frame(buffer: &mut Vec<u8>) -> Option<String> {
    let end = buffer.windows(2).position(|pair| pair == b"\n\n")?;
    let block = String::from_utf8_lossy(&buffer[..end]).into_owned();
    buffer.drain(..end + 2);
    let mut data = String::new();
    for line in block.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    }
    Some(data)
}

/// Letters, digits and dashes: nothing a path can be built from.
fn is_registry_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A check this client made before asking, in the same shape as the shell's own refusal.
fn refused(why: String) -> ShellError {
    ShellError::Refused(400, Some(why))
}

/// Two worker threads for every request. GPUI's executor does not provide Tokio's reactor,
/// which reqwest needs. A token read through `/usr/bin/security` blocks its worker, so one
/// read must not be able to stall an unrelated request.
fn runtime() -> &'static tokio::runtime::Handle {
    static RUNTIME: LazyLock<tokio::runtime::Handle> = LazyLock::new(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build the HTTP runtime");
        let handle = runtime.handle().clone();
        std::thread::Builder::new()
            .name("sjel-http".into())
            .spawn(move || runtime.block_on(std::future::pending::<()>()))
            .expect("start the HTTP thread");
        handle
    });
    &RUNTIME
}

/// Runs `work` on the HTTP runtime; the returned future can be awaited from GPUI.
/// A software row key as one URL path segment. Keys are a bundle id (`com.example.app`) or
/// `<manager>:<name>`, where an npm name may be scoped (`npm:@playwright/cli`); `/` and `@` are
/// percent-encoded so the key stays one segment. Anything else is refused before it reaches a URL.
fn web_link_key(id: &str) -> ShellResult<&str> {
    if id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(id)
    } else {
        Err(ShellError::Refused(400, Some("Not a web-link id.".into())))
    }
}

pub fn software_key(key: &str) -> ShellResult<String> {
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '@' | '/');
    // A key of only dots (`..`) would be a path segment the URL join collapses.
    if !key.chars().any(|c| c.is_ascii_alphanumeric()) || !key.chars().all(allowed) {
        return Err(ShellError::Refused(400, Some("Not a software key.".into())));
    }
    Ok(key.replace('@', "%40").replace('/', "%2F"))
}

pub fn run<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (tx, rx) = futures::channel::oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(work.await);
    });
    async move { rx.await.ok() }
}

/// Runs `work` on the HTTP runtime with nothing waiting for its answer, for a connection that
/// outlives one call.
pub fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(work);
}

/// The reason in a refused response: `{"error": …}`, or a short plain-text body. Some
/// capabilities answer a refusal as text (transit's `fail`, the proxy's 502), and dropping it
/// leaves only a status code to show.
pub(crate) fn error_text(body: &[u8]) -> Option<String> {
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
        return json.get("error")?.as_str().map(str::to_owned);
    }
    let text = std::str::from_utf8(body).ok()?.trim();
    (!text.is_empty() && text.len() <= 300 && !text.starts_with('<')).then(|| text.to_owned())
}

mod documents;
mod feed_mail;
mod finance_inventory;
mod people_calendar;
mod projects;
mod systems_backup;
#[cfg(test)]
mod tests;
