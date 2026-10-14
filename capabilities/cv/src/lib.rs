//! The `cv` capability: one master file, filtered by profile and priority, rendered by Typst.
//!
//! Rendering stays in `templates/cv.typ`, which reads the YAML itself. This crate owns
//! everything around it: the typed model, the page-limit loop, application files, edits that
//! must parse and render before they land, and the LinkedIn snapshot.

pub mod linkedin;
pub mod model;

use model::{Application, Lang, LogEntry, Master, Status};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result type of every operation; the error is the line printed after `cv: `.
pub type Res<T> = Result<T, String>;

/// Where the master, applications, LinkedIn snapshot, PDFs and template live.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Overlay root, which is a git repository when edits are to be committed.
    pub overlay: PathBuf,
    /// `<overlay>/data/cv`.
    pub data: PathBuf,
    /// The Typst template.
    pub template: PathBuf,
}

impl Paths {
    /// Resolve from `SJEL_PERSONAL_ROOT` and `SJEL_ROOT`, both exported by `tools/lib/paths.sh`.
    ///
    /// # Errors
    /// Either variable is unset.
    pub fn from_env() -> Res<Paths> {
        let overlay = sjel_config::overlay_root().ok_or("SJEL_PERSONAL_ROOT is not set")?;
        let root = std::env::var("SJEL_ROOT").map_err(|_| "SJEL_ROOT is not set")?;
        Ok(Paths {
            data: overlay.join("data/cv"),
            overlay,
            template: PathBuf::from(root).join("capabilities/cv/templates/cv.typ"),
        })
    }

    /// The master file.
    pub fn master(&self) -> PathBuf {
        self.data.join("master_cv.yaml")
    }
    /// Directory of application files.
    pub fn apps(&self) -> PathBuf {
        self.data.join("applications")
    }
    /// Rendered PDFs.
    pub fn dist(&self) -> PathBuf {
        self.data.join("dist")
    }
    /// Pulled LinkedIn snapshot.
    pub fn linkedin(&self) -> PathBuf {
        self.data.join("linkedin")
    }
    /// One application file.
    ///
    /// # Errors
    /// `slug` is not lowercase letters, digits and dashes.
    pub fn app(&self, slug: &str) -> Res<PathBuf> {
        valid_slug(slug)?;
        Ok(self.apps().join(format!("{slug}.yaml")))
    }
}

/// Refuse anything but lowercase letters, digits and dashes, not starting with a dash.
///
/// # Errors
/// The slug breaks that rule.
pub fn valid_slug(slug: &str) -> Res<()> {
    let ok = !slug.is_empty()
        && !slug.starts_with('-')
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!("slug must be lowercase letters, digits and dashes: '{slug}'"))
    }
}

fn read(path: &Path) -> Res<String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Load and validate the master.
///
/// # Errors
/// The file is missing, or [`Master::from_yaml`] refuses it.
pub fn load_master(paths: &Paths) -> Res<Master> {
    let path = paths.master();
    if !path.exists() {
        return Err(format!(
            "no master file at {}; copy capabilities/cv/master_cv.schema.yaml there and fill it in",
            path.display()
        ));
    }
    Master::from_yaml(&read(&path)?)
}

/// Load one application.
///
/// # Errors
/// Bad slug, missing file, or a file that does not parse.
pub fn load_app(paths: &Paths, slug: &str) -> Res<Application> {
    let path = paths.app(slug)?;
    if !path.exists() {
        return Err(format!("no application '{slug}' at {}", path.display()));
    }
    Application::from_yaml(&read(&path)?)
}

/// Every application with its slug, sorted by slug.
///
/// # Errors
/// The directory cannot be read, or one file does not parse.
pub fn list_apps(paths: &Paths) -> Res<Vec<(String, Application)>> {
    let Ok(dir) = fs::read_dir(paths.apps()) else { return Ok(Vec::new()) };
    let mut slugs: Vec<String> = dir
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".yaml").map(str::to_string))
        .collect();
    slugs.sort();
    slugs.into_iter().map(|s| Ok((s.clone(), load_app(paths, &s)?))).collect()
}

fn known_profile(master: &Master, profile: &str) -> Res<()> {
    let known = master.profiles();
    if known.iter().any(|p| p == profile) {
        Ok(())
    } else {
        Err(format!("unknown profile '{profile}'; known profiles: {}", known.join(" ")))
    }
}

/// One render request.
#[derive(Debug, Clone)]
pub struct Render<'a> {
    /// Master file to read; a temporary file during an edit check.
    pub master: &'a Path,
    /// Profile tag.
    pub profile: &'a str,
    /// Language.
    pub lang: Lang,
    /// Application file whose overrides apply.
    pub app: Option<&'a Path>,
}

impl Render<'_> {
    fn inputs(&self, keep: u8) -> Vec<String> {
        let mut args = vec![
            "--root".into(),
            "/".into(),
            "--input".into(),
            format!("master={}", self.master.display()),
            "--input".into(),
            format!("profile={}", self.profile),
            "--input".into(),
            format!("lang={}", self.lang.code()),
            "--input".into(),
            format!("keep={keep}"),
        ];
        if let Some(app) = self.app {
            args.extend(["--input".into(), format!("app={}", app.display())]);
        }
        args
    }
}

fn typst(args: &[String]) -> Res<String> {
    let out = Command::new("typst")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run typst ({e}); install it: brew install typst"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Page count of a render at priority level `keep`, from the template's `<pages>` metadata.
///
/// # Errors
/// Typst fails or prints something that is not a number.
pub fn pages(paths: &Paths, render: &Render, keep: u8) -> Res<u32> {
    let mut args = vec![
        "eval".to_string(),
        "query(<pages>).first().value".into(),
        "--in".into(),
        paths.template.display().to_string(),
    ];
    args.extend(render.inputs(keep));
    let out = typst(&args)?;
    out.parse().map_err(|_| format!("typst reported '{out}' as the page count"))
}

/// What a build produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The PDF.
    pub path: PathBuf,
    /// Its page count.
    pub pages: u32,
    /// Highest priority level kept.
    pub keep: u8,
}

/// Render at the highest priority level that fits `limit` pages (0 = no limit).
///
/// # Errors
/// Typst fails, or priority-1 items alone exceed the limit.
pub fn build(paths: &Paths, render: &Render, limit: u32, out: &Path) -> Res<Built> {
    let mut keep = 3;
    let pages = loop {
        let n = pages(paths, render, keep)?;
        if limit == 0 || n <= limit {
            break n;
        }
        if keep == 1 {
            return Err(format!(
                "{}/{} is {n} page(s) with priority-1 items only; limit is {limit}. \
                 Give more items priority 2 or 3, or hide items in the application file.",
                render.profile,
                render.lang.code()
            ));
        }
        keep -= 1;
    };
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut args = vec!["compile".to_string()];
    args.extend(render.inputs(keep));
    args.extend([paths.template.display().to_string(), out.display().to_string()]);
    typst(&args)?;
    Ok(Built { path: out.to_path_buf(), pages, keep })
}

/// Build one profile in one language into `dist/`, or into `out`.
///
/// # Errors
/// Unknown profile, or [`build`] fails.
pub fn build_profile(paths: &Paths, profile: &str, lang: Lang, out: Option<PathBuf>) -> Res<Built> {
    let master = load_master(paths)?;
    known_profile(&master, profile)?;
    let out = out.unwrap_or_else(|| paths.dist().join(format!("CV_{profile}_{}.pdf", lang.code())));
    let render = Render { master: &paths.master(), profile, lang, app: None };
    build(paths, &render, master.page_limit(profile), &out)
}

/// Build every profile in each of its languages, narrowed to `only` when given. Failures do
/// not stop the other builds.
pub fn build_all(paths: &Paths, only: Option<Lang>) -> Res<Vec<Res<Built>>> {
    let master = load_master(paths)?;
    let mut results = Vec::new();
    for profile in master.profiles() {
        for lang in master.langs(&profile) {
            if only.is_some_and(|o| o != lang) {
                continue;
            }
            results.push(build_profile(paths, &profile, lang, None));
        }
    }
    Ok(results)
}

/// Build the CV for one application into `dist/apps/`.
///
/// # Errors
/// The application does not load, names an unknown profile, or [`build`] fails.
pub fn build_app(paths: &Paths, slug: &str) -> Res<Built> {
    let app = load_app(paths, slug)?;
    let master = load_master(paths)?;
    known_profile(&master, &app.profile)?;
    let out = paths.dist().join(format!("apps/CV_{slug}_{}.pdf", app.lang.code()));
    let app_path = paths.app(slug)?;
    let render = Render { master: &paths.master(), profile: &app.profile, lang: app.lang, app: Some(&app_path) };
    build(paths, &render, master.page_limit(&app.profile), &out)
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Fields of a new application.
#[derive(Debug, Clone)]
pub struct NewApp {
    /// File name without `.yaml`.
    pub slug: String,
    /// Company.
    pub company: String,
    /// Role.
    pub role: String,
    /// Posting URL.
    pub url: String,
    /// Profile to build with.
    pub profile: String,
    /// Build language.
    pub lang: Lang,
}

/// Create an application file with status `draft`.
///
/// # Errors
/// Bad slug, empty company or role, unknown profile, or the file already exists.
pub fn new_app(paths: &Paths, new: NewApp) -> Res<PathBuf> {
    let path = paths.app(&new.slug)?;
    if new.company.is_empty() || new.role.is_empty() {
        return Err("app new needs --company and --role".into());
    }
    known_profile(&load_master(paths)?, &new.profile)?;
    if path.exists() {
        return Err(format!("application '{}' already exists at {}", new.slug, path.display()));
    }
    let app = Application {
        company: new.company,
        role: new.role,
        url: new.url,
        profile: new.profile,
        lang: new.lang,
        status: Status::Draft,
        log: vec![LogEntry { date: today(), status: Status::Draft, note: None }],
        title: None,
        summary: None,
        pin: Vec::new(),
        hide: Vec::new(),
        notes: None,
    };
    write_checked(paths, &path, &app.to_yaml()?, Target::App(&new.slug))?;
    Ok(path)
}

/// Set an application's status and append it to the log.
///
/// # Errors
/// The application does not load or cannot be written.
pub fn set_status(paths: &Paths, slug: &str, status: Status, note: Option<String>) -> Res<()> {
    let mut app = load_app(paths, slug)?;
    app.status = status;
    app.log.push(LogEntry { date: today(), status, note: note.filter(|n| !n.is_empty()) });
    write_checked(paths, &paths.app(slug)?, &app.to_yaml()?, Target::App(slug)).map(|_| ())
}

/// Which file an edit replaces.
#[derive(Debug, Clone, Copy)]
pub enum Target<'a> {
    /// The master.
    Master,
    /// One application, by slug.
    App(&'a str),
}

impl Target<'_> {
    /// The file this target names.
    ///
    /// # Errors
    /// Bad slug.
    pub fn path(&self, paths: &Paths) -> Res<PathBuf> {
        match self {
            Target::Master => Ok(paths.master()),
            Target::App(slug) => paths.app(slug),
        }
    }
}

/// Outcome of a checked write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Content equals the file on disk.
    Unchanged,
    /// Written, and committed when the overlay is a git repository.
    Written {
        /// Short commit hash.
        commit: Option<String>,
    },
}

/// Replace a master or application file after it parses and renders. The write is an atomic
/// rename that keeps the old file mode, and it is committed in the overlay's git.
///
/// # Errors
/// The text does not parse, does not render, or cannot be written.
pub fn write_checked(paths: &Paths, path: &Path, text: &str, target: Target) -> Res<Saved> {
    match target {
        Target::Master => {
            Master::from_yaml(text)?;
        }
        Target::App(_) => {
            Application::from_yaml(text)?;
        }
    }
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(Saved::Unchanged);
    }
    let dir = path.parent().ok_or("file has no directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".edit.{}.yaml", std::process::id()));
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    let checked = render_check(paths, &tmp, target);
    if let Err(e) = checked {
        let _ = fs::remove_file(&tmp);
        return Err(format!("does not render: {e}"));
    }
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(&tmp, meta.permissions()).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(Saved::Written { commit: commit(paths, path)? })
}

fn render_check(paths: &Paths, candidate: &Path, target: Target) -> Res<()> {
    let out = std::env::temp_dir().join(format!("cv-check-{}.pdf", std::process::id()));
    let master_path = paths.master();
    let profile;
    let render = match target {
        Target::Master => Render { master: candidate, profile: "default", lang: Lang::En, app: None },
        Target::App(_) => {
            profile = Application::from_yaml(&read(candidate)?)?.profile;
            Render { master: &master_path, profile: &profile, lang: Lang::En, app: Some(candidate) }
        }
    };
    let mut args = vec!["compile".to_string()];
    args.extend(render.inputs(3));
    args.extend([paths.template.display().to_string(), out.display().to_string()]);
    let result = typst(&args).map(|_| ());
    let _ = fs::remove_file(&out);
    result
}

fn commit(paths: &Paths, path: &Path) -> Res<Option<String>> {
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&paths.overlay).args(args).output();
    let is_repo = git(&["rev-parse", "--git-dir"]).is_ok_and(|o| o.status.success());
    if !is_repo {
        return Ok(None);
    }
    let rel = path.strip_prefix(&paths.overlay).map_err(|_| "file is outside the overlay")?;
    let rel = rel.to_string_lossy().to_string();
    let message = format!("cv: edit {rel}");
    // The pathspec limits the commit to this file, so other uncommitted overlay work stays out.
    for args in [vec!["add", "--", &rel], vec!["commit", "-q", "-m", &message, "--", &rel]] {
        let out = git(&args).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("git {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    let out = git(&["rev-parse", "--short", "HEAD"]).map_err(|e| e.to_string())?;
    Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
}

fn mtime(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Profiles, applications, built PDFs with staleness, and the LinkedIn pull date.
///
/// # Errors
/// The master or an application does not load.
pub fn status(paths: &Paths) -> Res<Value> {
    let master = load_master(paths)?;
    let master_mtime = mtime(&paths.master());
    let mut pdfs = Vec::new();
    for (dir, prefix) in [(paths.dist(), ""), (paths.dist().join("apps"), "apps/")] {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".pdf"))
            .collect();
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let m = mtime(&path);
            pdfs.push(json!({
                "name": format!("{prefix}{name}"),
                "path": path,
                "mtime": m,
                "stale": m < master_mtime,
            }));
        }
    }
    let applications: Vec<Value> = list_apps(paths)?
        .into_iter()
        .map(|(slug, app)| {
            let mut v = serde_json::to_value(app).unwrap_or_default();
            v["slug"] = json!(slug);
            v
        })
        .collect();
    let pulled = fs::read_to_string(paths.linkedin().join("PULLED_ON")).ok().map(|s| s.trim().to_string());
    Ok(json!({
        "master": paths.master(),
        "master_mtime": master_mtime,
        "profiles": master.profiles(),
        "statuses": Status::ALL.iter().map(|s| s.name()).collect::<Vec<_>>(),
        "applications": applications,
        "pdfs": pdfs,
        "linkedin_pulled": pulled,
    }))
}
