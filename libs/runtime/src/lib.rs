//! Per-device admission, independent of capability installation and privacy policy.
//! Only sjel-status writes these files; workers reread them before each unit of work.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const POWER_MAX_AGE_SECONDS: u64 = 90;
const DEFERRED_PREFIX: &str = "deferred by runtime profile:";
static WRITER: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Selection {
    #[default]
    Auto,
    Normal,
    OnTheGo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    OtherLocalModels,
    RemoteModels,
    BulkIndexing,
    Transcription,
    MediaConversion,
}

impl Category {
    pub const ALL: [Self; 5] = [Self::OtherLocalModels, Self::RemoteModels, Self::BulkIndexing, Self::Transcription, Self::MediaConversion];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OtherLocalModels => "other-local-models",
            Self::RemoteModels => "remote-models",
            Self::BulkIndexing => "bulk-indexing",
            Self::Transcription => "transcription",
            Self::MediaConversion => "media-conversion",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Power {
    Ac,
    Battery,
    NoBattery,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub selection: Selection,
    #[serde(default)]
    pub manual_power: Option<Power>,
    #[serde(default)]
    pub allow: BTreeSet<Category>,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub selection: Option<Selection>,
    pub allow: Option<BTreeSet<Category>>,
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerSample {
    pub power: Power,
    pub observed_at: u64,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub device: String,
    pub configured: bool,
    pub selection: Selection,
    pub effective: Selection,
    pub power: Power,
    pub power_fresh: bool,
    pub allow: BTreeSet<Category>,
    pub revision: u64,
    pub detail: Option<String>,
}

impl Status {
    pub fn require(&self, category: Category) -> Result<(), Deferred> {
        if self.effective == Selection::Normal || self.allow.contains(&category) {
            Ok(())
        } else {
            Err(Deferred(format!("{DEFERRED_PREFIX} enable {} while On the go", category.as_str())))
        }
    }

    pub fn admit_model(&self, backend: &str, model: &str, local: bool) -> Result<(), Deferred> {
        if local && backend == "foundation-models" && model == "apple-foundationmodel" {
            return Ok(());
        }
        self.require(if local { Category::OtherLocalModels } else { Category::RemoteModels })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferred(pub String);

impl fmt::Display for Deferred {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Deferred {}

pub fn is_deferred(message: &str) -> bool {
    message.starts_with(DEFERRED_PREFIX)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Resolution is pure so power transitions and stale samples can be tested without hardware.
pub fn resolve(device: String, settings: Option<&Settings>, sample: Option<&PowerSample>, at: u64) -> Status {
    let power = sample.map_or(Power::Unknown, |s| s.power);
    let power_fresh = sample.is_some_and(|s| s.observed_at <= at && at - s.observed_at <= POWER_MAX_AGE_SECONDS);
    let configured = settings.is_some();
    let selection = settings.map_or(Selection::Normal, |s| {
        if s.selection != Selection::Auto && power_fresh && power != Power::Unknown && s.manual_power != Some(power) {
            Selection::Auto
        } else {
            s.selection
        }
    });
    let effective = match selection {
        Selection::Normal => Selection::Normal,
        Selection::OnTheGo => Selection::OnTheGo,
        Selection::Auto if power_fresh && matches!(power, Power::Ac | Power::NoBattery) => Selection::Normal,
        Selection::Auto => Selection::OnTheGo,
    };
    let detail = if configured && !power_fresh {
        Some("Power-source sample is missing or stale; Auto restricts heavy work.".into())
    } else {
        sample.and_then(|s| s.detail.clone())
    };
    Status {
        device, configured, selection, effective, power, power_fresh,
        allow: settings.map(|s| s.allow.clone()).unwrap_or_default(),
        revision: settings.map_or(0, |s| s.revision), detail,
    }
}

#[derive(Debug, Clone)]
pub struct Files {
    device: String,
    settings: PathBuf,
    power: PathBuf,
}

impl Files {
    pub fn for_device(root: &Path, device: &str) -> Result<Self, String> {
        if device.is_empty() || device.len() > 255 || !device.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) || matches!(device, "." | "..") {
            return Err("invalid runtime device identity".into());
        }
        Ok(Self {
            device: device.into(),
            settings: root.join("config/runtime").join(format!("{device}.json")),
            power: root.join("data/runtime").join(format!("{device}.json")),
        })
    }

    pub fn from_deployment() -> Result<Option<Self>, String> {
        let Some(root) = sjel_config::overlay_root() else { return Ok(None) };
        Self::for_device(&root, &device_identity()?).map(Some)
    }

    pub fn status(&self) -> Result<Status, String> {
        let settings: Option<Settings> = read_optional(&self.settings)?;
        // A broken sensor sample is unknown, never a reason to silently permit heavy work.
        let (sample, error) = match read_optional::<PowerSample>(&self.power) {
            Ok(sample) => (sample, None),
            Err(error) => (None, Some(error)),
        };
        let mut status = resolve(self.device.clone(), settings.as_ref(), sample.as_ref(), now());
        if let Some(error) = error { status.detail = Some(error); }
        Ok(status)
    }

    pub fn update(&self, update: Update) -> Result<Status, String> {
        if update.selection.is_none() && update.allow.is_none() {
            return Err("runtime update must specify selection or allow".into());
        }
        let _guard = WRITER.lock().map_err(|_| "runtime writer lock poisoned")?;
        let previous: Option<Settings> = read_optional(&self.settings)?;
        if update.expected_revision.is_some_and(|expected| expected != previous.as_ref().map_or(0, |s| s.revision)) {
            return Err("runtime preferences changed; reload before saving".into());
        }
        let sample = probe_power();
        self.refresh_with(&sample)?;
        let mut settings: Settings = read_optional(&self.settings)?.unwrap_or_default();
        if let Some(selection) = update.selection {
            settings.selection = selection;
            settings.manual_power = (selection != Selection::Auto).then_some(sample.power);
        }
        if let Some(allow) = update.allow { settings.allow = allow; }
        settings.revision = settings.revision.checked_add(1).ok_or("runtime revision exhausted")?;
        write_atomic(&self.settings, &settings)?;
        self.status()
    }

    pub fn refresh_power(&self) -> Result<(), String> {
        let _guard = WRITER.lock().map_err(|_| "runtime writer lock poisoned")?;
        let sample = probe_power();
        self.refresh_with(&sample)
    }

    fn refresh_with(&self, sample: &PowerSample) -> Result<(), String> {
        write_atomic(&self.power, sample)?;
        if let Some(mut settings) = read_optional::<Settings>(&self.settings)? {
            if settings.selection != Selection::Auto && sample.power != Power::Unknown && settings.manual_power != Some(sample.power) {
                // Persist the return to Auto: a later switch back to the original source
                // must not resurrect a manual selection from the previous battery session.
                settings.selection = Selection::Auto;
                settings.manual_power = None;
                settings.revision = settings.revision.checked_add(1).ok_or("runtime revision exhausted")?;
                write_atomic(&self.settings, &settings)?;
            }
        }
        Ok(())
    }
}

fn read_optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("runtime state {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("runtime state {}: {e}", path.display())),
    }
}

fn write_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("runtime state has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), stamp));
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result.map_err(|e| format!("runtime state write: {e}"))
}

fn device_identity() -> Result<String, String> {
    if let Ok(path) = sjel_config::env_var("SJEL_MACHINE_TOML") {
        let path = Path::new(&path);
        if path.parent().and_then(Path::file_name).is_some_and(|p| p == "machines") {
            return path.file_stem().and_then(|s| s.to_str()).map(str::to_owned).ok_or("runtime machine identity is not UTF-8".into());
        }
    }
    static HOST: OnceLock<Result<String, String>> = OnceLock::new();
    HOST.get_or_init(|| {
        let output = std::process::Command::new("hostname").output().map_err(|e| format!("runtime hostname: {e}"))?;
        if !output.status.success() { return Err("runtime hostname failed".into()); }
        String::from_utf8(output.stdout).map(|s| s.trim().to_owned()).map_err(|e| e.to_string())
    }).clone()
}

pub fn current() -> Result<Status, Deferred> {
    let result = (|| {
        match Files::from_deployment()? {
            Some(files) => files.status(),
            None => Ok(resolve("unconfigured".into(), None, None, now())),
        }
    })();
    result.map_err(|e: String| Deferred(format!("{DEFERRED_PREFIX} {e}")))
}

pub fn require(category: Category) -> Result<(), Deferred> {
    current()?.require(category)
}

pub fn admit_model(backend: &str, model: &str, local: bool) -> Result<(), Deferred> {
    current()?.admit_model(backend, model, local)
}

/// Recognize only the documented pmset source marker; malformed output is not AC.
pub fn parse_power(output: &str) -> Power {
    let Some(first) = output.lines().next() else { return Power::Unknown };
    match first.trim() {
        "Now drawing from 'Battery Power'" => Power::Battery,
        "Now drawing from 'AC Power'" if output.lines().skip(1).any(|l| l.contains("InternalBattery")) => Power::Ac,
        "Now drawing from 'AC Power'" if output.lines().skip(1).all(|l| l.trim().is_empty()) => Power::NoBattery,
        _ => Power::Unknown,
    }
}

#[cfg(target_os = "macos")]
fn probe_power() -> PowerSample {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let result = (|| -> Result<Power, String> {
        let mut child = Command::new("/usr/bin/pmset").args(["-g", "batt"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("power probe: {e}"))?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(_)) => return Err("power probe failed".into()),
                Ok(None) if started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(20)),
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(match result { Err(e) => format!("power probe: {e}"), _ => "power probe timed out".into() });
                }
            }
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        let text = std::str::from_utf8(&output.stdout).map_err(|e| e.to_string())?;
        let power = parse_power(text);
        if power == Power::Unknown { return Err("power probe returned an unknown source".into()); }
        Ok(power)
    })();
    match result {
        Ok(power) => PowerSample { power, observed_at: now(), detail: None },
        Err(detail) => PowerSample { power: Power::Unknown, observed_at: now(), detail: Some(detail) },
    }
}

#[cfg(not(target_os = "macos"))]
fn probe_power() -> PowerSample {
    PowerSample { power: Power::NoBattery, observed_at: now(), detail: Some("Battery automation is supported on macOS; use manual On the go on this host.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(power: Power) -> PowerSample { PowerSample { power, observed_at: 100, detail: None } }
    fn status(settings: &Settings, power: Power) -> Status { resolve("test".into(), Some(settings), Some(&sample(power)), 101) }

    #[test]
    fn battery_auto_and_manual_precedence() {
        let mut settings = Settings::default();
        assert_eq!(status(&settings, Power::Ac).effective, Selection::Normal);
        assert_eq!(status(&settings, Power::Battery).effective, Selection::OnTheGo);
        settings.selection = Selection::Normal;
        settings.manual_power = Some(Power::Battery);
        assert_eq!(status(&settings, Power::Battery).effective, Selection::Normal);
        assert_eq!(status(&settings, Power::Ac).selection, Selection::Auto);
        settings.selection = Selection::OnTheGo;
        settings.manual_power = Some(Power::Ac);
        assert_eq!(status(&settings, Power::Ac).effective, Selection::OnTheGo);
        assert_eq!(status(&settings, Power::Battery).selection, Selection::Auto);
    }

    #[test]
    fn stale_missing_future_and_unknown_samples_restrict_auto() {
        let settings = Settings::default();
        for s in [None, Some(PowerSample { observed_at: 1, ..sample(Power::Ac) }), Some(sample(Power::Unknown)), Some(PowerSample { observed_at: 1000, ..sample(Power::Ac) })] {
            assert_eq!(resolve("test".into(), Some(&settings), s.as_ref(), 200).effective, Selection::OnTheGo);
        }
        assert_eq!(resolve("test".into(), None, None, 200).effective, Selection::Normal);
    }

    #[test]
    fn afm_identity_locality_and_categories_are_independent() {
        let mut settings = Settings::default();
        let s = status(&settings, Power::Battery);
        assert!(s.admit_model("foundation-models", "apple-foundationmodel", true).is_ok());
        assert!(s.admit_model("foundation-models", "apple-foundationmodel", false).is_err());
        assert!(s.admit_model("other", "apple-foundationmodel", true).is_err());
        assert!(s.admit_model("foundation-models", "other", true).is_err());
        settings.allow.insert(Category::BulkIndexing);
        let s = status(&settings, Power::Battery);
        assert!(s.require(Category::BulkIndexing).is_ok());
        assert!(s.admit_model("ollama", "small", true).is_err());
        settings.allow.insert(Category::OtherLocalModels);
        let s = status(&settings, Power::Battery);
        assert!(s.admit_model("ollama", "small", true).is_ok());
        assert!(s.admit_model("provider", "big", false).is_err());
    }

    #[test]
    fn pmset_parser_does_not_guess() {
        assert_eq!(parse_power("Now drawing from 'AC Power'\n -InternalBattery-0  90%; charging"), Power::Ac);
        assert_eq!(parse_power("Now drawing from 'Battery Power'\n -InternalBattery-0 80%; discharging"), Power::Battery);
        assert_eq!(parse_power("Now drawing from 'AC Power'\n"), Power::NoBattery);
        for bad in ["", "error", "Battery Power", "Now drawing from 'AC Power'\ninvalid"] {
            assert_eq!(parse_power(bad), Power::Unknown);
        }
    }

    #[test]
    fn updates_reject_unknown_fields_and_categories() {
        assert!(serde_json::from_str::<Update>(r#"{"allow":["unknown"]}"#).is_err());
        assert!(serde_json::from_str::<Update>(r#"{"selection":"normal","bypass":true}"#).is_err());
    }

    #[test]
    fn files_preserve_exceptions_reset_manual_and_isolate_devices() {
        let root = std::env::temp_dir().join(format!("sjel-runtime-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let a = Files::for_device(&root, "a").unwrap();
        let b = Files::for_device(&root, "b").unwrap();
        assert!(!a.status().unwrap().configured);
        let settings = Settings { selection: Selection::Normal, manual_power: Some(Power::Battery), allow: BTreeSet::from([Category::Transcription]), revision: 1 };
        write_atomic(&a.settings, &settings).unwrap();
        a.refresh_with(&sample(Power::Ac)).unwrap();
        a.refresh_with(&sample(Power::Battery)).unwrap();
        let saved: Settings = read_optional(&a.settings).unwrap().unwrap();
        assert_eq!(saved.selection, Selection::Auto);
        assert!(saved.allow.contains(&Category::Transcription));
        assert_eq!(saved.revision, 2);
        let stale = Update { allow: Some(BTreeSet::new()), expected_revision: Some(1), ..Default::default() };
        assert!(a.update(stale).unwrap_err().starts_with("runtime preferences changed"));
        let saved: Settings = read_optional(&a.settings).unwrap().unwrap();
        assert!(saved.allow.contains(&Category::Transcription));
        assert!(!b.status().unwrap().configured);
        fs::write(&a.settings, "broken").unwrap();
        assert!(a.status().is_err());
        assert!(Files::for_device(&root, "../escape").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
