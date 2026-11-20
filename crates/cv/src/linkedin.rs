//! LinkedIn, read-only: pull the Member Snapshot API, compare it with the master, and print a
//! paste sheet. The Member Data Portability API has no write path.
//!
//! API: learn.microsoft.com/linkedin/dma/member-data-portability/shared/member-snapshot-api

use crate::model::{Lang, Master};
use crate::{Paths, Res};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Keychain item that holds the token (scope `r_dma_portability_self_serve`).
pub const KEYCHAIN_SERVICE: &str = "sjel-linkedin-dma";

type Domains = BTreeMap<String, Vec<Value>>;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    Api,
    Export,
}

#[derive(Deserialize, Serialize)]
struct Capture {
    source: Source,
    domains: Domains,
}

/// Files of a "Get a copy of your data" export, as `<file> -> <domain>`. The export needs no
/// developer app or token, so it is the fallback when the snapshot API is unavailable.
const EXPORT_FILES: [(&str, &str); 6] = [
    ("Profile.csv", "PROFILE"),
    ("Positions.csv", "POSITIONS"),
    ("Education.csv", "EDUCATION"),
    ("Skills.csv", "SKILLS"),
    ("Languages.csv", "LANGUAGES"),
    ("Organizations.csv", "ORGANIZATIONS"),
];

/// CV-relevant snapshot domains, plus `JOB_APPLICATIONS` for the tracker.
pub const DOMAINS: [&str; 10] = [
    "PROFILE",
    "POSITIONS",
    "EDUCATION",
    "SKILLS",
    "LANGUAGES",
    "CERTIFICATIONS",
    "PROJECTS",
    "HONORS",
    "VOLUNTEERING_EXPERIENCES",
    "JOB_APPLICATIONS",
];

const MAX_PAGES: u32 = 100;

fn token() -> Res<String> {
    let user = std::env::var("USER").unwrap_or_default();
    let out = if cfg!(target_os = "macos") {
        Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                &user,
                "-w",
            ])
            .output()
    } else {
        Command::new("secret-tool")
            .args(["lookup", "service", KEYCHAIN_SERVICE])
            .output()
    };
    match out {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => Err(format!(
            "no LinkedIn token in the keychain (service '{KEYCHAIN_SERVICE}'); store it with: \
             security add-generic-password -U -s {KEYCHAIN_SERVICE} -a \"$USER\" -w"
        )),
    }
}

/// One GET; the token reaches curl on stdin so it never appears in the process list.
fn get(token: &str, domain: &str, start: u32) -> Res<(u16, String)> {
    let url = format!(
        "https://api.linkedin.com/rest/memberSnapshotData?q=criteria&domain={domain}&start={start}"
    );
    let mut child = Command::new("curl")
        .args([
            "-sS",
            "-H",
            "@-",
            "-H",
            "Linkedin-Version: 202312",
            "-w",
            "\n%{http_code}",
            &url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("curl stdin")?
        .write_all(format!("Authorization: Bearer {token}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("{domain} page {start}: curl {}", out.status));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok((code.trim().parse().unwrap_or(0), body.to_string()))
}

/// Pull the requested domains (all of [`DOMAINS`] when empty) into one local capture.
///
/// # Errors
/// Authentication, a failed page, or a capture with no available domains.
pub fn pull(paths: &Paths, domains: &[String]) -> Res<Vec<String>> {
    let token = token()?;
    pull_capture(paths, domains, |domain, start| get(&token, domain, start))
}

fn pull_capture(
    paths: &Paths,
    domains: &[String],
    mut page_at: impl FnMut(&str, u32) -> Res<(u16, String)>,
) -> Res<Vec<String>> {
    let requested: Vec<&str> = if domains.is_empty() {
        DOMAINS.to_vec()
    } else {
        domains.iter().map(String::as_str).collect()
    };
    let mut capture = Domains::new();
    let mut report = Vec::new();
    for domain in requested {
        if domain.is_empty() || !domain.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            return Err(format!("not a snapshot domain: {domain}"));
        }
        match pull_domain(domain, |start| page_at(domain, start))? {
            Some(rows) => {
                report.push(format!("{domain}: {} record(s)", rows.len()));
                capture.insert(domain.to_owned(), rows);
            }
            None => report.push(format!("{domain}: unavailable")),
        }
    }
    if capture.is_empty() {
        return Err("no LinkedIn domains available".into());
    }
    save_capture(paths, Source::Api, capture)?;
    Ok(report)
}

fn pull_domain(
    domain: &str,
    mut page_at: impl FnMut(u32) -> Res<(u16, String)>,
) -> Res<Option<Vec<Value>>> {
    let mut rows = Vec::new();
    for start in 0..MAX_PAGES {
        let (code, body) = page_at(start)?;
        if code != 200 {
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if matches!(code, 400 | 404)
                && detail
                    .to_ascii_lowercase()
                    .contains("no data found for this memberid")
            {
                return Ok((start > 0).then_some(rows));
            }
            return Err(format!("{domain} page {start}: HTTP {code} {detail}"));
        }
        let page: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let elements = page["elements"]
            .as_array()
            .ok_or_else(|| format!("{domain} page {start}: missing elements"))?;
        if elements.is_empty() {
            return Ok((start > 0).then_some(rows));
        }
        let before = rows.len();
        for entry in elements {
            if entry["snapshotDomain"].as_str() != Some(domain) {
                return Err(format!("{domain} page {start}: unexpected snapshot domain"));
            }
            let records = entry["snapshotData"]
                .as_array()
                .ok_or_else(|| format!("{domain} page {start}: missing snapshot data"))?;
            if !records.iter().all(Value::is_object) {
                return Err(format!("{domain} page {start}: invalid snapshot record"));
            }
            rows.extend(records.iter().cloned());
        }
        if rows.len() == before {
            return Ok(Some(rows));
        }
    }
    Err(format!("{domain}: snapshot page limit reached"))
}

fn save_capture(paths: &Paths, source: Source, domains: Domains) -> Res<()> {
    fn write(dir: &Path, capture: &Capture) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        let time = chrono::Utc::now().format("%Y-%m-%dT%H%M%S%.9fZ");
        let file = dir.join(format!("{time}-{}.json", std::process::id()));
        let staged = file.with_extension("part");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut output = options.open(&staged)?;
        let result = (|| {
            serde_json::to_writer(&mut output, capture)?;
            output.sync_all()?;
            fs::hard_link(&staged, file)
        })();
        drop(output);
        let _ = fs::remove_file(staged);
        result
    }
    let dir = paths.linkedin().join("captures");
    write(&dir, &Capture { source, domains }).map_err(|e| format!("{}: {e}", dir.display()))
}

fn latest_capture_path(paths: &Paths) -> Res<Option<PathBuf>> {
    let dir = paths.linkedin().join("captures");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", dir.display())),
    };
    let mut latest = None;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
            && latest.as_ref().is_none_or(|current| current < &path)
        {
            latest = Some(path);
        }
    }
    Ok(latest)
}

pub(crate) fn last_capture_date(paths: &Paths) -> Res<Option<String>> {
    Ok(latest_capture_path(paths)?
        .and_then(|path| path.file_stem()?.to_str()?.get(..10).map(str::to_owned)))
}

/// Import a LinkedIn CSV export as one local capture. Returns one line per imported domain.
///
/// # Errors
/// The source or a recognized CSV cannot be read, or no recognized CSV is present.
pub fn import(paths: &Paths, dir: &Path) -> Res<Vec<String>> {
    fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut capture = Domains::new();
    let mut report = Vec::new();
    for (file, domain) in EXPORT_FILES {
        let path = dir.join(file);
        let Some(rows) = export_csv(&path)? else {
            continue;
        };
        report.push(format!("{domain}: {} record(s) <- {file}", rows.len()));
        capture.insert(domain.to_owned(), rows);
    }
    if capture.is_empty() {
        return Err(format!("no LinkedIn export CSV found in {}", dir.display()));
    }
    let profile = capture.get_mut("PROFILE").and_then(|rows| rows.first_mut());
    if let Some((about, profile)) = export_summary(dir)?
        .zip(profile)
        .filter(|(_, profile)| profile["Summary"].as_str().unwrap_or("").trim().is_empty())
    {
        profile["Summary"] = Value::String(about);
    }
    save_capture(paths, Source::Export, capture)?;
    Ok(report)
}

/// The export keeps About in its own single-column file, which LinkedIn may leave empty.
fn export_summary(dir: &Path) -> Res<Option<String>> {
    let rows = export_csv(&dir.join("Profile Summary.csv"))?;
    Ok(rows
        .as_deref()
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("Profile Summary").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|about| !about.is_empty()))
}

fn export_csv(path: &Path) -> Res<Option<Vec<Value>>> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => read_csv(path).map(Some),
        Ok(_) => Err(format!("{}: not a CSV file", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn read_csv(path: &Path) -> Res<Vec<Value>> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let rows = split_rows(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some((header, body)) = rows.split_first() else {
        return Err(format!("{}: missing CSV header", path.display()));
    };
    body.iter()
        .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        .map(|row| {
            if row.len() != header.len() {
                return Err(format!(
                    "{}: CSV row has a different length than its header",
                    path.display()
                ));
            }
            let fields = header
                .iter()
                .zip(row)
                .map(|(key, cell)| (key.clone(), Value::String(cell.trim().to_owned())));
            Ok(Value::Object(fields.collect()))
        })
        .collect()
}

/// Split a CSV text into rows of cells. RFC 4180: fields are quoted, `""` is one quote, and a
/// position's description carries literal newlines inside its quotes.
fn split_rows(text: &str) -> Res<Vec<Vec<String>>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            // A bare `\r` is the CR of a CRLF line ending.
            '\r' => {}
            _ => field.push(c),
        }
    }
    if quoted {
        return Err("unterminated CSV quote".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn read_capture(path: &Path) -> Res<Capture> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn field<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("?")
}

/// Skill names lowercased with any "(...)" qualifier removed.
fn normalize(name: &str) -> String {
    let base = name.split(" (").next().unwrap_or(name);
    base.trim().to_lowercase()
}

/// Headline, About, positions and skills: LinkedIn next to the master.
///
/// # Errors
/// No coherent capture exists, or the latest capture cannot be read.
pub fn diff(paths: &Paths, master: &Master) -> Res<String> {
    let path = latest_capture_path(paths)?
        .ok_or("no LinkedIn capture; run 'cv linkedin import <dir>' or 'cv linkedin pull'")?;
    let capture = read_capture(&path)?;
    let id = path
        .file_stem()
        .ok_or("missing capture ID")?
        .to_string_lossy();
    let source = match capture.source {
        Source::Api => "api",
        Source::Export => "export",
    };
    let profile = capture.domains.get("PROFILE").and_then(|rows| rows.first());
    let mut out = format!("LinkedIn snapshot {id} ({source})\n\n");
    let _ = writeln!(out, "== Headline");
    let _ = writeln!(
        out,
        "  linkedin: {}",
        profile
            .and_then(|row| row["Headline"].as_str())
            .unwrap_or("unavailable in this snapshot")
    );
    let _ = writeln!(out, "  master:   {}\n", master.headline(Lang::En));
    let about = profile
        .and_then(|row| row["Summary"].as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let _ = writeln!(out, "== About");
    if let Some(about) = about {
        let same = about == master.about(Lang::En).trim();
        let _ = writeln!(out, "  differs: {}\n", if same { "no" } else { "yes" });
    } else {
        let _ = writeln!(out, "  linkedin: unavailable in this snapshot\n");
    }
    let _ = writeln!(out, "== Positions on LinkedIn");
    if let Some(positions) = capture.domains.get("POSITIONS") {
        if positions.is_empty() {
            let _ = writeln!(out, "  no records in this snapshot");
        }
        for p in positions {
            let end = p
                .get("Finished On")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("now");
            let _ = writeln!(
                out,
                "  {} - {end}  {} @ {}",
                field(p, "Started On"),
                field(p, "Title"),
                field(p, "Company Name")
            );
        }
    } else {
        let _ = writeln!(out, "  unavailable in this snapshot");
    }
    let _ = writeln!(out, "== Positions in master");
    for e in &master.experience {
        let role = e.role.as_ref().map_or("?", |r| r.get(Lang::En));
        let _ = writeln!(out, "  {}  {role} @ {}", e.date, e.company);
    }
    let Some(skills) = capture.domains.get("SKILLS") else {
        let _ = writeln!(out, "\n== Skills\n  unavailable in this snapshot");
        return Ok(out);
    };
    let theirs: BTreeSet<String> = skills
        .iter()
        .filter_map(|s| s["Name"].as_str())
        .map(normalize)
        .collect();
    // Both languages: a row may name a skill once per language, and LinkedIn holds both spellings.
    let mut ours: BTreeSet<String> = BTreeSet::new();
    for skill in &master.skills {
        for lang in [Lang::En, Lang::De] {
            ours.extend(skill.items.get(lang).iter().map(|item| normalize(item)));
        }
    }
    let _ = writeln!(out, "\n== Skills only on LinkedIn");
    theirs.difference(&ours).for_each(|s| {
        let _ = writeln!(out, "  {s}");
    });
    let _ = writeln!(out, "== Skills only in master");
    ours.difference(&theirs).for_each(|s| {
        let _ = writeln!(out, "  {s}");
    });
    Ok(out)
}

/// Headline, About and every position with its bullets, in the order LinkedIn shows them.
pub fn sheet(master: &Master, lang: Lang) -> String {
    let mut out = format!(
        "HEADLINE\n{}\n\nABOUT\n{}\n\nEXPERIENCE\n",
        master.headline(lang),
        master.about(lang)
    );
    let roles: Vec<String> = master
        .experience
        .iter()
        .map(|e| {
            let role = e.role.as_ref().map_or("", |r| r.get(lang));
            let mut block = format!("{role} @ {} | {} | {}", e.company, e.location, e.date);
            for b in &e.bullets {
                block.push_str(&format!("\n- {}", b.text.get(lang)));
            }
            block
        })
        .collect();
    out.push_str(&roles.join("\n\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_survives_quotes_commas_and_newlines() {
        let rows =
            split_rows("a,b,c\n1,\"x, y\",\"line1\nline2\"\n2,\"he said \"\"hi\"\"\",z\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ["1", "x, y", "line1\nline2"]);
        assert_eq!(rows[2][1], "he said \"hi\"");
    }

    #[test]
    fn export_rows_become_snapshot_objects_and_skip_empty_rows() {
        let dir = std::env::temp_dir().join("sjel-cv-linkedin-import-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Skills.csv"), "Name\nPython\n\n").unwrap();
        fs::write(dir.join("Positions.csv"), "Company Name,Title\nAcme,Dev\n").unwrap();
        let paths = Paths {
            overlay: dir.clone(),
            data: dir.clone(),
            template: dir.join("cv.typ"),
        };
        let report = import(&paths, &dir).unwrap();
        assert_eq!(report.len(), 2);
        let capture = read_capture(&latest_capture_path(&paths).unwrap().unwrap()).unwrap();
        let skills = &capture.domains["SKILLS"];
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["Name"], "Python");
        assert!(matches!(capture.source, Source::Export));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_takes_a_multiline_about_from_its_own_file() {
        let dir = std::env::temp_dir().join("sjel-cv-linkedin-import-about-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Profile.csv"), "Headline,Summary\nEngineer,\n").unwrap();
        fs::write(
            dir.join("Profile Summary.csv"),
            "Profile Summary\n\"First line\nSecond line\"\n",
        )
        .unwrap();
        let paths = Paths {
            overlay: dir.clone(),
            data: dir.clone(),
            template: dir.join("cv.typ"),
        };
        import(&paths, &dir).unwrap();
        let capture = read_capture(&latest_capture_path(&paths).unwrap().unwrap()).unwrap();
        assert_eq!(
            capture.domains["PROFILE"][0]["Summary"],
            "First line\nSecond line"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_refuses_a_directory_with_no_export() {
        let dir = std::env::temp_dir().join("sjel-cv-linkedin-import-empty-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            overlay: dir.clone(),
            data: dir.clone(),
            template: dir.join("cv.typ"),
        };
        assert!(import(&paths, &dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_export_does_not_reuse_previous_domains() {
        let dir =
            std::env::temp_dir().join(format!("sjel-cv-linkedin-partial-{}", std::process::id()));
        let first = dir.join("first");
        let partial = dir.join("partial");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&partial).unwrap();
        fs::write(first.join("Profile.csv"), "Headline\nFirst headline\n").unwrap();
        fs::write(
            first.join("Positions.csv"),
            "Company Name,Title\nSynthetic prior employer,Synthetic role\n",
        )
        .unwrap();
        fs::write(first.join("Skills.csv"), "Name\nSynthetic skill\n").unwrap();
        fs::write(partial.join("Profile.csv"), "Headline\nNew headline\n").unwrap();
        let paths = Paths {
            overlay: dir.clone(),
            data: dir.clone(),
            template: dir.join("cv.typ"),
        };
        let master = Master::from_yaml(include_str!("../master_cv.schema.yaml")).unwrap();
        let captures = paths.linkedin().join("captures");
        fs::create_dir_all(&captures).unwrap();
        fs::write(paths.linkedin().join("PULLED_ON"), "2000-01-01\n").unwrap();
        fs::write(captures.join("9999-01-01.part"), b"{").unwrap();
        assert_eq!(last_capture_date(&paths).unwrap(), None);
        assert!(diff(&paths, &master).is_err());

        import(&paths, &first).unwrap();
        let before = diff(&paths, &master).unwrap();
        let original = latest_capture_path(&paths).unwrap().unwrap();
        assert!(before.contains("Synthetic prior employer"));
        assert!(before.contains("synthetic skill"));

        fs::write(partial.join("Skills.csv"), "Name\n\"truncated").unwrap();
        assert!(import(&paths, &partial)
            .unwrap_err()
            .contains("unterminated"));
        assert_eq!(diff(&paths, &master).unwrap(), before);
        fs::remove_file(partial.join("Skills.csv")).unwrap();

        let requested = ["PROFILE".into(), "SKILLS".into()];
        let error = pull_capture(&paths, &requested, |domain, start| {
            Ok(match (domain, start) {
                ("PROFILE", 0) => (200, r#"{"elements":[{"snapshotDomain":"PROFILE","snapshotData":[{"Headline":"New API headline"}]}]}"#.into()),
                ("PROFILE", 1) => (200, r#"{"elements":[]}"#.into()),
                ("SKILLS", 0) => (503, "{}".into()),
                _ => panic!("unexpected page: {domain} {start}"),
            })
        }).unwrap_err();
        assert!(error.contains("SKILLS page 0: HTTP 503"));
        assert_eq!(diff(&paths, &master).unwrap(), before);

        import(&paths, &partial).unwrap();
        let after = diff(&paths, &master).unwrap();
        assert!(after.contains("linkedin: New headline"));
        assert!(after.contains("== Positions on LinkedIn\n  unavailable in this snapshot"));
        assert!(after.contains("== Skills\n  unavailable in this snapshot"));
        assert!(!after.contains("Synthetic prior employer"));
        assert!(!after.contains("synthetic skill"));
        assert!(!after.contains("Skills only in master"));
        fs::write(partial.join("Skills.csv"), "Name\n").unwrap();
        import(&paths, &partial).unwrap();
        assert!(diff(&paths, &master)
            .unwrap()
            .contains("Skills only in master"));
        assert!(read_capture(&original)
            .unwrap()
            .domains
            .contains_key("SKILLS"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn api_pagination_distinguishes_no_data_from_failure() {
        let page = r#"{"elements":[{"snapshotDomain":"SKILLS","snapshotData":[{"Name":"Synthetic skill"}]}]}"#;
        let end = r#"{"message":"No data found for this memberId"}"#;
        let rows = pull_domain("SKILLS", |start| {
            Ok(if start == 0 {
                (200, page.into())
            } else {
                (404, end.into())
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!(rows[0]["Name"], "Synthetic skill");
        assert!(pull_domain("SKILLS", |_| Ok((404, end.into())))
            .unwrap()
            .is_none());
        assert!(
            pull_domain("SKILLS", |_| Ok((200, r#"{"elements":[]}"#.into())))
                .unwrap()
                .is_none()
        );
        let empty = r#"{"elements":[{"snapshotDomain":"SKILLS","snapshotData":[]}]}"#;
        assert_eq!(
            pull_domain("SKILLS", |_| Ok((200, empty.into()))).unwrap(),
            Some(vec![])
        );

        let failure = pull_domain("SKILLS", |start| {
            Ok(if start == 0 {
                (200, page.into())
            } else {
                (500, "{}".into())
            })
        })
        .unwrap_err();
        assert!(failure.contains("page 1: HTTP 500"));
    }

    #[test]
    fn skill_names_compare_without_qualifiers_or_case() {
        assert_eq!(normalize("Python (FastAPI, Quart)"), "python");
        assert_eq!(normalize("Kubernetes"), "kubernetes");
    }

    #[test]
    fn sheet_localizes_roles() {
        let mut m = Master::from_yaml(include_str!("../master_cv.schema.yaml")).unwrap();
        m.experience[0].role = Some(yaml_serde::from_str("{en: Role EN, de: Rolle DE}").unwrap());
        assert!(sheet(&m, Lang::De).contains("Rolle DE @ "));
        assert!(sheet(&m, Lang::En).starts_with("HEADLINE\n"));
    }
}
