//! One home for "which model answers this, on this machine".
//!
//! Before this existed there were four shapes for the same fact: comms'
//! `SummarizerConfig` and `RelevanceConfig`, scouting's `EmbedConfig`,
//! the since-deleted `libs/ai-client`'s `RouterConfig`, plus `tools/graphify.sh`'s
//! env var. Each
//! knew a base URL and a model name, and each had to be edited by hand to move
//! a machine between runtimes. `systems.toml` already stated the rule this
//! module implements, in the oMLX entry: *"Referenced by id, not by URL,
//! because host/port/model differ per machine."*
//!
//! ## The shape
//!
//! Two levels, and callers only ever touch the second.
//!
//! * A **backend** is a server: an API shape, a base URL, optionally a file to
//!   read a bearer key out of. Declared once.
//! * A **role** is a job: `embedding`, `reranking`, `summarization`, `ocr`, or an explicitly
//!   reviewed `cloud_*` task. It names a backend, the
//!   model on it, and that model's input conventions.
//!
//! `ocr` is named here and **declared nowhere**, on purpose. It is rung 3 of the extraction
//! ladder (PRD Q63 -> B30) and `libs/extraction/src/ocr_role.rs` is its only caller. No engine
//! has cleared the frozen DE/EN corpus at `libs/extraction/eval/`, which is the same gate
//! `multilingual-e5-base-mlx` and `bge-reranker-v2-m3-mlx` cleared and
//! `multilingual-e5-small-mlx` did not. Declaring the role before an engine clears it would
//! point a real dispatch at an unmeasured model.
//!
//! A capability asks for a role. It never learns whether it just talked to
//! oMLX or Ollama, which is the whole point: oMLX needs Metal and cannot exist
//! on the family Pi, Ollama runs anywhere, and moving between them must be a
//! config edit rather than a code change.
//!
//! ## Two things portability actually requires
//!
//! **Models are not interchangeable.** `multilingual-e5-*` wants `query: ` and
//! `passage: ` role prefixes; `nomic-embed-text` wants `search_query: ` and
//! `search_document: `. Sending the wrong ones costs retrieval quality and
//! raises no error at all, so the prefixes belong to the role, beside the
//! model, and travel with it. The names are backend-specific too, which is why
//! the machine backend override moves a role only together with the
//! [`Role::on_backend`] entry that says what the job is called there.
//!
//! **Cached vectors belong to the model that produced them.** A cache keyed on
//! the profile alone silently serves e5 vectors to a nomic run after a backend
//! switch — every score wrong, nothing logged. [`ResolvedRole::cache_key`]
//! exists so a cache can name its producer, and consumers are expected to use
//! it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The wire shape a backend speaks. Both are embedding-capable; the request
/// and response bodies differ, nothing else does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    /// `POST {base_url}/embeddings`, bearer auth, `data[].embedding` back.
    /// oMLX, LM Studio, Ollama's own `/v1` shim, and every hosted provider.
    OpenAi,
    /// `POST {base_url}/api/embed`, no auth, `embeddings` back. Ollama native.
    Ollama,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    pub api: Api,
    /// For `OpenAi` this includes the version segment, e.g.
    /// `http://127.0.0.1:8000/v1`. For `Ollama` it is the bare origin.
    pub base_url: String,
    /// A file to read a bearer key from. If it parses as JSON, `.auth.api_key`
    /// is used, which is what lets this point straight at
    /// `~/.omlx/settings.json`; otherwise the trimmed contents are the key.
    /// Never a key value inline — secrets are references in this repo.
    #[serde(default)]
    pub api_key_file: Option<String>,
    /// The `systems.local.toml` id of the host that serves this backend, when
    /// it is not this machine (PRD Q39, 2026-08-25). The same `provided_by`
    /// vocabulary `tools/lib/external-ref.sh` already owns for a capability
    /// this machine consumes but does not run.
    ///
    /// Naming a host is not trusting it. Trust is the intersection of this id
    /// and the list in [`TRUSTED_PEERS_ENV`], which the shell resolver writes
    /// from the overlay — an address is not a permission, and neither is a name.
    #[serde(default)]
    pub provided_by: Option<String>,
}

/// `Default` is for the tests, and it earns its place: adding an optional field to this struct
/// broke the same six test constructors twice in one day, once for `chat_template_kwargs` and
/// once for `request_overrides`. Deserialization is unaffected -- `backend` and `model` carry no
/// `serde(default)` and stay required -- so this only lets a test say which fields it cares
/// about and leave the rest alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Role {
    pub backend: String,
    pub model: String,
    /// Safe operator-facing provider name. Cloud roles must declare this;
    /// endpoint and credential details remain backend-private.
    #[serde(default)]
    pub provider_name: Option<String>,
    /// Maximum cloud representation this role may receive. Local roles leave
    /// this unset; an unset cloud policy is not dispatch-eligible.
    #[serde(default)]
    pub cloud_data_tier: Option<CloudDataTier>,
    /// Billing boundary selected by the operator. There is deliberately no
    /// pay-as-you-go mode: a provider must be free-only or bounded by credits.
    #[serde(default)]
    pub billing_mode: Option<BillingMode>,
    /// Stable ordering among same-tier fallback roles. The explicitly selected
    /// role always runs first; lower values win for later candidates.
    #[serde(default)]
    pub failover_priority: Option<u16>,
    /// Local hard ceiling for provider requests started on one UTC date.
    /// Cloud roles without a non-zero ceiling are deliberately inert.
    ///
    /// It is a guard, not a description, and that makes it worthless set ABOVE the provider's
    /// own limit -- a distinction with a measured cost. On 2026-08-30 this deployment declared
    /// 1000/day for a Gemini free-tier role whose real quota is 20, and 200/day for a Cloudflare
    /// role that stopped serving after 81: 119 requests past the ceiling that mattered, all
    /// answered 429, all counted as failures against the items that asked for them.
    ///
    /// So the number belongs to the provider, and the provider will usually tell you. Google
    /// names it in the 429 body (`quotaId`, `quotaValue`); Cloudflare meters a compute unit
    /// rather than requests, so a count there is an approximation that moves with model size and
    /// has to be re-measured. Both are recorded in `systems.toml` beside the service they
    /// describe, which is where a role's ceiling should be read from rather than guessed.
    #[serde(default)]
    pub max_requests_per_day: Option<u32>,
    /// Local upper bound for request input tokens. Dispatch uses UTF-8 bytes as
    /// a conservative tokenizer-independent upper bound and never calls the
    /// provider when that bound exceeds this value.
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
    /// Required for prepaid-credit roles, in YYYY-MM-DD form. Free-only roles
    /// have no credit expiry and leave this unset.
    #[serde(default)]
    pub credit_expires_on: Option<String>,
    /// Prefixed onto the *query* side of a retrieval pair. Empty for models
    /// that take none.
    #[serde(default)]
    pub query_prefix: String,
    /// Prefixed onto the *document* side.
    #[serde(default)]
    pub document_prefix: String,
    /// Provider-specific chat-template arguments, passed through verbatim as
    /// `chat_template_kwargs` on a chat-completions request.
    ///
    /// The same category as the prefixes above -- this file's own words for what a role names are
    /// "the backend, the model on it, and that model's input conventions" -- and a reasoning
    /// toggle is an input convention rather than a Comms concern. Per role and never global,
    /// because a key one provider requires another rejects with a 400.
    ///
    /// Why it exists: `nvidia/nemotron-3-nano-30b-a3b` reasons before answering, and NIM returns
    /// that reasoning in `message.content` -- not in `reasoning_content` -- whenever the token
    /// budget runs out mid-thought. 15 of 23 stored cloud digests were the model's own monologue
    /// (measured 2026-08-30). `{"thinking": false}` spends the budget on the answer instead.
    ///
    /// KNOWN LIMIT, and the reason it is written here rather than filed away: the key name is
    /// fixed. `cloud_dispatch::chat` inserts this under the literal `chat_template_kwargs`,
    /// which is what vLLM and NIM read and what Gemini's OpenAI-compatible endpoint does not.
    /// `gemini-3.6-flash` reasons too, and with no way to declare its control it fails every
    /// digest with `finish_reason: length` -- measured 2026-08-30 at 32 failures and 0 successes
    /// in a day, against 42 successes and 0 failures for the NIM role that has the kwarg. The
    /// role was demoted below NVIDIA on that evidence; the real fix is a role field that can
    /// carry an arbitrary request override (Gemini wants top-level `reasoning_effort`), and it
    /// is unmeasured because the free tier was rate-limiting by the time the question was asked.
    #[serde(default)]
    pub chat_template_kwargs: Option<serde_json::Value>,
    /// Extra top-level fields merged verbatim into the chat-completions request body.
    ///
    /// The general form of the field above, and the reason it exists: `chat_template_kwargs`
    /// can only ever emit the key of that name, which is what vLLM and NIM read. A provider
    /// with the same need and a different spelling could not be configured at all --
    /// `gemini-3.6-flash` reasons before answering exactly as nemotron does, wants top-level
    /// `reasoning_effort`, and failed 33 of 33 digests on 2026-08-30 for want of a place to
    /// say so.
    ///
    /// Per role and never global, for the same reason as the narrow field: a key one provider
    /// requires another is entitled to reject with a 400. Merged after the request is built, so
    /// a role can also override a default the task set -- which is deliberate, and is why this
    /// is an operator-reviewed config value rather than something a caller passes.
    #[serde(default)]
    pub request_overrides: Option<serde_json::Value>,
    /// The same job on another local runtime, keyed by backend id. Read only
    /// when [`BACKEND_OVERRIDE_ENV`] names one of these; a role that names none
    /// is simply not available on a machine that overrides its backend.
    #[serde(default)]
    pub on_backend: HashMap<String, BackendModel>,
}

/// What a role is called on one specific backend.
///
/// A model id is backend-specific: `multilingual-e5-base-mlx` exists on oMLX
/// and nowhere else, so a machine override that swapped the backend alone would
/// ask Ollama for an MLX name and get a 404 for every request. The name has to
/// move with the backend or the override is not portability, only a different
/// way to fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendModel {
    pub model: String,
    /// Deliberately NOT inherited from the role. Prefixes belong to the model
    /// and this entry names a different one: `query: ` on `nomic-embed-text`
    /// costs retrieval quality and raises nothing, which is the failure the
    /// per-role prefixes exist to prevent. Empty is the safe default.
    #[serde(default)]
    pub query_prefix: String,
    #[serde(default)]
    pub document_prefix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudDataTier {
    Public,
    PseudonymizedPersonal,
}

impl CloudDataTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::PseudonymizedPersonal => "pseudonymized_personal",
        }
    }

    /// How much this tier admits. Higher accepts everything lower accepts, and more.
    ///
    /// Not an arbitrary ranking: `cloud_derivative::tier_allows` already says a
    /// `pseudonymized_personal` role takes a public passthrough *as well as* a redacted
    /// personal derivative, while a `public` role takes only the passthrough. This makes
    /// that containment a value the rest of the code can ask about instead of restating.
    fn breadth(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::PseudonymizedPersonal => 1,
        }
    }

    /// Whether a role at this tier may take work selected for `other`.
    ///
    /// Widening is safe and narrowing is not, which is the whole asymmetry: a provider
    /// reviewed for pseudonymized personal content is already trusted with more than a
    /// public document, so handing it one takes nothing new. The reverse would put a
    /// redacted personal derivative in front of a role reviewed only for public text.
    pub fn admits_at_least(self, other: Self) -> bool {
        self.breadth() >= other.breadth()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    FreeOnly,
    PrepaidCredit,
}

impl BillingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FreeOnly => "free_only",
            Self::PrepaidCredit => "prepaid_credit",
        }
    }
}

/// Which side of a retrieval pair a text is, so the right prefix goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    Query,
    Document,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceConfig {
    #[serde(default)]
    pub backends: HashMap<String, Backend>,
    #[serde(default)]
    pub roles: HashMap<String, Role>,
}

/// A role with its backend already looked up. This is what callers hold.
#[derive(Debug, Clone)]
pub struct ResolvedRole {
    pub backend_name: String,
    pub backend: Backend,
    pub model: String,
    pub provider_name: Option<String>,
    pub cloud_data_tier: Option<CloudDataTier>,
    pub billing_mode: Option<BillingMode>,
    pub failover_priority: Option<u16>,
    pub max_requests_per_day: Option<u32>,
    pub max_input_tokens: Option<u32>,
    pub credit_expires_on: Option<String>,
    pub query_prefix: String,
    pub document_prefix: String,
    /// See the config field of the same name: provider-specific chat-template arguments,
    /// forwarded verbatim by the caller that builds the request.
    pub chat_template_kwargs: Option<serde_json::Value>,
    /// See the config field of the same name: extra top-level request fields, merged verbatim.
    pub request_overrides: Option<serde_json::Value>,
    /// Whether this role's backend runs on hardware the operator declared as
    /// their own (PRD Q39). Resolved at construction from the backend's
    /// `provided_by` and [`TRUSTED_PEERS_ENV`], so the environment is read in
    /// one place and a test can state the answer directly.
    pub trusted_peer: bool,
}

fn model_ids_match(configured: &str, installed: &str) -> bool {
    if installed == configured {
        return true;
    }
    let configured_basename = configured.rsplit('/').next().unwrap_or(configured);
    let installed_without_latest = installed.strip_suffix(":latest").unwrap_or(installed);
    installed_without_latest == configured
        || (!installed.contains('/') && installed_without_latest == configured_basename)
}

/// Whether a base URL addresses this machine. Backend-level rather than
/// role-level because the machine override has to ask it of a declared backend
/// before any role is resolved against it.
fn is_loopback_url(base_url: &str) -> bool {
    let address = base_url.trim().to_ascii_lowercase();
    let authority = address
        .strip_prefix("http://")
        .or_else(|| address.strip_prefix("https://"))
        .unwrap_or(&address)
        .split('/')
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = if authority.starts_with('[') {
        authority
            .strip_prefix('[')
            .and_then(|value| value.split(']').next())
            .unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host == "127.0.0.1"
        || host.starts_with("127.")
}

fn valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// Names the one local model runtime this machine actually has.
///
/// `service-runner.sh` exports this from `machine.toml`'s `[inference] backend`,
/// the same path `[capability.<name>] port` already takes to reach a process.
/// A machine that cannot run the configured backend says so once, in the file
/// that already holds machine-local facts, and no capability config changes.
///
/// It moves a role only when the role's declared backend is loopback and the
/// role names a model on the target under `on_backend`. `InferenceConfig`'s
/// resolution states why each of those two conditions is load-bearing.
pub const BACKEND_OVERRIDE_ENV: &str = "SJEL_INFERENCE_BACKEND";

/// Points at the config file directly. Mainly for tests and one-off runs.
pub const CONFIG_PATH_ENV: &str = "SJEL_INFERENCE_CONFIG";

/// Comma-separated `systems.local.toml` ids the operator has declared to be
/// their own hardware, written by `tools/service-runner.sh` from the resolver
/// in `tools/lib/external-ref.sh` (PRD Q39, 2026-08-25).
///
/// **Why this arrives as an environment variable rather than being read here.**
/// The declaration lives in two overlay files — `machine.toml` says who provides
/// what, `systems.local.toml` holds the address and the ownership — and
/// `external-ref.sh` is the one resolver for that pair. Q39 says extend it
/// rather than invent a second one, so this crate receives the answer instead of
/// re-deriving it, exactly as it already receives the machine's backend choice
/// through [`BACKEND_OVERRIDE_ENV`].
///
/// Unset means no trusted peers, which is this machine's state until the second
/// host lands (B9) and is byte-for-byte the behaviour that shipped before Q39.
pub const TRUSTED_PEERS_ENV: &str = "SJEL_INFERENCE_TRUSTED_PEERS";

/// Split the declared ids out of the raw variable. Empty entries are dropped,
/// so a trailing comma or a stray separator cannot declare the empty id — which
/// a backend with no `provided_by` would otherwise match.
///
/// Separate from the read so it can be tested without touching the process
/// environment. `std::env::set_var` is process-wide and the test binary is
/// threaded; a suite that mutates it is the failure PRD §13.1 records as the
/// second half of its third silent failure.
fn parse_trusted_peers(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// The declared ids, read once per resolved role.
fn declared_trusted_peers() -> Vec<String> {
    parse_trusted_peers(&std::env::var(TRUSTED_PEERS_ENV).unwrap_or_default())
}

impl InferenceConfig {
    /// Reads the overlay's `inference.json`, or returns an empty config when
    /// there is none. Absent config is not an error: every consumer is
    /// expected to degrade to something that still works offline (scouting
    /// falls back to hash embedding), so a machine with no inference set up at
    /// all keeps running rather than failing at startup.
    pub fn load(overlay_config: impl Fn(&str) -> Option<PathBuf>) -> Self {
        let path = match std::env::var(CONFIG_PATH_ENV) {
            Ok(explicit) if !explicit.trim().is_empty() => PathBuf::from(explicit),
            _ => match overlay_config("inference.json") {
                Some(path) => path,
                None => return Self::default(),
            },
        };
        Self::from_path(&path)
    }

    pub fn from_path(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match text.parse::<Self>() {
                Ok(mut config) => {
                    config.resolve_relative_key_files(path.parent().unwrap_or(Path::new(".")));
                    config
                }
                Err(error) => {
                    eprintln!(
                        "  inference: {} is not readable as config ({error}) — continuing without it",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    fn resolve_relative_key_files(&mut self, config_directory: &Path) {
        for backend in self.backends.values_mut() {
            let Some(raw) = backend.api_key_file.as_deref() else {
                continue;
            };
            if raw.starts_with("~/") || Path::new(raw).is_absolute() {
                continue;
            }
            backend.api_key_file = Some(config_directory.join(raw).to_string_lossy().into_owned());
        }
    }

    /// Looks a role up and resolves its backend, applying the machine
    /// override. `None` means this machine has no way to do that job, which is
    /// a normal state a caller degrades from, not a crash.
    pub fn role(&self, name: &str) -> Option<ResolvedRole> {
        let machine_backend = match std::env::var(BACKEND_OVERRIDE_ENV) {
            Ok(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            _ => None,
        };
        self.role_on(name, machine_backend.as_deref())
    }

    /// [`Self::role`] with the machine override handed in instead of read from
    /// the environment. Separate because an env var is process-global while
    /// `cargo test` runs threads in parallel, so a test that set one would
    /// decide what every other test resolved.
    ///
    /// Two conditions gate the override, and each one is a defect it prevents:
    ///
    /// * **The declared backend must be loopback.** The override states which
    ///   *local* runtime exists here. A hosted backend answers from every
    ///   machine, so there is nothing machine-local to replace — and rewriting
    ///   a `cloud_*` role's backend would point a reviewed provider policy at
    ///   whatever this host happens to run.
    /// * **The role must name a model on the target.** Swapping the backend
    ///   alone leaves an MLX model id addressed to Ollama. `None` here is the
    ///   documented degrade path (reranking has no Ollama equivalent at all),
    ///   and it fails at resolution rather than at the first HTTP 404.
    fn role_on(&self, name: &str, machine_backend: Option<&str>) -> Option<ResolvedRole> {
        let role = self.roles.get(name)?;
        let mut backend_name = role.backend.clone();
        let mut model = role.model.as_str();
        let mut query_prefix = role.query_prefix.as_str();
        let mut document_prefix = role.document_prefix.as_str();

        if let Some(target) = machine_backend.filter(|target| *target != role.backend) {
            let declared_is_local = self
                .backends
                .get(&role.backend)
                .is_some_and(|backend| is_loopback_url(&backend.base_url));
            if declared_is_local {
                let variant = role.on_backend.get(target).or_else(|| {
                    eprintln!(
                        "  inference: role '{name}' names no model on backend '{target}' \
                         (machine.toml [inference] backend) — this machine cannot do that job"
                    );
                    None
                })?;
                backend_name = target.to_string();
                model = &variant.model;
                query_prefix = &variant.query_prefix;
                document_prefix = &variant.document_prefix;
            }
        }

        let backend = self.backends.get(&backend_name).cloned().or_else(|| {
            eprintln!(
                "  inference: role '{name}' wants backend '{backend_name}', which is not declared"
            );
            None
        })?;
        let trusted_peer = backend
            .provided_by
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .is_some_and(|id| declared_trusted_peers().iter().any(|peer| peer == id));
        Some(ResolvedRole {
            backend_name,
            backend,
            model: model.to_string(),
            provider_name: role.provider_name.clone(),
            cloud_data_tier: role.cloud_data_tier,
            billing_mode: role.billing_mode,
            failover_priority: role.failover_priority,
            max_requests_per_day: role.max_requests_per_day,
            max_input_tokens: role.max_input_tokens,
            credit_expires_on: role.credit_expires_on.clone(),
            chat_template_kwargs: role.chat_template_kwargs.clone(),
            request_overrides: role.request_overrides.clone(),
            query_prefix: query_prefix.to_string(),
            document_prefix: document_prefix.to_string(),
            trusted_peer,
        })
    }

    /// Resolve explicitly named cloud roles in stable order. Consumers still
    /// decide whether the endpoint policy is suitable for their operation.
    pub fn roles_with_prefix(&self, prefix: &str) -> Vec<(String, ResolvedRole)> {
        let mut names = self
            .roles
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| self.role(&name).map(|role| (name, role)))
            .collect()
    }

    /// The selected role runs first. Remaining candidates are deterministic
    /// and may only share its exact reviewed data tier; billing and runtime
    /// budget checks still happen immediately before each request.
    pub fn cloud_failover_roles(&self, selected: &str) -> Vec<(String, ResolvedRole)> {
        let Some(selected_role) = self.role(selected).filter(|role| role.has_cloud_policy()) else {
            return Vec::new();
        };
        let Some(tier) = selected_role.cloud_data_tier else {
            return Vec::new();
        };
        // Same tier OR WIDER, not equality. Equality is what parked 41 public feed digests
        // on this machine: `public` is the narrowest tier, `tier_rank` in comms' cloud_run
        // deliberately selects the narrowest role first, and so the only failover candidate
        // for a public digest was the single `public` role that had just returned 429. The
        // two roles that would have taken it -- `tier_allows` says a pseudonymized_personal
        // role admits a public passthrough, in as many words -- were never offered, and the
        // job burned its attempts against one rate-limited provider.
        //
        // Widening the CANDIDATE list is not widening permission. `cloud_run::admits` still
        // asks `tier_allows` and `verbatim_send_allowed` about every candidate at dispatch,
        // per candidate, which is the actual door.
        let mut roles = self
            .roles_with_prefix("cloud_")
            .into_iter()
            .filter(|(_, role)| {
                role.has_cloud_policy()
                    && role
                        .cloud_data_tier
                        .is_some_and(|candidate| candidate.admits_at_least(tier))
            })
            .collect::<Vec<_>>();
        roles.sort_by(|(left_name, left), (right_name, right)| {
            let left_selected = left_name != selected;
            let right_selected = right_name != selected;
            left_selected
                .cmp(&right_selected)
                // Narrowest tier first among the rest, for the same reason `tier_rank` sorts
                // that way: a role declared for exactly this work should be preferred over one
                // that merely also permits it, or the role reviewed for the wider class ends up
                // doing all the narrow work.
                .then_with(|| {
                    left.cloud_data_tier
                        .map(CloudDataTier::breadth)
                        .cmp(&right.cloud_data_tier.map(CloudDataTier::breadth))
                })
                .then_with(|| left.failover_priority().cmp(&right.failover_priority()))
                .then_with(|| left_name.cmp(right_name))
        });
        roles
    }
}

impl FromStr for InferenceConfig {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(text).map_err(|error| error.to_string())
    }
}

impl ResolvedRole {
    /// Identifies the thing that produced a vector, so a cache can refuse to
    /// serve it to a different one. Cheap to store beside cached data and the
    /// only defence against a backend switch silently reusing incompatible
    /// vectors.
    pub fn cache_key(&self) -> String {
        format!("{}:{}", self.backend_name, self.model)
    }

    fn embedding_endpoint(&self) -> String {
        let base = self.backend.base_url.trim_end_matches('/');
        match self.backend.api {
            Api::OpenAi => format!("{base}/embeddings"),
            Api::Ollama => format!("{base}/api/embed"),
        }
    }

    fn rerank_endpoint(&self) -> Result<String, String> {
        let base = self.backend.base_url.trim_end_matches('/');
        match self.backend.api {
            Api::OpenAi => Ok(format!("{base}/rerank")),
            Api::Ollama => Err("the Ollama-native API does not expose /v1/rerank".into()),
        }
    }

    /// OpenAI-compatible chat-completions endpoint for generative roles.
    /// Ollama exposes the same wire shape under its `/v1` compatibility path.
    pub fn chat_completions_endpoint(&self) -> String {
        let base = self.backend.base_url.trim_end_matches('/');
        match self.backend.api {
            Api::OpenAi => format!("{base}/chat/completions"),
            Api::Ollama => format!("{base}/v1/chat/completions"),
        }
    }

    pub fn provider_label(&self) -> &'static str {
        // Three tiers, not two. A trusted peer reported as "local" would hide
        // that the request leaves this machine; reported as "cloud" it would
        // repeat the misclassification Q39 exists to fix.
        if self.is_trusted_peer() {
            return match self.backend.api {
                Api::OpenAi => "OpenAI-compatible trusted-peer endpoint",
                Api::Ollama => "Ollama-compatible trusted-peer endpoint",
            };
        }
        match (self.backend.api, self.is_loopback()) {
            (Api::OpenAi, true) => "OpenAI-compatible local endpoint",
            (Api::OpenAi, false) => "OpenAI-compatible cloud endpoint",
            (Api::Ollama, true) => "Ollama-compatible local endpoint",
            (Api::Ollama, false) => "Ollama-compatible cloud endpoint",
        }
    }

    /// This machine. Keep this for anything about THIS host's resources — the
    /// local GPU admission gate is the live example, and a trusted peer must
    /// not queue behind it, because it does not share the GPU.
    pub fn is_loopback(&self) -> bool {
        is_loopback_url(&self.backend.base_url)
    }

    /// Hardware the operator owns, reached over the tailnet (PRD Q39,
    /// 2026-08-25). Declared, never inferred: the backend names a host and the
    /// overlay declares that host to be the operator's.
    pub fn is_trusted_peer(&self) -> bool {
        self.trusted_peer
    }

    /// The question every data-class gate should ask: may this endpoint see
    /// anything at all?
    ///
    /// Loopback and a trusted peer answer the same, because nothing leaves
    /// hardware the operator controls. `is_loopback` used to carry both this
    /// question and *"does this share my GPU"*, and Q39 is the point at which
    /// one predicate could no longer answer both.
    pub fn trusted_for_every_class(&self) -> bool {
        self.is_loopback() || self.is_trusted_peer()
    }

    /// Cloud dispatch is restricted to encrypted endpoints that are neither
    /// this machine nor a host the operator owns.
    ///
    /// **The trap Q39 names, closed here.** `upstreams.toml` chose Tailscale
    /// over NetBird for `tailscale cert`, so a peer's URL is `https://` and
    /// non-loopback — and without the third term this function classifies the
    /// operator's own MacBook as a cloud provider and demands a reviewed
    /// `providers.toml` entry for it. Getting the transport security right made
    /// the classification wrong.
    pub fn is_cloud_endpoint(&self) -> bool {
        self.backend.base_url.trim().starts_with("https://")
            && !self.is_loopback()
            && !self.is_trusted_peer()
    }

    /// A cloud endpoint is inert until it has a complete reviewed policy.
    pub fn has_cloud_policy(&self) -> bool {
        self.is_cloud_endpoint()
            && self
                .provider_name
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty())
            && self.cloud_data_tier.is_some()
            && self.billing_mode.is_some()
            && self.max_requests_per_day.is_some_and(|limit| limit > 0)
            && self.max_input_tokens.is_some_and(|limit| limit > 0)
            && match self.billing_mode {
                Some(BillingMode::FreeOnly) => self.credit_expires_on.is_none(),
                Some(BillingMode::PrepaidCredit) => self
                    .credit_expires_on
                    .as_deref()
                    .is_some_and(valid_iso_date),
                None => false,
            }
    }

    pub fn failover_priority(&self) -> u16 {
        self.failover_priority.unwrap_or(u16::MAX)
    }

    /// A prepaid role stops before dispatch after its declared final UTC day.
    /// Free-only roles are controlled by their daily local request ceiling.
    pub fn billing_active_on(&self, utc_date: &str) -> bool {
        if !valid_iso_date(utc_date) {
            return false;
        }
        match self.billing_mode {
            Some(BillingMode::FreeOnly) => true,
            Some(BillingMode::PrepaidCredit) => self
                .credit_expires_on
                .as_deref()
                .filter(|expires| valid_iso_date(expires))
                .is_some_and(|expires| expires >= utc_date),
            None => false,
        }
    }

    /// Reads only the configured private key file and returns a boolean. The
    /// value is never exposed through provider discovery or logs.
    pub fn credential_ready(&self) -> bool {
        self.bearer_key().is_some()
    }

    pub fn dispatch_ready(&self) -> bool {
        self.has_cloud_policy() && self.credential_ready()
    }

    pub fn bearer_key(&self) -> Option<String> {
        api_key_from_file(self.backend.api_key_file.as_deref())
    }

    /// Cheap readiness probe that confirms the role's selected model is listed
    /// without loading it or running inference.
    pub fn model_reachable(&self) -> bool {
        if self.backend.base_url.trim().is_empty() || self.model.trim().is_empty() {
            return false;
        }
        let client = match axon_http::client(
            axon_http::Purpose::new("inference-probe"),
            std::time::Duration::from_secs(3),
        ) {
            Ok(client) => client,
            Err(_) => return false,
        };
        let base = self.backend.base_url.trim_end_matches('/');
        let endpoint = match self.backend.api {
            Api::OpenAi => format!("{base}/models"),
            Api::Ollama => format!("{base}/api/tags"),
        };
        let mut request = client.get(endpoint);
        if let Some(key) = self.bearer_key() {
            request = request.bearer_auth(key);
        }
        let body = match request.send() {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>() {
                    Ok(body) => body,
                    Err(_) => return false,
                }
            }
            _ => return false,
        };
        let models = match self.backend.api {
            Api::OpenAi => body.get("data").and_then(|models| models.as_array()),
            Api::Ollama => body.get("models").and_then(|models| models.as_array()),
        };
        models.is_some_and(|models| {
            models.iter().any(|model| {
                let installed = match self.backend.api {
                    Api::OpenAi => model.get("id"),
                    Api::Ollama => model.get("name").or_else(|| model.get("model")),
                }
                .and_then(|value| value.as_str())
                .unwrap_or_default();
                model_ids_match(&self.model, installed)
            })
        })
    }

    fn prefix(&self, text_role: TextRole) -> &str {
        match text_role {
            TextRole::Query => &self.query_prefix,
            TextRole::Document => &self.document_prefix,
        }
    }

    pub fn request_body(&self, texts: &[String], text_role: TextRole) -> serde_json::Value {
        let inputs = texts
            .iter()
            .cloned()
            .map(|text| (text, text_role))
            .collect::<Vec<_>>();
        self.request_body_mixed(&inputs)
    }

    pub fn request_body_mixed(&self, inputs: &[(String, TextRole)]) -> serde_json::Value {
        let input: Vec<String> = inputs
            .iter()
            .map(|(text, role)| format!("{}{text}", self.prefix(*role)))
            .collect();
        let mut payload = serde_json::json!({ "model": self.model, "input": input });
        if self.backend.api == Api::Ollama {
            // Ollama otherwise keeps the model resident for five minutes.
            // These calls are cached and bursty, so unloading immediately
            // trades a future cold start for handing unified memory back to
            // the desktop. Lifted from comms, which learned it first.
            payload["keep_alive"] = serde_json::json!(0);
        }
        payload
    }

    pub fn rerank_request_body(&self, query: &str, documents: &[String]) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "query": query,
            "documents": documents,
            "top_n": documents.len(),
            "return_documents": false,
        })
    }

    /// Scores documents jointly with one query through the Cohere/Jina-style
    /// `/v1/rerank` contract. Results are restored to input order even though
    /// the server returns them sorted by relevance.
    pub fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<f32>, String> {
        self.refuse_ungoverned_cloud_call("a query and documents to rerank")?;
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let endpoint = self.rerank_endpoint()?;
        let client = axon_http::client(
            axon_http::Purpose::new("inference-rerank"),
            std::time::Duration::from_secs(120),
        )
        .map_err(|error| format!("client build: {error}"))?;
        let mut request = client
            .post(&endpoint)
            .json(&self.rerank_request_body(query, documents));
        if let Some(key) = self.bearer_key() {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .map_err(|error| format!("POST {endpoint}: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("{endpoint} returned HTTP {}", response.status()));
        }
        let body = response
            .json::<RerankResponse>()
            .map_err(|error| format!("parse rerank response: {error}"))?;
        rerank_scores_in_input_order(body.results, documents.len())
    }

    /// Refuses to send text off this machine at all.
    ///
    /// `embed`/`rerank` are transport-agnostic and originally consulted no cloud
    /// policy whatsoever, while `has_cloud_policy()` was only ever called from
    /// comms' cloud handlers. So `role.embed()` on an https role reached a cloud
    /// provider with no preview, no redaction, no approval, no budget check and
    /// no ledger entry — the entire review discipline bypassed by a code path
    /// that never mentions the cloud. Requiring a declared policy closed most of
    /// that, and left the part that matters: a *policy* says which class of
    /// content a provider may receive, and neither of these functions is given a
    /// class. There is nothing here to check it against.
    ///
    /// The reviewed-derivative path has an item, a stored class and a preview,
    /// and `cloud_derivative::tier_allows` asks the policy about that exact
    /// pair. Embedding has a `&[String]` and nothing else. So this is loopback
    /// or nothing until something gives it an item to reason about — cloud
    /// embeddings are a design conversation, not a config flip. Nothing
    /// configured today embeds remotely, so the narrowing costs nothing now and
    /// would cost a redesign after the first feature built on the gap.
    fn refuse_ungoverned_cloud_call(&self, what: &str) -> Result<(), String> {
        if self.is_loopback() {
            return Ok(());
        }
        Err(format!(
            "this role would send {what} to {} (backend '{}'), and no data class travels with it \
             to check against a cloud policy: point the role at loopback, or route the content \
             through the reviewed cloud-derivative queue, which does carry one",
            self.backend.base_url, self.backend_name
        ))
    }

    pub fn embed(&self, texts: &[String], text_role: TextRole) -> Result<Vec<Vec<f32>>, String> {
        let inputs = texts
            .iter()
            .cloned()
            .map(|text| (text, text_role))
            .collect::<Vec<_>>();
        self.embed_mixed(&inputs)
    }

    /// Embeds query and document inputs in one batch while applying each
    /// input's declared role prefix. Retrieval consumers use this to keep both
    /// sides in one request and one vector space.
    pub fn embed_mixed(&self, inputs: &[(String, TextRole)]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        self.refuse_ungoverned_cloud_call("text to embed")?;
        let endpoint = self.embedding_endpoint();
        let client = axon_http::client(
            axon_http::Purpose::new("inference-embed"),
            std::time::Duration::from_secs(120),
        )
        .map_err(|error| format!("client build: {error}"))?;

        let mut request = client
            .post(&endpoint)
            .json(&self.request_body_mixed(inputs));
        if let Some(key) = self.bearer_key() {
            request = request.bearer_auth(key);
        }

        let response = request
            .send()
            .map_err(|error| format!("POST {endpoint}: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("{endpoint} returned HTTP {}", response.status()));
        }

        let vectors = match self.backend.api {
            Api::Ollama => {
                response
                    .json::<OllamaEmbedResponse>()
                    .map_err(|error| format!("parse ollama response: {error}"))?
                    .embeddings
            }
            Api::OpenAi => {
                let mut body = response
                    .json::<OpenAiEmbedResponse>()
                    .map_err(|error| format!("parse openai response: {error}"))?;
                body.data.sort_by_key(|datum| datum.index);
                body.data.into_iter().map(|datum| datum.embedding).collect()
            }
        };

        if vectors.len() != inputs.len() {
            return Err(format!(
                "expected {} embeddings, got {}",
                inputs.len(),
                vectors.len()
            ));
        }
        Ok(vectors)
    }
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedDatum>,
}

#[derive(Deserialize)]
struct OpenAiEmbedDatum {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[derive(Deserialize)]
struct RerankResult {
    index: usize,
    relevance_score: f32,
}

fn rerank_scores_in_input_order(
    results: Vec<RerankResult>,
    document_count: usize,
) -> Result<Vec<f32>, String> {
    if results.len() != document_count {
        return Err(format!(
            "expected {document_count} rerank results, got {}",
            results.len()
        ));
    }
    let mut scores = vec![None; document_count];
    for result in results {
        if result.index >= document_count {
            return Err(format!(
                "rerank result index {} is out of range",
                result.index
            ));
        }
        if !result.relevance_score.is_finite() || !(0.0..=1.0).contains(&result.relevance_score) {
            return Err(format!(
                "rerank score for index {} is outside 0..=1",
                result.index
            ));
        }
        if scores[result.index]
            .replace(result.relevance_score)
            .is_some()
        {
            return Err(format!("duplicate rerank result index {}", result.index));
        }
    }
    scores
        .into_iter()
        .enumerate()
        .map(|(index, score)| score.ok_or_else(|| format!("missing rerank result index {index}")))
        .collect()
}

/// Reads a bearer key out of a file. JSON content yields `.auth.api_key` so
/// this can point straight at `~/.omlx/settings.json`; anything else is taken
/// as the trimmed key itself. Same contract comms established.
pub fn api_key_from_file(path: Option<&str>) -> Option<String> {
    let raw = path?;
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        PathBuf::from(std::env::var("HOME").ok()?).join(rest)
    } else {
        PathBuf::from(raw)
    };
    let content = std::fs::read_to_string(expanded).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return json
            .get("auth")
            .and_then(|auth| auth.get("api_key"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    // ── Q39: the trusted-peer tier ─────────────────────────────────────

    fn peer_role(base_url: &str, provided_by: Option<&str>, trusted_peer: bool) -> ResolvedRole {
        ResolvedRole {
            backend_name: "peer".into(),
            backend: Backend {
                api: Api::OpenAi,
                base_url: base_url.into(),
                api_key_file: None,
                provided_by: provided_by.map(str::to_string),
            },
            model: "m".into(),
            provider_name: None,
            cloud_data_tier: None,
            billing_mode: None,
            failover_priority: None,
            max_requests_per_day: None,
            max_input_tokens: None,
            credit_expires_on: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
            chat_template_kwargs: None,
            request_overrides: None,
            trusted_peer,
        }
    }

    #[test]
    fn a_tailnet_peer_is_not_a_cloud_provider() {
        // The trap Q39 names. `tailscale cert` gives the operator's own machine
        // a real https:// certificate, and without the third term the two-tier
        // test classifies it as a cloud endpoint and demands a reviewed policy.
        let url = "https://lars-mac.tail2df5b3.ts.net:11434";
        let undeclared = peer_role(url, None, false);
        assert!(
            undeclared.is_cloud_endpoint(),
            "an undeclared https host stays a cloud endpoint — that is the safe default"
        );
        assert!(!undeclared.trusted_for_every_class());

        let declared = peer_role(url, Some("lars-mac"), true);
        assert!(
            !declared.is_cloud_endpoint(),
            "the operator's own Mac is not a provider"
        );
        assert!(
            declared.trusted_for_every_class(),
            "trusted exactly as loopback is"
        );
        assert!(!declared.is_loopback(), "and it is still not this machine");
    }

    #[test]
    fn a_trusted_peer_does_not_share_this_machines_gpu() {
        // The two questions one predicate used to answer. A trusted peer may see
        // any class AND must not queue behind this host's admission gate.
        let peer = peer_role("https://peer.example.ts.net", Some("peer"), true);
        assert!(peer.trusted_for_every_class());
        assert!(!peer.is_loopback());

        let here = peer_role("http://127.0.0.1:11434", None, false);
        assert!(here.trusted_for_every_class());
        assert!(here.is_loopback());
    }

    #[test]
    fn naming_a_host_is_not_trusting_it() {
        // provided_by alone grants nothing: the intersection with the declared
        // list is the permission. [nvidia-nim] lives in the same overlay file
        // and is emphatically not this operator's hardware.
        let named_only = peer_role("https://api.example.com/v1", Some("nvidia-nim"), false);
        assert!(named_only.is_cloud_endpoint());
        assert!(!named_only.trusted_for_every_class());
    }

    #[test]
    fn the_third_tier_says_so_in_its_label() {
        let peer = peer_role("https://peer.example.ts.net", Some("peer"), true);
        assert_eq!(
            peer.provider_label(),
            "OpenAI-compatible trusted-peer endpoint"
        );
        // Reported as neither "local" (it is not this machine) nor "cloud"
        // (which is the misclassification Q39 exists to fix).
        assert!(!peer.provider_label().contains("local"));
        assert!(!peer.provider_label().contains("cloud"));
    }

    #[test]
    fn an_empty_declaration_trusts_nothing() {
        assert!(parse_trusted_peers("").is_empty());
        assert!(parse_trusted_peers("   ").is_empty());
        // The one that matters: a stray separator must not produce the empty id,
        // which a backend with no provided_by would match.
        assert!(parse_trusted_peers(",,").is_empty());
        assert_eq!(parse_trusted_peers("a, b ,,c"), vec!["a", "b", "c"]);
    }

    use super::*;

    const SAMPLE: &str = r#"{
      "backends": {
        "omlx":   { "api": "openai", "base_url": "http://127.0.0.1:8000/v1",
                    "api_key_file": "~/.omlx/settings.json" },
        "ollama": { "api": "ollama", "base_url": "http://127.0.0.1:11434" }
      },
      "roles": {
        "embedding": { "backend": "omlx", "model": "multilingual-e5-base-mlx",
                       "query_prefix": "query: ", "document_prefix": "passage: ",
                       "on_backend": {
                         "ollama": { "model": "nomic-embed-text",
                                     "query_prefix": "search_query: ",
                                     "document_prefix": "search_document: " } } },
        "reranking": { "backend": "omlx", "model": "bge-reranker-v2-m3-mlx" }
      }
    }"#;

    fn config() -> InferenceConfig {
        SAMPLE.parse::<InferenceConfig>().expect("sample parses")
    }

    fn write_temp_key(name: &str, content: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("axon-inference-key-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("key");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn a_role_resolves_to_its_backend() {
        let role = config().role("embedding").expect("declared");
        assert_eq!(role.backend_name, "omlx");
        assert_eq!(role.backend.api, Api::OpenAi);
        assert_eq!(role.model, "multilingual-e5-base-mlx");
    }

    #[test]
    fn an_undeclared_role_is_none_not_a_panic() {
        assert!(config().role("summarization").is_none());
    }

    #[test]
    fn an_undeclared_ocr_role_resolves_to_nothing() {
        // Rung 3 of the extraction ladder, absent BY CONTRACT rather than by
        // accident. `libs/extraction/src/ocr_role.rs` degrades from this `None`;
        // an engine may only be declared here after it clears the frozen corpus
        // at libs/extraction/eval/, which is how the embedding and reranking
        // models entered.
        assert!(config().role("ocr").is_none());
    }

    #[test]
    fn cloud_roles_are_explicit_sorted_and_require_https_off_host() {
        let mut cfg = config();
        cfg.backends.insert(
            "hosted".into(),
            Backend {
                api: Api::OpenAi,
                base_url: "https://api.example.com/v1".into(),
                api_key_file: Some("/private/key-file".into()),
                provided_by: None,
            },
        );
        cfg.roles.insert(
            "cloud_summarization".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: Some("Hosted test".into()),
                cloud_data_tier: Some(CloudDataTier::PseudonymizedPersonal),
                billing_mode: Some(BillingMode::FreeOnly),
                failover_priority: Some(10),
                max_requests_per_day: Some(20),
                max_input_tokens: Some(8_000),
                credit_expires_on: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        let mut backup = cfg.roles["cloud_summarization"].clone();
        backup.model = "backup-model".into();
        backup.failover_priority = Some(5);
        cfg.roles.insert("cloud_backup".into(), backup.clone());
        backup.cloud_data_tier = Some(CloudDataTier::Public);
        cfg.roles.insert("cloud_public".into(), backup);

        let roles = cfg.roles_with_prefix("cloud_");
        assert_eq!(roles.len(), 3);
        let selected = cfg.role("cloud_summarization").unwrap();
        assert!(selected.is_cloud_endpoint());
        assert!(selected.has_cloud_policy());
        assert!(!selected.dispatch_ready());
        assert_eq!(
            selected.cloud_data_tier,
            Some(CloudDataTier::PseudonymizedPersonal)
        );
        assert_eq!(
            selected.provider_label(),
            "OpenAI-compatible cloud endpoint"
        );
        assert_eq!(
            cfg.cloud_failover_roles("cloud_summarization")
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec!["cloud_summarization", "cloud_backup"],
            "the chosen role stays first, and a NARROWER tier never enters failover: \
             cloud_public is reviewed for public text only and must never be offered a \
             redacted personal derivative"
        );

        // The other direction, which is not symmetric and was wrong until 2026-08-30.
        // `tier_allows` says a pseudonymized_personal role admits a public passthrough, so
        // those two roles can take this job -- and `tier_rank` in comms' cloud_run selects
        // the NARROWEST role first, which made cloud_public the selected role and, under an
        // equality filter, the only candidate. One rate-limited provider then burned the
        // job's whole attempt budget with two healthy providers sitting idle.
        assert_eq!(
            cfg.cloud_failover_roles("cloud_public")
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            vec!["cloud_public", "cloud_backup", "cloud_summarization"],
            "a public job falls over to the wider tier, selected role still first and the \
             rest by failover_priority (backup is 5, summarization is 10)"
        );
        assert!(CloudDataTier::PseudonymizedPersonal.admits_at_least(CloudDataTier::Public));
        assert!(!CloudDataTier::Public.admits_at_least(CloudDataTier::PseudonymizedPersonal));
        assert!(config().role("embedding").unwrap().is_loopback());
    }

    #[test]
    fn the_endpoint_follows_the_api_shape_not_the_backend_name() {
        let mut cfg = config();
        let openai = cfg.role("embedding").unwrap();
        assert_eq!(
            openai.embedding_endpoint(),
            "http://127.0.0.1:8000/v1/embeddings"
        );

        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        let ollama = cfg.role("embedding").unwrap();
        assert_eq!(
            ollama.embedding_endpoint(),
            "http://127.0.0.1:11434/api/embed"
        );
    }

    #[test]
    fn the_role_prefix_reaches_the_request_body() {
        let role = config().role("embedding").unwrap();
        let query = role.request_body(&["Kanutour".into()], TextRole::Query);
        assert_eq!(query["input"][0], "query: Kanutour");

        let document = role.request_body(&["Kanutour".into()], TextRole::Document);
        assert_eq!(document["input"][0], "passage: Kanutour");
    }

    #[test]
    fn mixed_embedding_inputs_keep_query_and_document_roles() {
        let role = config().role("embedding").unwrap();
        let body = role.request_body_mixed(&[
            ("lens".into(), TextRole::Query),
            ("item".into(), TextRole::Document),
        ]);
        assert_eq!(body["input"][0], "query: lens");
        assert_eq!(body["input"][1], "passage: item");
    }

    #[test]
    fn rerank_request_uses_the_cohere_jina_wire_shape() {
        let role = config().role("reranking").unwrap();
        assert_eq!(
            role.rerank_endpoint().unwrap(),
            "http://127.0.0.1:8000/v1/rerank"
        );
        let body = role.rerank_request_body("lens", &["first".into(), "second".into()]);
        assert_eq!(body["model"], "bge-reranker-v2-m3-mlx");
        assert_eq!(body["query"], "lens");
        assert_eq!(body["documents"][1], "second");
        assert_eq!(body["top_n"], 2);
        assert_eq!(body["return_documents"], false);
    }

    #[test]
    fn rerank_results_are_restored_to_document_order_and_validated() {
        let scores = rerank_scores_in_input_order(
            vec![
                RerankResult {
                    index: 1,
                    relevance_score: 0.9,
                },
                RerankResult {
                    index: 0,
                    relevance_score: 0.2,
                },
            ],
            2,
        )
        .unwrap();
        assert_eq!(scores, vec![0.2, 0.9]);
        assert!(rerank_scores_in_input_order(
            vec![
                RerankResult {
                    index: 0,
                    relevance_score: 0.2
                },
                RerankResult {
                    index: 0,
                    relevance_score: 0.9
                },
            ],
            2,
        )
        .is_err());
    }

    #[test]
    fn chat_endpoint_follows_the_backend_api_shape() {
        let mut cfg = config();
        assert_eq!(
            cfg.role("embedding").unwrap().chat_completions_endpoint(),
            "http://127.0.0.1:8000/v1/chat/completions"
        );
        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        assert_eq!(
            cfg.role("embedding").unwrap().chat_completions_endpoint(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }

    #[test]
    fn installed_model_basename_matches_a_namespaced_config_id() {
        assert!(model_ids_match(
            "mlx-community/gemma-4-26b-a4b-it-4bit",
            "gemma-4-26b-a4b-it-4bit"
        ));
        assert!(model_ids_match(
            "nomic-embed-text",
            "nomic-embed-text:latest"
        ));
        assert!(!model_ids_match("owner-a/model", "owner-b/model"));
    }

    #[test]
    fn bearer_key_reads_json_and_raw_secret_references() {
        let json = write_temp_key(
            "json",
            "{\"auth\": {\"api_key\": \"sk-json\"}, \"other\": 1}",
        );
        let raw = write_temp_key("raw", "  sk-raw\n");
        assert_eq!(api_key_from_file(json.to_str()), Some("sk-json".into()));
        assert_eq!(api_key_from_file(raw.to_str()), Some("sk-raw".into()));
        let _ = std::fs::remove_dir_all(json.parent().unwrap());
        let _ = std::fs::remove_dir_all(raw.parent().unwrap());
    }

    #[test]
    fn bearer_key_rejects_empty_or_wrong_json_shape() {
        let empty = write_temp_key("empty", "  \n");
        let wrong = write_temp_key("wrong-json", "{\"auth\": {}}");
        assert_eq!(api_key_from_file(empty.to_str()), None);
        assert_eq!(api_key_from_file(wrong.to_str()), None);
        let _ = std::fs::remove_dir_all(empty.parent().unwrap());
        let _ = std::fs::remove_dir_all(wrong.parent().unwrap());
    }

    #[test]
    fn relative_key_files_resolve_beside_the_private_config() {
        let directory =
            std::env::temp_dir().join(format!("axon-inference-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let config_path = directory.join("inference.json");
        std::fs::write(
            &config_path,
            r#"{
              "backends": {
                "hosted": {
                  "api": "openai",
                  "base_url": "https://api.example.com/v1",
                  "api_key_file": "runtime-secrets/provider-key"
                }
              },
              "roles": {}
            }"#,
        )
        .unwrap();

        let config = InferenceConfig::from_path(&config_path);
        assert_eq!(
            config.backends["hosted"].api_key_file.as_deref(),
            Some(
                directory
                    .join("runtime-secrets/provider-key")
                    .to_str()
                    .unwrap()
            )
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cloud_policy_requires_name_tier_and_billing_mode() {
        let mut cfg = config();
        cfg.backends.insert(
            "hosted".into(),
            Backend {
                api: Api::OpenAi,
                base_url: "https://api.example.com/v1".into(),
                api_key_file: None,
                provided_by: None,
            },
        );
        cfg.roles.insert(
            "cloud_incomplete".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: None,
                cloud_data_tier: Some(CloudDataTier::Public),
                billing_mode: Some(BillingMode::FreeOnly),
                failover_priority: None,
                max_requests_per_day: None,
                max_input_tokens: None,
                credit_expires_on: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        assert!(!cfg.role("cloud_incomplete").unwrap().has_cloud_policy());
    }

    /// `embed`/`rerank` are transport-agnostic and consulted no cloud policy,
    /// while `has_cloud_policy()` was only ever called from comms' cloud
    /// handlers. So a role pointed at an https endpoint reached a provider with
    /// no preview, no redaction, no approval, no budget check and no ledger
    /// entry: the whole review discipline bypassed by a path that never mentions
    /// the cloud.
    ///
    /// A declared policy is no longer enough either. A policy says which class
    /// of content a provider may receive; these two functions take a `&[String]`
    /// and are handed no class at all, so there is nothing to check the policy
    /// against. Loopback or nothing.
    #[test]
    fn nothing_embeds_off_this_machine_policy_or_no_policy() {
        let mut cfg = config();
        cfg.backends.insert(
            "hosted".into(),
            Backend {
                api: Api::OpenAi,
                base_url: "https://api.example.com/v1".into(),
                api_key_file: None,
                provided_by: None,
            },
        );
        cfg.roles.insert(
            "ungoverned".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: None,
                cloud_data_tier: None,
                billing_mode: None,
                failover_priority: None,
                max_requests_per_day: None,
                max_input_tokens: None,
                credit_expires_on: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        let role = cfg.role("ungoverned").unwrap();
        assert!(!role.has_cloud_policy());
        assert!(!role.is_loopback());

        let refused = role
            .refuse_ungoverned_cloud_call("text")
            .expect_err("a remote endpoint must be refused");
        // Named, so the fix is obvious from the message rather than the source.
        assert!(refused.contains("data class"), "got: {refused}");
        assert!(refused.contains("api.example.com"), "got: {refused}");

        // And a *fully governed* cloud role is refused just the same. This is
        // the case the earlier version let through: a reviewed policy on the
        // role says which class the provider may receive, and `embed` never
        // learns the class of what it was handed.
        cfg.roles.insert(
            "cloud_governed".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: Some("Hosted test".into()),
                cloud_data_tier: Some(CloudDataTier::PseudonymizedPersonal),
                billing_mode: Some(BillingMode::FreeOnly),
                failover_priority: Some(10),
                max_requests_per_day: Some(20),
                max_input_tokens: Some(8_000),
                credit_expires_on: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        let governed = cfg.role("cloud_governed").unwrap();
        assert!(governed.has_cloud_policy());
        assert!(governed
            .refuse_ungoverned_cloud_call("text to embed")
            .is_err());

        // A local role is unaffected, which is what makes this cheap to land:
        // every configured embedding role today is loopback.
        let local = cfg.role("embedding").or_else(|| cfg.role("summarization"));
        if let Some(local) = local.filter(|role| role.is_loopback()) {
            assert!(local.refuse_ungoverned_cloud_call("text").is_ok());
        }
    }

    #[test]
    fn prepaid_credit_policy_requires_a_valid_non_expired_date() {
        let mut cfg = config();
        cfg.backends.insert(
            "hosted".into(),
            Backend {
                api: Api::OpenAi,
                base_url: "https://api.example.com/v1".into(),
                api_key_file: None,
                provided_by: None,
            },
        );
        cfg.roles.insert(
            "cloud_credit".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: Some("Credit provider".into()),
                cloud_data_tier: Some(CloudDataTier::Public),
                billing_mode: Some(BillingMode::PrepaidCredit),
                failover_priority: Some(20),
                max_requests_per_day: Some(5),
                max_input_tokens: Some(2_000),
                credit_expires_on: Some("2026-08-31".into()),
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        let role = cfg.role("cloud_credit").unwrap();
        assert!(role.has_cloud_policy());
        assert!(role.billing_active_on("2026-08-31"));
        assert!(!role.billing_active_on("2026-09-01"));

        cfg.roles.get_mut("cloud_credit").unwrap().credit_expires_on = Some("2026-02-30".into());
        assert!(!cfg.role("cloud_credit").unwrap().has_cloud_policy());
        assert!(!valid_iso_date("2026-é-01"));
    }

    /// Ollama keeps a model resident for five minutes unless told otherwise.
    #[test]
    fn the_ollama_shape_asks_for_an_immediate_unload() {
        let mut cfg = config();
        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        let body = cfg
            .role("embedding")
            .unwrap()
            .request_body(&["x".into()], TextRole::Query);
        assert_eq!(body["keep_alive"], 0);
    }

    /// The claim that makes a backend switch safe rather than silently wrong.
    #[test]
    fn the_cache_key_changes_when_the_producing_model_changes() {
        let mut cfg = config();
        let omlx = cfg.role("embedding").unwrap().cache_key();

        cfg.roles.get_mut("embedding").unwrap().backend = "ollama".into();
        cfg.roles.get_mut("embedding").unwrap().model = "nomic-embed-text".into();
        let ollama = cfg.role("embedding").unwrap().cache_key();

        assert_ne!(omlx, ollama, "a cache keyed on this would swap silently");
        assert_eq!(omlx, "omlx:multilingual-e5-base-mlx");
    }

    #[test]
    fn a_missing_config_file_is_an_empty_config_not_a_failure() {
        let cfg = InferenceConfig::from_path(Path::new("/nonexistent/inference.json"));
        assert!(cfg.roles.is_empty() && cfg.backends.is_empty());
    }

    /// The Intel always-on host: Ollama is the only runtime it has.
    #[test]
    fn the_machine_backend_carries_the_model_and_its_prefixes_with_it() {
        let cfg = config();
        let ollama = cfg.role_on("embedding", Some("ollama")).expect("declared");
        assert_eq!(ollama.backend_name, "ollama");
        assert_eq!(ollama.model, "nomic-embed-text");
        assert_eq!(
            (
                ollama.query_prefix.as_str(),
                ollama.document_prefix.as_str()
            ),
            ("search_query: ", "search_document: "),
            "e5's prefixes on nomic cost retrieval quality and raise nothing"
        );
        assert_eq!(
            ollama.embedding_endpoint(),
            "http://127.0.0.1:11434/api/embed"
        );

        for unchanged in [None, Some("omlx")] {
            let role = cfg.role_on("embedding", unchanged).expect("declared");
            assert_eq!(role.backend_name, "omlx");
            assert_eq!(role.model, "multilingual-e5-base-mlx");
            assert_eq!(role.query_prefix, "query: ");
        }
    }

    /// Swapping the backend alone would leave an MLX id addressed to Ollama.
    /// Ollama has no `/v1/rerank` at all, so there is nothing to name here.
    #[test]
    fn a_role_with_no_model_on_the_machine_backend_is_none_not_a_wrong_name() {
        assert!(config().role_on("reranking", Some("ollama")).is_none());
        assert_eq!(
            config().role_on("reranking", None).unwrap().model,
            "bge-reranker-v2-m3-mlx"
        );
    }

    /// The override says which *local* runtime exists. A hosted backend answers
    /// from every machine, and moving a reviewed cloud role onto whatever this
    /// host runs would hand its policy to a model that never passed review.
    #[test]
    fn the_machine_backend_leaves_a_hosted_role_where_it_is() {
        let mut cfg = config();
        cfg.backends.insert(
            "hosted".into(),
            Backend {
                api: Api::OpenAi,
                base_url: "https://api.example.com/v1".into(),
                api_key_file: None,
                provided_by: None,
            },
        );
        cfg.roles.insert(
            "cloud_summarization".into(),
            Role {
                backend: "hosted".into(),
                model: "hosted-model".into(),
                provider_name: Some("Hosted test".into()),
                cloud_data_tier: Some(CloudDataTier::Public),
                billing_mode: Some(BillingMode::FreeOnly),
                failover_priority: Some(10),
                max_requests_per_day: Some(20),
                max_input_tokens: Some(8_000),
                credit_expires_on: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                chat_template_kwargs: None,
                ..Default::default()
            },
        );
        let role = cfg
            .role_on("cloud_summarization", Some("ollama"))
            .expect("a hosted role survives a machine that overrides its local runtime");
        assert_eq!(role.backend_name, "hosted");
        assert_eq!(role.model, "hosted-model");
        assert!(role.has_cloud_policy());
    }

    /// The claim that makes the override safe rather than silently wrong: two
    /// machines running the same role still name different producers.
    #[test]
    fn the_cache_key_names_the_producer_on_both_sides_of_the_override() {
        let cfg = config();
        assert_eq!(
            cfg.role_on("embedding", None).unwrap().cache_key(),
            "omlx:multilingual-e5-base-mlx"
        );
        assert_eq!(
            cfg.role_on("embedding", Some("ollama"))
                .unwrap()
                .cache_key(),
            "ollama:nomic-embed-text"
        );
    }

    #[test]
    fn a_role_pointing_at_an_undeclared_backend_resolves_to_none() {
        let mut cfg = config();
        cfg.roles.get_mut("embedding").unwrap().backend = "vllm".into();
        assert!(cfg.role("embedding").is_none());
    }
}
