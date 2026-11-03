//! LinkedIn, read-only: pull the Member Snapshot API, compare it with the master, and print a
//! paste sheet. The Member Data Portability API has no write path.
//!
//! API: learn.microsoft.com/linkedin/dma/member-data-portability/shared/member-snapshot-api

use crate::model::{Lang, Master};
use crate::{Paths, Res};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Keychain item that holds the token (scope `r_dma_portability_self_serve`).
pub const KEYCHAIN_SERVICE: &str = "sjel-linkedin-dma";

/// Snapshot capture date, `YYYY-MM-DD`.
const PULLED_ON: &str = "PULLED_ON";

/// Where the snapshot came from: `api` or `export`.
const SOURCE: &str = "SOURCE";

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
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    Ok((code.trim().parse().unwrap_or(0), body.to_string()))
}

/// Pull `domains` (all of [`DOMAINS`] when empty) into `linkedin/<DOMAIN>.json`. Returns one
/// line per domain.
///
/// # Errors
/// No token, an unknown domain name, or HTTP 401.
pub fn pull(paths: &Paths, domains: &[String]) -> Res<Vec<String>> {
    let token = token()?;
    let domains: Vec<String> = if domains.is_empty() {
        DOMAINS.iter().map(|d| d.to_string()).collect()
    } else {
        domains.to_vec()
    };
    let dir = paths.linkedin();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut report = Vec::new();
    for domain in &domains {
        if !domain.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            return Err(format!("not a snapshot domain: {domain}"));
        }
        let mut rows: Vec<Value> = Vec::new();
        let mut failed = None;
        // The API's `total` is unreliable; its docs say to page until a request fails.
        for start in 0..MAX_PAGES {
            let (code, body) = get(&token, domain, start)?;
            if code != 200 {
                if start == 0 && code != 404 {
                    let msg = serde_json::from_str::<Value>(&body)
                        .ok()
                        .and_then(|v| v["message"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    if code == 401 {
                        return Err(format!("{domain}: HTTP 401 {msg}"));
                    }
                    failed = Some(format!("{domain}: HTTP {code} {msg}"));
                }
                break;
            }
            let page: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            let found: Vec<Value> = page["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|e| e["snapshotData"].as_array().cloned().unwrap_or_default())
                .collect();
            if found.is_empty() {
                break;
            }
            rows.extend(found);
        }
        if let Some(f) = failed {
            report.push(f);
            continue;
        }
        let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
        fs::write(dir.join(format!("{domain}.json")), text).map_err(|e| e.to_string())?;
        report.push(format!("{domain}: {} record(s)", rows.len()));
    }
    stamp(&dir, "api")?;
    Ok(report)
}

/// Record the capture date and where the snapshot came from, in the snapshot directory.
fn stamp(dir: &Path, source: &str) -> Res<()> {
    let date = fs::metadata(dir)
        .and_then(|m| m.modified())
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| chrono::Local::now().format("%Y-%m-%d").to_string());
    fs::write(dir.join(PULLED_ON), format!("{date}\n")).map_err(|e| e.to_string())?;
    fs::write(dir.join(SOURCE), format!("{source}\n")).map_err(|e| e.to_string())
}

/// Import a "Get a copy of your data" CSV export from `dir` into `linkedin/<DOMAIN>.json`, in
/// the shape the snapshot API returns. Returns one line per imported domain.
///
/// # Errors
/// `dir` cannot be read, a CSV it holds cannot be read, or it holds none of [`EXPORT_FILES`].
pub fn import(paths: &Paths, dir: &Path) -> Res<Vec<String>> {
    fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let dest = paths.linkedin();
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let mut report = Vec::new();
    for (file, domain) in EXPORT_FILES {
        let path = dir.join(file);
        if !path.is_file() {
            continue;
        }
        let rows = read_csv(&path)?;
        let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
        fs::write(dest.join(format!("{domain}.json")), text).map_err(|e| e.to_string())?;
        report.push(format!("{domain}: {} record(s) <- {file}", rows.len()));
    }
    if report.is_empty() {
        return Err(format!("no LinkedIn export CSV found in {}", dir.display()));
    }
    if let Some(about) = export_summary(dir)? {
        fill_summary(&dest.join("PROFILE.json"), &about)?;
    }
    stamp(&dest, "export")?;
    Ok(report)
}

/// The export keeps About in its own single-column file, which LinkedIn may leave empty.
fn export_summary(dir: &Path) -> Res<Option<String>> {
    let path = dir.join("Profile Summary.csv");
    if !path.is_file() {
        return Ok(None);
    }
    Ok(read_csv(&path)?
        .first()
        .and_then(|row| row.get("Profile Summary").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|about| !about.is_empty()))
}

/// Put the export's separate About text into `PROFILE`'s `Summary`, which the export leaves empty.
fn fill_summary(profile: &Path, about: &str) -> Res<()> {
    let text = fs::read_to_string(profile)
        .map_err(|e| format!("cannot read {}: {e}", profile.display()))?;
    let mut rows: Vec<Value> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let Some(first) = rows.first_mut() else {
        return Ok(());
    };
    if first["Summary"].as_str().unwrap_or("").trim().is_empty() {
        first["Summary"] = Value::String(about.to_string());
    }
    let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
    fs::write(profile, text).map_err(|e| e.to_string())
}

/// Parse a CSV file into one JSON object per data row, keyed by the header.
///
/// # Errors
/// The file cannot be read.
fn read_csv(path: &Path) -> Res<Vec<Value>> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let rows = split_rows(&text);
    let Some((header, body)) = rows.split_first() else {
        return Ok(Vec::new());
    };
    Ok(body
        .iter()
        .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        .map(|row| {
            let fields = header.iter().enumerate().map(|(i, key)| {
                let cell = row.get(i).map_or("", String::as_str).trim();
                (key.clone(), Value::String(cell.to_string()))
            });
            Value::Object(fields.collect())
        })
        .collect())
}

/// Split a CSV text into rows of cells. RFC 4180: fields are quoted, `""` is one quote, and a
/// position's description carries literal newlines inside its quotes.
fn split_rows(text: &str) -> Vec<Vec<String>> {
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
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn snapshot(paths: &Paths, domain: &str) -> Res<Vec<Value>> {
    let path = paths.linkedin().join(format!("{domain}.json"));
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("{} missing; run 'cv linkedin pull' first", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
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
/// The PROFILE, POSITIONS or SKILLS snapshot is missing or unreadable.
pub fn diff(paths: &Paths, master: &Master) -> Res<String> {
    let profile = snapshot(paths, "PROFILE")?;
    let positions = snapshot(paths, "POSITIONS")?;
    let skills = snapshot(paths, "SKILLS")?;
    let first = profile.first().cloned().unwrap_or(Value::Null);
    let dest = paths.linkedin();
    let meta = |name: &str| {
        let path = dest.join(name);
        fs::read_to_string(path)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let date = meta(PULLED_ON);
    let source = meta(SOURCE);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "LinkedIn snapshot {} ({})\n",
        if date.is_empty() { "?" } else { &date },
        if source.is_empty() {
            "source unknown"
        } else {
            &source
        }
    );
    let _ = writeln!(out, "== Headline");
    let _ = writeln!(
        out,
        "  linkedin: {}",
        first["Headline"].as_str().unwrap_or("")
    );
    let _ = writeln!(out, "  master:   {}\n", master.headline(Lang::En));
    // A CSV export carries no About, and an empty one is not a difference.
    let about = first["Summary"].as_str().unwrap_or("").trim();
    let _ = writeln!(out, "== About");
    if about.is_empty() {
        let _ = writeln!(out, "  linkedin: absent in this snapshot\n");
    } else {
        let same = about == master.about(Lang::En).trim();
        let _ = writeln!(out, "  differs: {}\n", if same { "no" } else { "yes" });
    }
    let _ = writeln!(out, "== Positions on LinkedIn");
    for p in &positions {
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
    let _ = writeln!(out, "== Positions in master");
    for e in &master.experience {
        let role = e.role.as_ref().map_or("?", |r| r.get(Lang::En));
        let _ = writeln!(out, "  {}  {role} @ {}", e.date, e.company);
    }
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
        let rows = split_rows("a,b,c\n1,\"x, y\",\"line1\nline2\"\n2,\"he said \"\"hi\"\"\",z\n");
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
        let skills: Vec<Value> = {
            let text = fs::read_to_string(paths.linkedin().join("SKILLS.json")).unwrap();
            serde_json::from_str(&text).unwrap()
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["Name"], "Python");
        assert_eq!(
            fs::read_to_string(paths.linkedin().join(SOURCE))
                .unwrap()
                .trim(),
            "export"
        );
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
        let text = fs::read_to_string(paths.linkedin().join("PROFILE.json")).unwrap();
        let profile: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(profile[0]["Summary"], "First line\nSecond line");
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
