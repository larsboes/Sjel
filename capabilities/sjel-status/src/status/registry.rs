use super::*;

pub(crate) static STARTED_AT: OnceLock<Instant> = OnceLock::new();

/// One service as the registry renders it. Every field is a string because the
/// manifests are single-line TOML and the shell emitter does not guess types.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Service {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) scope: String,
    pub(crate) port: String,
    pub(crate) health_path: String,
    /// Liveness answers "is the process there", which is not what an availability surface
    /// is asking. A capability that reaches a database declares this too, and answers it by
    /// touching that database. Empty for everything that has no dependency worth checking —
    /// and an empty one means this surface keeps polling `health_path`, exactly as before.
    #[serde(default)]
    pub(crate) ready_path: String,
    pub(crate) panel_port: String,
    pub(crate) panel_path: String,
    pub(crate) autostart: String,
    /// Seconds a panel may go unread before it is stopped. Empty means never — a
    /// capability opts into being reaped, it is not opted in by having a panel.
    #[serde(default)]
    pub(crate) idle_timeout: String,
    /// The systems.toml id a backup lands on. Non-empty is this surface's whole
    /// definition of "has a backup contract", because `tools/backup.sh` refuses a run
    /// without one. Never projected outward — an id is one hop from the private
    /// coordinates in the overlay's systems.local.toml, and no backup UI needs it.
    #[serde(default)]
    pub(crate) backup_target: String,
    /// Declared when the capability's data is a SQLite file the container owns. This is
    /// the field that decides whether a backup run stops the service: `backup.sh` holds a
    /// capability down for exactly this case, to take the cold copy. Read as a contract
    /// rather than matching a service name, so a capability that later declares one
    /// inherits the warning without anything here changing.
    #[serde(default)]
    pub(crate) backup_sqlite: String,
    /// The same data, copied while it is open — `sqlite3 .backup`, no service hold. It is
    /// the field above's counter-example and is why `holds_service` keys on that one
    /// alone: capabilities/store's file is read by every capability at once, so there is
    /// no single service whose stopping would make the copy cold. Projected outward with
    /// the rest of the row, because a surface that offers a run should be able to say
    /// which of the two it is about to take.
    #[serde(default)]
    pub(crate) backup_sqlite_online: String,
    /// What timely MEANS for this data — how often it should be backed up, and the
    /// age past which the last backup is a problem rather than merely due. Two numbers
    /// because "you could" and "you have a problem" are different answers.
    #[serde(default)]
    pub(crate) backup_advise_days: String,
    #[serde(default)]
    pub(crate) backup_stale_days: String,
    /// Set only where `scope == "external"`: the base URL of the deployment that actually
    /// provides this capability, resolved by the active overlay (retired-tracker#169). Empty
    /// for everything this machine runs, which is the normal case and the one that must not
    /// change — an empty endpoint means loopback, exactly as before.
    #[serde(default)]
    pub(crate) endpoint: String,
    /// Narrows the uniform `/<name>` mount to `/<name>/api`, for a capability that also serves
    /// something at its root which the shell must not shadow. Emitted by
    /// `tools/capability.sh registry` since the Vite proxy needed it; deserialised here since
    /// `src/proxy.rs` became the second reader.
    #[serde(default)]
    pub(crate) proxy_api_only: String,
    /// Prefixes that pass through UNSTRIPPED — transit's `/api`, scouting's `/discover`. Paths
    /// that predate the uniform rule and are declared per capability rather than guessed.
    #[serde(default)]
    pub(crate) proxy_extra: Vec<String>,
    #[serde(default)]
    pub(crate) requires: Vec<String>,
}

impl Service {
    /// A capability this machine consumes rather than runs. The registry decides this; the
    /// question is asked here often enough that spelling the comparison out at each call site
    /// would be four chances to spell it differently.
    pub(crate) fn is_external(&self) -> bool {
        self.scope == "external"
    }

    /// Where to reach one of this capability's probe paths, or `None` when there is nothing
    /// to poll — a capability without such a surface is reported as unknown rather than
    /// silently "down".
    ///
    /// Two shapes, and the port is what separates them. A local capability's URL is loopback
    /// plus the port it publishes here. An external one has no port on this machine at all —
    /// the registry blanks it, because a port number is a fact about the host that binds it —
    /// so its URL is the resolved endpoint plus the same path. The probe paths are the only
    /// manifest fields the two have in common, which is the whole reason they are the only
    /// ones a consuming machine inherits.
    pub(crate) fn probe_url(&self, path: &str) -> Option<String> {
        if path.is_empty() {
            return None;
        }
        if self.is_external() {
            if self.endpoint.is_empty() {
                return None;
            }
            return Some(format!("{}{}", self.endpoint, path));
        }
        if self.port.is_empty() {
            return None;
        }
        Some(format!("http://127.0.0.1:{}{}", self.port, path))
    }

    pub(crate) fn health_url(&self) -> Option<String> {
        self.probe_url(&self.health_path)
    }

    /// What availability should be judged on: readiness where a capability declares it,
    /// liveness everywhere else.
    ///
    /// The fallback is the compatibility contract. Until 2026-08-07 this surface polled
    /// `health_path` for every capability, and five database-backed ones answered it from a
    /// stateless handler that could not observe their database — so they reported themselves up
    /// through an outage that made every query behind them fail (#126). A capability that
    /// declares no `ready_path` still behaves exactly as it did.
    pub(crate) fn readiness_url(&self) -> Option<String> {
        self.probe_url(&self.ready_path)
            .or_else(|| self.health_url())
    }

    /// How long this panel may go unread, when it says so at all.
    ///
    /// Three conditions, all required, and each one is a separate way the reaper could
    /// otherwise stop something it has no business stopping: a panel to look at, an
    /// explicit timeout, and NOT autostart — a capability the machine is supposed to
    /// keep running is never idle by definition.
    pub(crate) fn idle_timeout_secs(&self) -> Option<u64> {
        if self.panel_port.is_empty() || self.autostart == "true" {
            return None;
        }
        self.idle_timeout.parse::<u64>().ok().filter(|s| *s > 0)
    }

    /// `None` for a capability with nothing to back up, which is most of them.
    ///
    /// Presence is `backup_target`, not the day fields: a capability may decline to say
    /// what timely means and still have a backup contract, and treating that as "no
    /// contract" would hide it from the only surface that could tell you it has never
    /// run. The reverse — day fields without a target — is a manifest `backup.sh`
    /// itself rejects, so there is nothing to represent.
    pub(crate) fn backup_contract(&self) -> Option<BackupContract> {
        if self.backup_target.is_empty() {
            return None;
        }
        Some(BackupContract {
            holds_service: !self.backup_sqlite.is_empty(),
            advise_days: self.backup_advise_days.parse::<u64>().ok(),
            stale_days: self.backup_stale_days.parse::<u64>().ok(),
        })
    }
}

/// What the manifest declares about backing this capability up, with the private half
/// already dropped. Every field here is safe to render.
#[derive(Clone, Copy, Serialize)]
pub(crate) struct BackupContract {
    /// A run stops the capability. The UI must say so BEFORE asking for confirmation,
    /// derived from the contract rather than a hardcoded service name.
    pub(crate) holds_service: bool,
    pub(crate) advise_days: Option<u64>,
    pub(crate) stale_days: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct CapabilityStatus {
    pub(crate) up: bool,
    pub(crate) url: String,
}

#[derive(Serialize)]
pub(crate) struct AxonStatusHealth {
    pub(crate) ok: bool,
    pub(crate) version: String,
    pub(crate) uptime_seconds: u64,
    pub(crate) capabilities: HashMap<String, CapabilityStatus>,
}

#[derive(Serialize)]
pub(crate) struct CapabilityView {
    #[serde(flatten)]
    pub(crate) service: Service,
    /// `None` — not false — when the manifest declares no health surface. A running
    /// container with nothing to poll is unknown from here, and reporting it as down
    /// would be a lie the shell then renders as a red dot.
    pub(crate) up: Option<bool>,
    pub(crate) health_url: Option<String>,
}

/// The repo this binary belongs to. Passed in rather than derived from the binary's
/// own path: that path is `target/release/`, which locates a build output rather than
/// the checkout, and says nothing once the binary is copied anywhere else.
/// `tools/lib/paths.sh` exports it, so anything started by `service-runner.sh`
/// inherits it; a hand-started run has to say where it is.
pub(crate) fn axon_root() -> Result<PathBuf, String> {
    sjel_config::env_var("SJEL_ROOT")
        .map(PathBuf::from)
        .map_err(|_| {
            "SJEL_ROOT is not set — start this through tools/service-runner.sh, or export it"
                .to_string()
        })
}

/// Every file `tools/capability.sh registry` reads, with its mtime — the cache key
/// below. `None` when the enabled set's location is unknown, which disables caching
/// rather than caching against a key that cannot see the enabled set change.
///
/// Globbed on every call rather than remembered once: a cache keyed on a remembered
/// file list cannot notice a manifest that did not exist when the list was built, and
/// adding a capability is exactly that case. Directory reads and stats, no forks.
// `std::path::Path` stays qualified: `axum::extract::Path` owns the bare name in this
// file, same as in `reap_idle_panels` below.
pub(crate) fn manifest_key(root: &std::path::Path) -> Option<Vec<(PathBuf, SystemTime)>> {
    // Same env contract as SJEL_ROOT above: tools/lib/paths.sh exports it, so anything
    // started by service-runner.sh inherits it.
    let machine_toml = sjel_config::env_var("SJEL_MACHINE_TOML").ok()?;
    let mut paths = vec![PathBuf::from(machine_toml)];

    // Spine services at the repo root, capabilities under capabilities/, and the same
    // directory in the active overlay — the globs capability.sh itself uses
    // (`_spine_names`, `_cap_dirs`). A top-level service.toml IS the declaration
    // (README.md#three-architectural-nouns), so there is no list to keep in sync here
    // either.
    //
    // The overlay directory belongs in the key even though the registry it guards is
    // rendered by capability.sh: this key is what notices a manifest appearing, and an
    // overlay capability that is added while the process runs would otherwise stay
    // invisible until a restart. SJEL_OVERLAY_CAPS_DIR comes from tools/lib/paths.sh,
    // and its absence is normal — a deployment need not own any overlay capability.
    let mut dirs = vec![root.to_path_buf(), root.join("capabilities")];
    if let Ok(overlay_caps) = sjel_config::env_var("SJEL_OVERLAY_CAPS_DIR") {
        dirs.push(PathBuf::from(overlay_caps));
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let manifest = entry.path().join("service.toml");
            if manifest.is_file() {
                paths.push(manifest);
            }
        }
    }

    paths.sort();
    Some(
        paths
            .into_iter()
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);
                (p, mtime)
            })
            .collect(),
    )
}

pub(crate) type RegistrySnapshot = (Vec<(PathBuf, SystemTime)>, Vec<Service>);
pub(crate) static REGISTRY_CACHE: OnceLock<Mutex<Option<RegistrySnapshot>>> = OnceLock::new();

/// The enabled set, from the manifests, memoized against their mtimes.
///
/// The shell call underneath costs ~440ms on this machine — `tools/lib/toml.sh` forks a
/// grep and a sed per key per manifest, ~12 manifests × 9 keys — and every handler here
/// starts with it, including the `/capabilities` the dashboard's layout polls every 15s
/// on every page. That put ~440ms of dead time in front of every Axon page for an answer
/// that changes only when somebody edits a file.
///
/// Two concurrent misses may both run the script. That is left alone deliberately: the
/// script is a pure read, running it twice costs one extra fork, and avoiding it would
/// mean holding a lock across an await for no correctness gain.
/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
pub(crate) async fn registry() -> Result<Vec<Service>, String> {
    let root = axon_root()?;
    let key = manifest_key(&root);

    if let Some(key) = key.as_ref() {
        if let Ok(cache) = REGISTRY_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            if let Some((cached_key, services)) = cache.as_ref() {
                if cached_key == key {
                    return Ok(services.clone());
                }
            }
        }
    }

    let out = tokio::process::Command::new(root.join("tools/capability.sh"))
        .arg("registry")
        .output()
        .await
        .map_err(|e| format!("could not run tools/capability.sh: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tools/capability.sh registry failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let services: Vec<Service> = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("registry is not valid JSON: {e}"))?;

    if let Some(key) = key {
        if let Ok(mut cache) = REGISTRY_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = Some((key, services.clone()));
        }
    }
    Ok(services)
}
