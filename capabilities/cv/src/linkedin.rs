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
use std::process::{Command, Stdio};

/// Keychain item that holds the token (scope `r_dma_portability_self_serve`).
pub const KEYCHAIN_SERVICE: &str = "sjel-linkedin-dma";

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
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", &user, "-w"])
            .output()
    } else {
        Command::new("secret-tool").args(["lookup", "service", KEYCHAIN_SERVICE]).output()
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
        .args(["-sS", "-H", "@-", "-H", "Linkedin-Version: 202312", "-w", "\n%{http_code}", &url])
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
    fs::write(dir.join("PULLED_ON"), chrono::Local::now().format("%Y-%m-%d\n").to_string())
        .map_err(|e| e.to_string())?;
    Ok(report)
}

fn snapshot(paths: &Paths, domain: &str) -> Res<Vec<Value>> {
    let path = paths.linkedin().join(format!("{domain}.json"));
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("{} missing; run 'cv linkedin pull' first", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn field<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("?")
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
    let pulled = fs::read_to_string(paths.linkedin().join("PULLED_ON")).unwrap_or_else(|_| "?".into());
    let mut out = String::new();
    let _ = writeln!(out, "LinkedIn pulled on {}\n", pulled.trim());
    let _ = writeln!(out, "== Headline");
    let _ = writeln!(out, "  linkedin: {}", first["Headline"].as_str().unwrap_or(""));
    let _ = writeln!(out, "  master:   {}\n", master.headline(Lang::En));
    let differs = first["Summary"].as_str().unwrap_or("") != master.about(Lang::En);
    let _ = writeln!(out, "== About differs: {}\n", if differs { "yes" } else { "no" });
    let _ = writeln!(out, "== Positions on LinkedIn");
    for p in &positions {
        let end = p.get("Finished On").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("now");
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
    let theirs: BTreeSet<String> =
        skills.iter().filter_map(|s| s["Name"].as_str()).map(normalize).collect();
    let ours: BTreeSet<String> = master
        .skills
        .iter()
        .flat_map(|s| s.items.get(Lang::En).iter().map(|i| normalize(i)))
        .collect();
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
    let mut out = format!("HEADLINE\n{}\n\nABOUT\n{}\n\nEXPERIENCE\n", master.headline(lang), master.about(lang));
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
    fn skill_names_compare_without_qualifiers_or_case() {
        assert_eq!(normalize("Python (FastAPI, Quart)"), "python");
        assert_eq!(normalize("Kubernetes"), "kubernetes");
    }

    #[test]
    fn sheet_localizes_roles() {
        let mut m = Master::from_yaml(include_str!("../master_cv.schema.yaml")).unwrap();
        m.experience[0].role = Some(serde_yaml_ng::from_str("{en: Role EN, de: Rolle DE}").unwrap());
        assert!(sheet(&m, Lang::De).contains("Rolle DE @ "));
        assert!(sheet(&m, Lang::En).starts_with("HEADLINE\n"));
    }
}
