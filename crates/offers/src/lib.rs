//! Offers under `<overlay>/data/offers/<thread-id>/`: `offer.toml` and the `original.eml` it
//! was read from. Extraction is deterministic and keeps the line each value was read from.
//! A mail with no offer is recorded too, as `no-offer`, so its owner can release it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub type Res<T> = Result<T, String>;

/// Days after the mail date that an offer with no stated expiry stays active.
pub const UNDATED_DAYS: i64 = 30;
/// Latest expiry accepted, in days after the mail date. A later date is rarely the offer's.
const HORIZON_DAYS: i64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Active,
    Used,
    Dismissed,
    NoOffer,
}

impl Status {
    /// # Errors
    /// The name is not `active`, `used` or `dismissed`.
    pub fn parse_settable(name: &str) -> Res<Status> {
        match name {
            "active" => Ok(Status::Active),
            "used" => Ok(Status::Used),
            "dismissed" => Ok(Status::Dismissed),
            other => Err(format!("status '{other}' is not active, used or dismissed")),
        }
    }
}

/// One value read from the mail, with the line it was read from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quoted {
    pub value: String,
    pub evidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    /// `gmail:<thread-id>`.
    pub source: String,
    pub mail_date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Quoted>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount: Option<Quoted>,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Quoted>,
    /// File name of the picture taken from inside the mail, beside `offer.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

impl Offer {
    /// The last day the offer counts as valid: the stated expiry, else the mail date plus
    /// [`UNDATED_DAYS`].
    pub fn last_day(&self) -> Option<NaiveDate> {
        match &self.valid_until {
            Some(until) => day(&until.value),
            None => day(&self.mail_date).map(|d| d + Duration::days(UNDATED_DAYS)),
        }
    }

    /// Whether the mail may go: used, dismissed, no offer, or past its last day.
    pub fn released(&self, today: NaiveDate) -> bool {
        match self.status {
            Status::Used | Status::Dismissed | Status::NoOffer => true,
            Status::Active => self.last_day().is_some_and(|last| last < today),
        }
    }
}

fn day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d").ok()
}

/// The offers root, `<overlay>/data/offers`.
///
/// # Errors
/// No overlay is configured.
pub fn root() -> Res<PathBuf> {
    sjel_config::overlay_data_dir("offers").ok_or_else(|| "SJEL_PERSONAL_ROOT is not set".into())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What `comms mail export` writes beside each `original.eml`.
#[derive(Deserialize)]
struct ExportMeta {
    source: String,
    date: Option<String>,
    from: Option<String>,
    subject: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: usize,
    pub offers: usize,
    pub no_offer: usize,
    pub already_stored: usize,
    pub failed: Vec<String>,
}

/// Import every `<dir>/<thread-id>/` that `comms mail export` wrote. A stored offer is never
/// overwritten, so a status the owner set survives a rerun.
///
/// # Errors
/// The export directory cannot be read.
pub fn import(root: &Path, dir: &Path) -> Res<ImportReport> {
    let mut report = ImportReport::default();
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let id = entry.file_name().to_string_lossy().to_string();
        if !valid_id(&id) {
            continue;
        }
        let target = root.join(&id);
        if target.join("offer.toml").exists() {
            report.already_stored += 1;
            continue;
        }
        match import_one(&entry.path(), &target) {
            Ok(offer) => {
                report.imported += 1;
                if offer.status == Status::NoOffer {
                    report.no_offer += 1;
                } else {
                    report.offers += 1;
                }
            }
            Err(error) => report.failed.push(format!("{id}: {error}")),
        }
    }
    Ok(report)
}

fn import_one(from: &Path, target: &Path) -> Res<Offer> {
    let meta: ExportMeta = serde_json::from_slice(
        &fs::read(from.join("meta.json")).map_err(|e| format!("meta.json: {e}"))?,
    )
    .map_err(|e| format!("meta.json: {e}"))?;
    let eml = fs::read(from.join("original.eml")).map_err(|e| format!("original.eml: {e}"))?;
    let mail_date = meta.date.ok_or("meta.json has no date")?;
    let mail_day = day(&mail_date).ok_or("meta.json date is not YYYY-MM-DD")?;
    let text = mail_text(&eml)?;
    let found = extract(&text, mail_day);
    let offer = Offer {
        source: meta.source,
        mail_date,
        from: meta.from,
        subject: meta.subject,
        status: if found.is_empty() {
            Status::NoOffer
        } else {
            Status::Active
        },
        code: found.code,
        discount: found.discount,
        valid_until: found.valid_until,
        image: None,
    };
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    fs::write(target.join("original.eml"), &eml).map_err(|e| e.to_string())?;
    let offer = Offer {
        image: save_image(&eml, target)?,
        ..offer
    };
    write_offer(target, &offer)?;
    Ok(offer)
}

fn write_offer(folder: &Path, offer: &Offer) -> Res<()> {
    let text = toml::to_string(offer).map_err(|e| e.to_string())?;
    fs::write(folder.join("offer.toml"), text).map_err(|e| e.to_string())
}

/// Every stored offer with its thread id, newest mail first.
///
/// # Errors
/// The root exists but cannot be read, or an `offer.toml` does not parse.
pub fn load(root: &Path) -> Res<Vec<(String, Offer)>> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let id = entry.file_name().to_string_lossy().to_string();
        let path = entry.path().join("offer.toml");
        if !valid_id(&id) || !path.exists() {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let offer: Offer = toml::from_str(&text).map_err(|e| format!("{id}: {e}"))?;
        out.push((id, offer));
    }
    out.sort_by(|a, b| b.1.mail_date.cmp(&a.1.mail_date));
    Ok(out)
}

/// # Errors
/// The id is unknown or the record cannot be written.
pub fn set_status(root: &Path, id: &str, status: Status) -> Res<()> {
    if !valid_id(id) {
        return Err(format!("'{id}' is not a thread id"));
    }
    let folder = root.join(id);
    let text =
        fs::read_to_string(folder.join("offer.toml")).map_err(|_| format!("no offer {id}"))?;
    let mut offer: Offer = toml::from_str(&text).map_err(|e| e.to_string())?;
    offer.status = status;
    write_offer(&folder, &offer)
}

/// Read every stored original again with the current extraction. A `used` or `dismissed`
/// status stays; `active` and `no-offer` follow the new reading. Returns how many changed.
///
/// # Errors
/// A stored record or original cannot be read or written.
pub fn reread(root: &Path) -> Res<usize> {
    let mut changed = 0;
    for (id, offer) in load(root)? {
        let folder = root.join(&id);
        let eml = fs::read(folder.join("original.eml")).map_err(|e| format!("{id}: {e}"))?;
        let mail_day = day(&offer.mail_date).ok_or_else(|| format!("{id}: bad mail_date"))?;
        let found = extract(&mail_text(&eml)?, mail_day);
        let status = match offer.status {
            Status::Used | Status::Dismissed => offer.status,
            Status::Active | Status::NoOffer if found.is_empty() => Status::NoOffer,
            Status::Active | Status::NoOffer => Status::Active,
        };
        let next = Offer {
            status,
            code: found.code,
            discount: found.discount,
            valid_until: found.valid_until,
            image: save_image(&eml, &folder)?,
            ..offer.clone()
        };
        if next != offer {
            write_offer(&folder, &next)?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// The shop's display name from a `From` header: `Rituals Cosmetics <news@…>` gives `Rituals Cosmetics`.
fn shop(from: Option<&str>) -> String {
    let from = from.unwrap_or_default();
    let name = from
        .split('<')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    if name.is_empty() {
        from.trim_matches(['<', '>']).to_string()
    } else {
        name.to_string()
    }
}

/// A self-contained page of the valid offers, soonest expiry first. It loads nothing from the
/// network: pictures and mails are files beside it.
pub fn page(offers: &[(String, Offer)], today: NaiveDate) -> String {
    let mut valid: Vec<&(String, Offer)> =
        offers.iter().filter(|(_, o)| !o.released(today)).collect();
    valid.sort_by_key(|(_, o)| o.last_day());
    let mut cards = String::new();
    for (id, offer) in &valid {
        let days = offer.last_day().map(|d| (d - today).num_days());
        let left = match days {
            Some(0) => "last day".to_string(),
            Some(1) => "1 day left".to_string(),
            Some(n) => format!("{n} days left"),
            None => String::new(),
        };
        let until = match &offer.valid_until {
            Some(q) => format!(
                "<span title=\"{}\">until {}</span>",
                escape(&q.evidence),
                escape(&q.value)
            ),
            None => "no stated end".into(),
        };
        let image = offer.image.as_ref().map_or_else(
            || "<div class=\"img none\"></div>".to_string(),
            |name| {
                format!(
                    "<img class=\"img\" src=\"{}/{}\" alt=\"\">",
                    escape(id),
                    escape(name)
                )
            },
        );
        let discount = offer.discount.as_ref().map_or_else(String::new, |q| {
            format!(
                "<span class=\"badge\" title=\"{}\">−{}</span>",
                escape(&q.evidence),
                escape(&q.value)
            )
        });
        let code = offer.code.as_ref().map_or_else(String::new, |q| {
            format!(
                "<button class=\"code\" title=\"{}\" onclick=\"copy(this)\">{}</button>",
                escape(&q.evidence),
                escape(&q.value)
            )
        });
        let urgent = if days.is_some_and(|n| n <= 2) {
            " urgent"
        } else {
            ""
        };
        cards.push_str(&format!(
            "<article class=\"card\">{image}<div class=\"body\"><div class=\"shop\">{shop}</div>\
             <h2>{subject}</h2><div class=\"row\">{discount}{code}</div>\
             <div class=\"meta{urgent}\">{left} · {until}</div>\
             <a class=\"open\" href=\"{id}/original.eml\">Open mail</a></div></article>\n",
            shop = escape(&shop(offer.from.as_deref())),
            subject = escape(offer.subject.as_deref().unwrap_or_default()),
            id = escape(id),
        ));
    }
    if valid.is_empty() {
        cards.push_str("<p class=\"empty\">No valid offers.</p>");
    }
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self' file:; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Offers</title>
<style>
:root {{ color-scheme: light dark; --bg:#f6f5f2; --card:#fff; --ink:#1d1d1f; --mute:#6e6e73; --line:#e4e2dd; --accent:#2f6f4f; --warn:#b3261e; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#161616; --card:#222; --ink:#f2f2f2; --mute:#a1a1a6; --line:#333; --accent:#7cc8a0; --warn:#ff8a80; }} }}
body {{ margin:0; background:var(--bg); color:var(--ink); font:15px/1.4 -apple-system, system-ui, sans-serif; }}
header {{ padding:28px 32px 8px; }} h1 {{ margin:0; font-size:26px; }} header p {{ margin:4px 0 0; color:var(--mute); }}
main {{ display:grid; grid-template-columns:repeat(auto-fill, minmax(260px, 1fr)); gap:18px; padding:20px 32px 40px; }}
.card {{ background:var(--card); border:1px solid var(--line); border-radius:14px; overflow:hidden; display:flex; flex-direction:column; }}
.img {{ width:100%; aspect-ratio:4/3; object-fit:cover; display:block; background:var(--line); }}
.img.none {{ aspect-ratio:8/1; }}
.body {{ padding:14px 16px 16px; display:flex; flex-direction:column; gap:8px; flex:1; }}
.shop {{ color:var(--mute); font-size:12px; text-transform:uppercase; letter-spacing:.06em; }}
h2 {{ margin:0; font-size:16px; font-weight:600; }}
.row {{ display:flex; gap:8px; flex-wrap:wrap; }}
.badge {{ background:var(--accent); color:var(--card); border-radius:999px; padding:2px 10px; font-weight:600; }}
.code {{ font:600 13px ui-monospace, monospace; border:1px dashed var(--accent); color:var(--accent); background:none; border-radius:8px; padding:2px 10px; cursor:pointer; }}
.meta {{ color:var(--mute); font-size:13px; margin-top:auto; }} .meta.urgent {{ color:var(--warn); font-weight:600; }}
.open {{ color:var(--accent); font-size:13px; text-decoration:none; }}
.empty {{ color:var(--mute); }}
</style></head><body>
<header><h1>Offers</h1><p>{count} valid · built {today}</p></header>
<main>
{cards}</main>
<script>
function copy(el) {{
  const text = el.textContent;
  const done = () => {{ el.textContent = 'copied'; setTimeout(() => el.textContent = text, 1200); }};
  if (navigator.clipboard) {{ navigator.clipboard.writeText(text).then(done, () => fallback(text, done)); }} else {{ fallback(text, done); }}
}}
function fallback(text, done) {{
  const area = document.createElement('textarea'); area.value = text; document.body.appendChild(area);
  area.select(); document.execCommand('copy'); area.remove(); done();
}}
</script></body></html>
"#,
        count = valid.len(),
        today = today.format("%Y-%m-%d"),
    )
}

/// Thread ids whose mail may go, for `comms mail cleanup --released`.
pub fn released(offers: &[(String, Offer)], today: NaiveDate) -> BTreeSet<String> {
    offers
        .iter()
        .filter(|(_, offer)| offer.released(today))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The readable text of a mail: subject, then every `text/plain` part, else every
/// `text/html` part as text.
///
/// # Errors
/// The bytes are not a parseable mail.
pub fn mail_text(eml: &[u8]) -> Res<String> {
    let parsed = mailparse::parse_mail(eml).map_err(|e| e.to_string())?;
    let mut plain = Vec::new();
    let mut html = Vec::new();
    collect(&parsed, &mut plain, &mut html);
    let body = if plain.iter().any(|p| !p.trim().is_empty()) {
        plain.join("\n")
    } else {
        html.iter()
            .filter_map(|h| html2text::from_read(h.as_bytes(), 200).ok())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let subject = parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case("subject"))
        .map(|h| h.get_value())
        .unwrap_or_default();
    Ok(format!("{subject}\n{body}"))
}

/// Smallest picture kept. Smaller parts are tracking pixels, spacers and icons.
const MIN_IMAGE_BYTES: usize = 8 * 1024;

/// The largest picture carried inside the mail, as `(extension, bytes)`. Pictures the mail
/// only links to are never fetched: loading them tells the sender the mail was opened.
///
/// # Errors
/// The bytes are not a parseable mail.
pub fn embedded_image(eml: &[u8]) -> Res<Option<(&'static str, Vec<u8>)>> {
    let parsed = mailparse::parse_mail(eml).map_err(|e| e.to_string())?;
    let mut best: Option<(&'static str, Vec<u8>)> = None;
    walk_images(&parsed, &mut best);
    Ok(best)
}

fn walk_images(part: &mailparse::ParsedMail<'_>, best: &mut Option<(&'static str, Vec<u8>)>) {
    let ext = match part.ctype.mimetype.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    };
    if let (Some(ext), Ok(bytes)) = (ext, part.get_body_raw()) {
        let larger = best.as_ref().is_none_or(|(_, b)| bytes.len() > b.len());
        if bytes.len() >= MIN_IMAGE_BYTES && larger {
            *best = Some((ext, bytes));
        }
    }
    for sub in &part.subparts {
        walk_images(sub, best);
    }
}

/// Write the mail's own picture as `image.<ext>` and return its file name.
fn save_image(eml: &[u8], folder: &Path) -> Res<Option<String>> {
    let Some((ext, bytes)) = embedded_image(eml)? else {
        return Ok(None);
    };
    let name = format!("image.{ext}");
    fs::write(folder.join(&name), bytes).map_err(|e| e.to_string())?;
    Ok(Some(name))
}

fn collect(part: &mailparse::ParsedMail<'_>, plain: &mut Vec<String>, html: &mut Vec<String>) {
    let mime = part.ctype.mimetype.to_ascii_lowercase();
    if part.subparts.is_empty() {
        if let Ok(body) = part.get_body() {
            match mime.as_str() {
                "text/plain" => plain.push(body),
                "text/html" => html.push(body),
                _ => {}
            }
        }
    }
    for sub in &part.subparts {
        collect(sub, plain, html);
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Extracted {
    pub code: Option<Quoted>,
    pub discount: Option<Quoted>,
    pub valid_until: Option<Quoted>,
}

impl Extracted {
    pub fn is_empty(&self) -> bool {
        self.code.is_none() && self.discount.is_none() && self.valid_until.is_none()
    }
}

static CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i:\b(?:gutschein|rabatt|promo|discount|coupon|voucher)?\s?code)\s*[:：]?\s*[„“\x22']?([A-Z0-9][A-Z0-9-]{3,19})\b")
        .expect("code pattern")
});
static DISCOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^\d])(-?)(\d{1,2})\s?%").expect("discount pattern"));
static NUMERIC_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(\d{1,2})\.(\d{1,2})\.(\d{4}|\d{2})?").expect("numeric date pattern")
});
static GERMAN_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d{1,2})\.\s?(januar|februar|märz|maerz|april|mai|juni|juli|august|september|oktober|november|dezember)(?:\s+(\d{4}))?")
        .expect("german date pattern")
});
static ENGLISH_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(january|february|march|april|may|june|july|august|september|october|november|december)\s+(\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(\d{4}))?")
        .expect("english date pattern")
});
/// A percentage is a discount only with a minus sign before it or one of these on its line.
const DISCOUNT_CUES: [&str; 11] = [
    "rabatt",
    "spar",
    "reduziert",
    "günstiger",
    "sale",
    "off",
    "discount",
    "save",
    "nachlass",
    "auf alles",
    "statt",
];
/// A line names an expiry only with one of these words.
const EXPIRY_CUES: [&str; 14] = [
    "bis",
    "until",
    "endet",
    "ends",
    "gültig",
    "valid",
    "läuft",
    "expires",
    "nur noch",
    "letzte",
    "last chance",
    "through",
    "spätestens",
    "deadline",
];

/// Read code, discount and expiry from mail text. Each value keeps its line as evidence.
pub fn extract(text: &str, mail_day: NaiveDate) -> Extracted {
    let mut found = Extracted::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let evidence = || line.chars().take(200).collect::<String>();
        if found.code.is_none() {
            if let Some(code) = CODE.captures(line).and_then(|c| c.get(1)) {
                let value = code.as_str();
                // A code holds a digit or is all capitals; this skips words like "Code Ihre".
                if value.bytes().any(|b| b.is_ascii_digit()) || value.len() >= 5 {
                    found.code = Some(Quoted {
                        value: value.to_string(),
                        evidence: evidence(),
                    });
                }
            }
        }
        let lower = line.to_lowercase();
        if found.discount.is_none() {
            let cued = DISCOUNT_CUES.iter().any(|cue| lower.contains(cue));
            if let Some(percent) = DISCOUNT
                .captures(line)
                .filter(|c| cued || c.get(1).is_some_and(|m| !m.is_empty()))
                .and_then(|c| c.get(2))
            {
                if percent
                    .as_str()
                    .parse::<u32>()
                    .is_ok_and(|p| (5..=90).contains(&p))
                {
                    found.discount = Some(Quoted {
                        value: format!("{}%", percent.as_str()),
                        evidence: evidence(),
                    });
                }
            }
        }
        if EXPIRY_CUES.iter().any(|cue| lower.contains(cue)) {
            for date in dates(line, mail_day) {
                let later = found
                    .valid_until
                    .as_ref()
                    .and_then(|q| day(&q.value))
                    .is_none_or(|current| date > current);
                if later {
                    found.valid_until = Some(Quoted {
                        value: date.format("%Y-%m-%d").to_string(),
                        evidence: evidence(),
                    });
                }
            }
        }
    }
    found
}

/// Dates written in the line that fall between the mail date and the horizon.
fn dates(line: &str, mail_day: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    for c in NUMERIC_DATE.captures_iter(line) {
        push_date(&mut out, mail_day, num(&c, 3), num(&c, 2), num(&c, 1));
    }
    for c in GERMAN_DATE.captures_iter(line) {
        let month = month_number(&c[2]);
        push_date(&mut out, mail_day, num(&c, 3), month, num(&c, 1));
    }
    for c in ENGLISH_DATE.captures_iter(line) {
        let month = month_number(&c[1]);
        push_date(&mut out, mail_day, num(&c, 3), month, num(&c, 2));
    }
    out
}

fn num(c: &regex::Captures<'_>, i: usize) -> Option<u32> {
    c.get(i).and_then(|m| m.as_str().parse().ok())
}

fn push_date(
    out: &mut Vec<NaiveDate>,
    mail_day: NaiveDate,
    year: Option<u32>,
    month: Option<u32>,
    dom: Option<u32>,
) {
    let (Some(month), Some(dom)) = (month, dom) else {
        return;
    };
    let years: Vec<i32> = match year {
        Some(y) if y < 100 => vec![2000 + i32::try_from(y).unwrap_or(0)],
        Some(y) => vec![i32::try_from(y).unwrap_or(0)],
        // A date without a year is this year's, or next year's if that has passed.
        None => vec![mail_day.year(), mail_day.year() + 1],
    };
    let horizon = mail_day + Duration::days(HORIZON_DAYS);
    if let Some(date) = years
        .into_iter()
        .filter_map(|y| NaiveDate::from_ymd_opt(y, month, dom))
        .find(|d| *d >= mail_day && *d <= horizon)
    {
        out.push(date);
    }
}

fn month_number(name: &str) -> Option<u32> {
    let name = name.to_lowercase();
    let months = [
        ["januar", "january"],
        ["februar", "february"],
        ["märz", "march"],
        ["april", "april"],
        ["mai", "may"],
        ["juni", "june"],
        ["juli", "july"],
        ["august", "august"],
        ["september", "september"],
        ["oktober", "october"],
        ["november", "november"],
        ["dezember", "december"],
    ];
    if name == "maerz" {
        return Some(3);
    }
    months
        .iter()
        .position(|pair| pair.contains(&name.as_str()))
        .and_then(|i| u32::try_from(i + 1).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn code_discount_and_expiry_come_with_their_lines() {
        let text = "Green Friday\nSpare 20% auf alles\nMit dem Code GREEN20 an der Kasse\nGültig bis 30.11.";
        let found = extract(text, d("2026-11-20"));
        assert_eq!(found.code.unwrap().value, "GREEN20");
        assert_eq!(found.discount.unwrap().value, "20%");
        let until = found.valid_until.unwrap();
        assert_eq!(until.value, "2026-11-30");
        assert_eq!(until.evidence, "Gültig bis 30.11.");
    }

    #[test]
    fn a_percentage_without_a_discount_word_is_no_discount() {
        let found = extract("66 % der Menschen haben Probleme", d("2026-07-31"));
        assert!(found.discount.is_none());
        let found = extract("Jetzt 25 % Rabatt sichern", d("2026-07-31"));
        assert_eq!(found.discount.unwrap().value, "25%");
        let found = extract("Frühstück - 66 % der Menschen", d("2026-07-31"));
        assert!(found.discount.is_none());
        let found = extract("-20 % auf Nachfüllpackungen", d("2026-07-31"));
        assert_eq!(found.discount.unwrap().value, "20%");
    }

    #[test]
    fn written_months_and_year_rollover_parse() {
        let found = extract("Angebot endet am 3. Januar", d("2026-12-20"));
        assert_eq!(found.valid_until.unwrap().value, "2027-01-03");
        let found = extract("Offer valid until December 1st, 2026", d("2026-11-20"));
        assert_eq!(found.valid_until.unwrap().value, "2026-12-01");
    }

    #[test]
    fn dates_without_a_cue_or_out_of_range_are_ignored() {
        assert!(extract("Lieferung am 30.11.", d("2026-11-20"))
            .valid_until
            .is_none());
        assert!(extract("gültig bis 01.01.2020", d("2026-11-20"))
            .valid_until
            .is_none());
        assert!(extract("Hallo, wie geht es?", d("2026-11-20")).is_empty());
    }

    #[test]
    fn release_follows_status_and_last_day() {
        let mut offer = Offer {
            source: "gmail:a1".into(),
            mail_date: "2026-11-01".into(),
            from: None,
            subject: None,
            status: Status::Active,
            code: None,
            discount: None,
            valid_until: None,
            image: None,
        };
        assert!(!offer.released(d("2026-12-01")));
        assert!(offer.released(d("2026-12-02")));
        offer.valid_until = Some(Quoted {
            value: "2026-11-10".into(),
            evidence: String::new(),
        });
        assert!(offer.released(d("2026-11-11")));
        assert!(!offer.released(d("2026-11-10")));
        offer.status = Status::Used;
        assert!(offer.released(d("2026-11-02")));
    }

    #[test]
    fn only_a_large_enough_embedded_picture_is_kept() {
        use std::fmt::Write as _;
        let big = "A".repeat(12 * 1024);
        let mut eml = String::from("Subject: Sale\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=b\r\n\r\n");
        write!(
            eml,
            "--b\r\nContent-Type: text/html\r\n\r\n<img src=\"https://shop.example/x.jpg\">\r\n"
        )
        .unwrap();
        write!(eml, "--b\r\nContent-Type: image/gif\r\n\r\nGIF89a\r\n").unwrap();
        write!(
            eml,
            "--b\r\nContent-Type: image/png\r\n\r\n{big}\r\n--b--\r\n"
        )
        .unwrap();
        let (ext, bytes) = embedded_image(eml.as_bytes()).unwrap().unwrap();
        assert_eq!(ext, "png");
        assert!(bytes.len() >= MIN_IMAGE_BYTES);
        let none = b"Subject: x\r\nContent-Type: text/html\r\n\r\n<img src=\"https://shop.example/x.jpg\">\r\n";
        assert!(embedded_image(none).unwrap().is_none());
    }

    #[test]
    fn the_page_escapes_mail_text_and_loads_nothing_remote() {
        let offer = Offer {
            source: "gmail:a1".into(),
            mail_date: "2026-11-20".into(),
            from: Some("Shop <news@shop.example>".into()),
            subject: Some("<script>alert(1)</script> Sale".into()),
            status: Status::Active,
            code: Some(Quoted {
                value: "SAVE10".into(),
                evidence: "Code \"SAVE10\"".into(),
            }),
            discount: None,
            valid_until: Some(Quoted {
                value: "2026-11-22".into(),
                evidence: "bis 22.11.".into(),
            }),
            image: None,
        };
        let html = page(&[("a1".into(), offer)], d("2026-11-21"));
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("1 day left"));
        assert!(html.contains("default-src 'none'"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn mail_text_prefers_plain_and_falls_back_to_html() {
        let plain = b"Subject: Sale\r\nContent-Type: text/plain\r\n\r\nNur bis 30.11.\r\n";
        assert!(mail_text(plain).unwrap().contains("Nur bis 30.11."));
        let html = b"Subject: Sale\r\nContent-Type: text/html\r\n\r\n<p>Code <b>XMAS25</b></p>\r\n";
        assert!(mail_text(html).unwrap().contains("XMAS25"));
    }
}
