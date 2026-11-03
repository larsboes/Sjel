//! Whether a swept mail comes from the sender it names.
//!
//! The header rung reads only metadata the sweep already fetches. It trusts the
//! `Authentication-Results` header that Gmail itself stamps (`mx.google.com`):
//! every other instance of that header arrives from the sender and proves
//! nothing (RFC 8601 section 5).
//!
//! A finding names domains only, never a local part, a subject or a snippet.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::mail_model::sender_domain;

pub const AUTHENTICITY_REVISION: &str = "mail-authenticity-v1";

/// The authserv-id of the receiving Gmail server.
const TRUSTED_AUTHSERV_ID: &str = "mx.google.com";

/// Ordered from best to worst, so the combined verdict is the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Verified,
    Unverified,
    Suspicious,
    Failed,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Suspicious => "suspicious",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verified" => Some(Self::Verified),
            "unverified" => Some(Self::Unverified),
            "suspicious" => Some(Self::Suspicious),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One observation. `verdict` is the worst verdict it allows; an informational
/// finding carries `Verified` and so changes nothing. A rung with no findings
/// raises no doubt, so its verdict is `Verified`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub verdict: Verdict,
    pub detail: String,
}

impl Finding {
    fn new(code: &str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            verdict,
            detail: detail.into(),
        }
    }
}

/// Which rung wrote an assessment. A thread holds at most one per source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Headers,
    Body,
    Model,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Body => "body",
            Self::Model => "model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    pub source: Source,
    pub verdict: Verdict,
    pub findings: Vec<Finding>,
    pub revision: String,
}

impl Assessment {
    pub fn from_findings(source: Source, findings: Vec<Finding>) -> Self {
        let verdict = findings
            .iter()
            .map(|finding| finding.verdict)
            .max()
            .unwrap_or(Verdict::Verified);
        Self {
            source,
            verdict,
            findings,
            revision: AUTHENTICITY_REVISION.into(),
        }
    }
}

/// The metadata the header rung reads.
pub struct HeaderFacts<'a> {
    pub from: &'a str,
    pub subject: &'a str,
    pub authentication_results: &'a [String],
    pub reply_to: Option<&'a str>,
}

/// Organisational domains of earlier senders. `None` when the caller has no
/// store, which skips the two checks that need history.
pub type KnownOrgs = Option<BTreeSet<String>>;

pub fn assess_headers(facts: &HeaderFacts<'_>, known: &KnownOrgs) -> Assessment {
    let mut findings = Vec::new();
    let from_domain = sender_domain(facts.from);
    let from_org = from_domain.as_deref().map(org_domain);

    findings.push(authentication_finding(
        facts.authentication_results,
        from_org.as_deref(),
    ));

    if is_forward(facts.subject) {
        findings.push(Finding::new(
            "forwarded",
            Verdict::Unverified,
            "Forwarded mail: the headers prove the forwarder, not the original sender.",
        ));
    }

    if let (Some(domain), Some(org)) = (from_domain.as_deref(), from_org.as_deref()) {
        if domain.split('.').any(|label| label.starts_with("xn--")) {
            findings.push(Finding::new(
                "punycode_domain",
                Verdict::Suspicious,
                format!("Sender domain {domain} uses punycode, which can imitate another name."),
            ));
        }
        if let Some(named) = display_name_domain(facts.from) {
            if org_domain(&named) != org {
                // A sending platform sends for other brands by design.
                let platform = TRACKERS.contains(&org);
                findings.push(Finding::new(
                    "display_name_domain",
                    if platform { Verdict::Verified } else { Verdict::Suspicious },
                    format!("The display name shows {named}, but the mail is from {domain}."),
                ));
            }
        }
        if let Some(reply_org) = facts.reply_to.and_then(sender_domain).map(|d| org_domain(&d)) {
            if reply_org != org {
                let freemail = FREEMAIL.contains(&reply_org.as_str());
                findings.push(Finding::new(
                    "reply_to_mismatch",
                    if freemail { Verdict::Suspicious } else { Verdict::Verified },
                    format!("Replies go to {reply_org}, not {org}."),
                ));
            }
        }
        if let Some(known) = known {
            if !known.contains(org) {
                if let Some(original) = lookalike_of(org, known) {
                    findings.push(Finding::new(
                        "lookalike_domain",
                        Verdict::Suspicious,
                        format!("{org} looks like {original}, a sender you had mail from before."),
                    ));
                } else {
                    findings.push(Finding::new(
                        "new_sender",
                        Verdict::Verified,
                        format!("First mail from {org}."),
                    ));
                }
            }
        }
    }

    Assessment::from_findings(Source::Headers, findings)
}

fn authentication_finding(results: &[String], from_org: Option<&str>) -> Finding {
    let Some(from_org) = from_org else {
        return Finding::new("no_sender", Verdict::Unverified, "The From header names no domain.");
    };
    let Some(trusted) = results.iter().find_map(|header| trusted_results(header)) else {
        return Finding::new(
            "no_auth_result",
            Verdict::Unverified,
            "Gmail recorded no authentication result for this mail.",
        );
    };
    let aligned = |domain: &str| org_domain(domain) == from_org;

    if let Some(dmarc) = trusted.iter().find(|m| m.method == "dmarc") {
        match dmarc.result.as_str() {
            "pass" => {
                return match dmarc.property("header.from") {
                    Some(domain) if !aligned(domain) => Finding::new(
                        "dmarc_other_domain",
                        Verdict::Suspicious,
                        format!("DMARC passed for {domain}, not for the sender {from_org}."),
                    ),
                    _ => Finding::new(
                        "dmarc_pass",
                        Verdict::Verified,
                        format!("DMARC passed for {from_org}."),
                    ),
                };
            }
            "fail" => {
                return Finding::new(
                    "dmarc_fail",
                    Verdict::Failed,
                    format!("DMARC failed: the mail is not authorised by {from_org}."),
                );
            }
            _ => {}
        }
    }

    let dkim_aligned = trusted.iter().any(|m| {
        m.method == "dkim"
            && m.result == "pass"
            && m.property("header.d")
                .or_else(|| m.property("header.i").map(|i| i.trim_start_matches('@')))
                .and_then(|i| i.rsplit('@').next())
                .is_some_and(aligned)
    });
    if dkim_aligned {
        return Finding::new(
            "dkim_aligned",
            Verdict::Verified,
            format!("A DKIM signature of {from_org} verified."),
        );
    }
    let spf_aligned = trusted.iter().any(|m| {
        m.method == "spf"
            && m.result == "pass"
            && m.property("smtp.mailfrom")
                .and_then(|address| address.rsplit('@').next())
                .is_some_and(aligned)
    });
    if spf_aligned {
        return Finding::new(
            "spf_aligned",
            Verdict::Verified,
            format!("SPF passed for {from_org}."),
        );
    }
    let failed = |method: &str| {
        trusted.iter().any(|m| {
            m.method == method && matches!(m.result.as_str(), "fail" | "softfail" | "permerror")
        })
    };
    if failed("dkim") || failed("spf") {
        return Finding::new(
            "auth_fail",
            Verdict::Failed,
            format!("SPF or DKIM failed, and nothing authenticates {from_org}."),
        );
    }
    Finding::new(
        "not_aligned",
        Verdict::Unverified,
        format!("No check that passed is aligned with {from_org}."),
    )
}

#[derive(Debug, PartialEq)]
struct MethodResult {
    method: String,
    result: String,
    properties: Vec<(String, String)>,
}

impl MethodResult {
    fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The method results of one header, when Gmail stamped it.
fn trusted_results(header: &str) -> Option<Vec<MethodResult>> {
    let header = strip_comments(header);
    let mut parts = header.split(';');
    let authserv = parts.next()?.split_whitespace().next()?;
    if !authserv.eq_ignore_ascii_case(TRUSTED_AUTHSERV_ID) {
        return None;
    }
    Some(
        parts
            .filter_map(|part| {
                let mut tokens = part.split_whitespace();
                let (method, result) = tokens.next()?.split_once('=')?;
                let properties = tokens
                    .filter_map(|token| token.split_once('='))
                    .map(|(key, value)| (key.to_ascii_lowercase(), value.to_ascii_lowercase()))
                    .collect();
                Some(MethodResult {
                    method: method.to_ascii_lowercase(),
                    result: result.to_ascii_lowercase(),
                    properties,
                })
            })
            .collect(),
    )
}

/// RFC 8601 permits parenthesised comments anywhere; they may contain `;`.
fn strip_comments(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    let mut depth = 0usize;
    for c in header.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// `WG:`, `Fwd:`, `FW:` and their variants at the start of a subject.
pub fn is_forward(subject: &str) -> bool {
    let head = subject.trim_start().to_ascii_lowercase();
    ["wg:", "fwd:", "fw:", "tr:", "wtrlt:"]
        .iter()
        .any(|prefix| head.starts_with(prefix))
}

/// A domain written into the display name, such as `"paypal.com" <x@evil.test>`.
fn display_name_domain(from: &str) -> Option<String> {
    let name = &from[..from.rfind('<')?];
    name.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | ',' | '<'))
        .filter_map(|token| {
            let token = token.rsplit('@').next()?.trim_matches('.').to_ascii_lowercase();
            let (_, tld) = token.rsplit_once('.')?;
            (tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
                && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .then_some(token)
        })
        .next()
}

/// Suffixes under which the registrable domain has three labels.
// ponytail: fixed list instead of the Public Suffix List; replace with the PSL
// when a sender under another multi-label suffix is misgrouped.
const MULTI_LABEL_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.jp", "co.nz",
    "com.br", "com.cn", "co.at", "or.at", "co.za", "com.tr", "co.in", "com.mx",
];

/// The registrable domain: `em.cloudfest.com` becomes `cloudfest.com`.
pub fn org_domain(domain: &str) -> String {
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').collect();
    let keep = if labels.len() >= 3
        && MULTI_LABEL_SUFFIXES.contains(&labels[labels.len() - 2..].join(".").as_str())
    {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

const FREEMAIL: &[&str] = &[
    "gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "live.com", "yahoo.com",
    "icloud.com", "me.com", "gmx.de", "gmx.net", "web.de", "proton.me", "protonmail.com",
    "aol.com", "mail.ru", "yandex.ru", "t-online.de",
];

/// A known organisational domain that `org` imitates: one edit apart, or equal
/// once common look-alike characters are folded.
// ponytail: compares only the first label; a TLD swap (paypal.co) is not caught.
fn lookalike_of<'k>(org: &str, known: &'k BTreeSet<String>) -> Option<&'k str> {
    let label = org.split('.').next()?;
    if label.len() < 5 {
        return None;
    }
    let folded = fold_confusables(label);
    known
        .iter()
        .filter(|candidate| candidate.as_str() != org)
        .find(|candidate| {
            let Some(other) = candidate.split('.').next() else {
                return false;
            };
            other.len() >= 5
                && other != label
                && (edit_distance(label, other) <= 1 || folded == fold_confusables(other))
        })
        .map(String::as_str)
}

fn fold_confusables(label: &str) -> String {
    label
        .replace("rn", "m")
        .replace("vv", "w")
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            '3' => 'e',
            '5' => 's',
            _ => c,
        })
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

/// The decoded parts of one message, as the body rung reads them.
pub struct BodyFacts<'a> {
    pub from: &'a str,
    pub subject: &'a str,
    pub html: Option<&'a str>,
    pub text: Option<&'a str>,
    /// Whether the header rung authenticated the `From` domain.
    pub sender_authenticated: bool,
}

/// Hosts that rewrite links for click tracking on a sender's behalf.
const TRACKERS: &[&str] = &[
    "hubspotlinks.com", "hubspotemail.net", "hubspotlinks-eu1.com", "hubspotemail-eu1.net",
    "hs-sites.com", "hs-sites-eu1.com", "list-manage.com", "mailchi.mp",
    "sendgrid.net", "mailgun.org", "sparkpostmail.com", "mandrillapp.com", "rs6.net",
    "exacttarget.com", "mktomail.com", "mkt.com", "klaviyo.com", "brevo.com", "sendibt3.com",
    "sendinblue.com", "awstrack.me", "beehiiv.com", "substack.com", "cmail19.com",
    "cmail20.com", "createsend.com", "postmarkapp.com", "e.customeriomail.com",
    "customeriomail.com", "safelinks.protection.outlook.com", "urldefense.com",
];

const SHORTENERS: &[&str] = &[
    "bit.ly", "tinyurl.com", "goo.gl", "ow.ly", "is.gd", "buff.ly", "rebrand.ly", "cutt.ly",
    "t.ly", "shorturl.at", "rb.gy", "tiny.cc",
];

/// Links, forms and, for a forward, the original sender.
pub fn assess_body(facts: &BodyFacts<'_>) -> Assessment {
    let mut findings = Vec::new();
    let text = facts
        .text
        .map(str::to_string)
        .or_else(|| facts.html.map(sjel_extraction::html_to_lines))
        .unwrap_or_default();

    let mut sender = sender_domain(facts.from);
    if is_forward(facts.subject) {
        match forwarded_sender(&text) {
            Some(original) => {
                findings.push(Finding::new(
                    "forwarded_sender",
                    Verdict::Unverified,
                    format!("The forwarded mail names {original} as its sender; that claim is not authenticated."),
                ));
                sender = Some(original);
            }
            None => findings.push(Finding::new(
                "forward_unparsed",
                Verdict::Unverified,
                "No original sender found in the forwarded text.",
            )),
        }
    }
    let sender_org = sender.as_deref().map(org_domain);

    let links = match facts.html {
        Some(html) => anchors(html),
        None => text_links(&text),
    };
    // A forward's original sender is never authenticated by these headers.
    let authenticated = facts.sender_authenticated && !is_forward(facts.subject);
    findings.extend(link_findings(&links, sender_org.as_deref(), authenticated));

    if facts
        .html
        .is_some_and(|html| html.to_ascii_lowercase().contains("<form"))
    {
        findings.push(Finding::new(
            "form_in_mail",
            Verdict::Suspicious,
            "The mail contains a form; a legitimate sender asks you to enter data on its site.",
        ));
    }
    Assessment::from_findings(Source::Body, findings)
}

/// One link: where it goes, and the text a reader sees.
#[derive(Debug, PartialEq)]
struct Link {
    href: String,
    label: String,
}

fn link_findings(
    links: &[Link],
    sender_org: Option<&str>,
    sender_authenticated: bool,
) -> Vec<Finding> {
    let brand = |org: &str| org.split('.').next().map(str::to_string);
    let sender_brand = sender_org.and_then(brand);
    let mut findings = Vec::new();
    let mut own = 0usize;
    let mut tracked = 0usize;
    let mut others = BTreeSet::new();
    let mut flagged = BTreeSet::new();
    let mut flag = |code: &str, verdict: Verdict, detail: String| {
        if flagged.insert((code.to_string(), detail.clone())) {
            findings.push(Finding::new(code, verdict, detail));
        }
    };

    for link in links {
        let href = link.href.trim();
        let lower = href.to_ascii_lowercase();
        if lower.starts_with("mailto:") || lower.starts_with("tel:") || lower.starts_with('#') {
            continue;
        }
        if lower.starts_with("javascript:") || lower.starts_with("data:") {
            flag(
                "script_link",
                Verdict::Failed,
                "A link runs a script or embeds data instead of opening a page.".into(),
            );
            continue;
        }
        let Ok(url) = reqwest::Url::parse(href) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        // `domain()` is `None` for an IP literal, `host_str()` is not.
        let Some(host) = url.domain().map(str::to_ascii_lowercase) else {
            if let Some(address) = url.host_str() {
                flag(
                    "ip_link",
                    Verdict::Suspicious,
                    format!("A link goes to the bare address {address}, not to a named site."),
                );
            }
            continue;
        };
        if host.split('.').any(|label| label.starts_with("xn--")) {
            flag(
                "punycode_link",
                Verdict::Suspicious,
                format!("A link goes to {host}, a punycode name that can imitate another."),
            );
        }
        let org = org_domain(&host);
        if SHORTENERS.contains(&org.as_str()) || SHORTENERS.contains(&host.as_str()) {
            flag(
                "shortened_link",
                Verdict::Unverified,
                format!("A link is shortened through {host}, which hides where it goes."),
            );
        }
        let is_own = sender_org == Some(org.as_str());
        let is_tracker = TRACKERS.iter().any(|t| host == *t || host.ends_with(&format!(".{t}")));
        if let Some(shown) = label_domain(&link.label) {
            let shown_org = org_domain(&shown);
            // An authenticated sender that shows its own brand is not
            // pretending to be someone else, wherever its redirect lives.
            let own_brand = sender_authenticated && brand(&shown_org) == sender_brand;
            if shown_org != org && !is_tracker && !is_own && !own_brand {
                // An authenticated sender can still be a compromised account.
                flag(
                    "deceptive_link_text",
                    if sender_authenticated { Verdict::Unverified } else { Verdict::Suspicious },
                    format!("A link shows {shown} but goes to {host}."),
                );
            }
        }
        if is_own {
            own += 1;
        } else if is_tracker {
            tracked += 1;
        } else {
            others.insert(org);
        }
    }

    let total = own + tracked + others.len();
    if total > 0 {
        let mut detail = format!(
            "{} link(s) to the sender's domain, {tracked} through a tracking service",
            own
        );
        if !others.is_empty() {
            let list: Vec<&str> = others.iter().take(6).map(String::as_str).collect();
            detail.push_str(&format!(", and links to {}", list.join(", ")));
            if others.len() > 6 {
                detail.push_str(&format!(" and {} more", others.len() - 6));
            }
        }
        detail.push('.');
        findings.push(Finding::new("link_summary", Verdict::Verified, detail));
    }
    findings
}

/// The domain a link label displays, when the label is a URL or a bare domain.
fn label_domain(label: &str) -> Option<String> {
    let label = label.trim().trim_end_matches('/');
    if label.contains(char::is_whitespace) || !label.contains('.') {
        return None;
    }
    let candidate = if label.contains("://") {
        reqwest::Url::parse(label).ok()?.host_str()?.to_string()
    } else {
        label.split('/').next()?.to_string()
    };
    let candidate = candidate.to_ascii_lowercase();
    let (_, tld) = candidate.rsplit_once('.')?;
    (tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && candidate.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    .then_some(candidate)
}

/// `<a href="…">label</a>` pairs. A scanner, not a parser: mail HTML is
/// malformed often enough that a strict parser loses links.
fn anchors(html: &str) -> Vec<Link> {
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<a").map(|i| at + i) {
        let Some(tag_end) = lower[start..].find('>').map(|i| start + i) else {
            break;
        };
        at = tag_end + 1;
        let next = lower.as_bytes().get(start + 2).copied();
        if !next.is_some_and(|c| c.is_ascii_whitespace()) {
            continue;
        }
        let tag = &html[start..tag_end];
        let Some(href) = attribute(tag, "href") else {
            continue;
        };
        let close = lower[at..].find("</a").map_or(lower.len(), |i| at + i);
        let label = sjel_extraction::decode_basic_entities(&strip_tags(&html[at..close]));
        links.push(Link {
            href: sjel_extraction::decode_basic_entities(&href),
            label: sjel_extraction::collapse_ws(&label),
        });
    }
    links
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name).map(|i| search + i) {
        search = found + name.len();
        let before = lower.as_bytes().get(found.wrapping_sub(1)).copied();
        if found > 0 && !before.is_some_and(|c| c.is_ascii_whitespace()) {
            continue;
        }
        let rest = tag[search..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next()? {
            quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
            _ => rest.split(|c: char| c.is_whitespace() || c == '>').next()?,
        };
        return Some(value.to_string());
    }
    None
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Links in plain text: `label<https://…>` as Outlook writes them, and bare URLs.
fn text_links(text: &str) -> Vec<Link> {
    let mut links = Vec::new();
    for (index, _) in text.match_indices("http") {
        let rest = &text[index..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '>' | '"' | ')' | ']'))
            .unwrap_or(rest.len());
        let href = rest[..end].to_string();
        let label = if text[..index].ends_with('<') {
            let before = &text[..index - 1];
            let start = before.rfind(|c: char| c.is_whitespace() || c == ']').map_or(0, |i| i + 1);
            before[start..].to_string()
        } else {
            String::new()
        };
        links.push(Link { href, label });
    }
    links
}

/// The sender line of the first quoted header block in a forward.
fn forwarded_sender(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter_map(|line| {
            ["von:", "from:", "de:", "da:", "van:"]
                .iter()
                .find_map(|key| {
                    line.get(..key.len())
                        .filter(|head| head.eq_ignore_ascii_case(key))
                        .map(|_| &line[key.len()..])
                })
        })
        .find_map(sender_domain)
}

/// Both rungs for one stored thread, fetched fresh from Gmail. The body is
/// read, assessed and dropped; only the findings are stored. `None` when the
/// thread is not in the store or no longer in Gmail.
pub fn audit(
    store: &crate::store::Store,
    token: &str,
    id: &str,
) -> Result<Option<Vec<Assessment>>, String> {
    if store.get_triage(id).map_err(|e| e.to_string())?.is_none() {
        return Ok(None);
    }
    let Some(meta) = crate::google::thread_meta_lookup(token, id).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let from = meta.from_addr.clone().unwrap_or_default();
    let subject = meta.subject.clone().unwrap_or_default();
    let known = store.known_sender_orgs(Some(id)).ok();
    let headers = assess_headers(
        &HeaderFacts {
            from: &from,
            subject: &subject,
            authentication_results: &meta.authentication_results,
            reply_to: meta.reply_to.as_deref(),
        },
        &known,
    );
    let authenticated = headers
        .findings
        .iter()
        .any(|f| matches!(f.code.as_str(), "dmarc_pass" | "dkim_aligned" | "spf_aligned"));
    let mut assessments = vec![headers];
    if let Some(parts) =
        crate::google::thread_message_parts(token, id).map_err(|e| e.to_string())?
    {
        assessments.push(assess_body(&BodyFacts {
            from: &from,
            subject: &subject,
            html: parts.html.as_deref(),
            text: parts.text.as_deref(),
            sender_authenticated: authenticated,
        }));
    }
    for assessment in &assessments {
        store
            .replace_triage_authenticity(id, assessment)
            .map_err(|e| e.to_string())?;
    }
    Ok(Some(assessments))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOUDFEST: &str = "mx.google.com; dkim=pass header.i=@cloudfest.com header.s=hs1 \
        header.b=abc; spf=pass (google.com: domain of bounce@bf.cloudfest.com designates \
        1.2.3.4 as permitted sender) smtp.mailfrom=bounce@bf.cloudfest.com; \
        dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=cloudfest.com";

    fn assess(from: &str, subject: &str, results: &[&str], reply_to: Option<&str>) -> Assessment {
        let results: Vec<String> = results.iter().map(|r| r.to_string()).collect();
        let known = Some(BTreeSet::from(["paypal.com".to_string(), "cloudfest.com".to_string()]));
        assess_headers(
            &HeaderFacts {
                from,
                subject,
                authentication_results: &results,
                reply_to,
            },
            &known,
        )
    }

    fn codes(assessment: &Assessment) -> Vec<&str> {
        assessment.findings.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn gmail_dmarc_pass_is_verified() {
        let a = assess("CloudFest Team <team@cloudfest.com>", "Last call", &[CLOUDFEST], None);
        assert_eq!(a.verdict, Verdict::Verified);
        assert_eq!(codes(&a), ["dmarc_pass"]);
    }

    #[test]
    fn a_result_the_sender_wrote_is_ignored() {
        let forged = "evil.example; dmarc=pass header.from=paypal.com";
        let a = assess("PayPal <service@paypal.com>", "Konto", &[forged], None);
        assert_eq!(a.verdict, Verdict::Unverified);
        assert_eq!(codes(&a)[0], "no_auth_result");
    }

    #[test]
    fn dmarc_fail_fails() {
        let header = "mx.google.com; spf=fail smtp.mailfrom=x@evil.test; dmarc=fail \
                      (p=REJECT) header.from=paypal.com";
        let a = assess("PayPal <service@paypal.com>", "Konto", &[header], None);
        assert_eq!(a.verdict, Verdict::Failed);
    }

    #[test]
    fn dkim_from_another_domain_does_not_verify() {
        let header = "mx.google.com; dkim=pass header.i=@sendgrid.net; spf=pass \
                      smtp.mailfrom=bounce@sendgrid.net";
        let a = assess("Shop <news@shop.example>", "Sale", &[header], None);
        assert_eq!(a.verdict, Verdict::Unverified);
        assert_eq!(codes(&a)[0], "not_aligned");
    }

    #[test]
    fn subdomain_dkim_aligns_with_the_organisation() {
        let header = "mx.google.com; dkim=pass header.i=@em.shop.example";
        let a = assess("Shop <news@shop.example>", "Sale", &[header], None);
        assert_eq!(codes(&a)[0], "dkim_aligned");
    }

    #[test]
    fn a_forward_is_never_verified() {
        let a = assess("Me <me@cloudfest.com>", "WG: Last call", &[CLOUDFEST], None);
        assert_eq!(a.verdict, Verdict::Unverified);
        assert!(codes(&a).contains(&"forwarded"));
    }

    #[test]
    fn display_name_naming_another_domain_is_suspicious() {
        let header = "mx.google.com; dmarc=pass header.from=evil.test";
        let a = assess("\"paypal.com\" <x@evil.test>", "Konto", &[header], None);
        assert_eq!(a.verdict, Verdict::Suspicious);
        assert!(codes(&a).contains(&"display_name_domain"));
        let header = "mx.google.com; dmarc=pass header.from=substack.com";
        let b = assess("Zach from DataExpert.io <x@substack.com>", "Post", &[header], None);
        assert_eq!(b.verdict, Verdict::Verified, "a newsletter platform sends for others");
    }

    #[test]
    fn freemail_reply_to_is_suspicious_and_other_reply_to_is_noted() {
        let a = assess("C <team@cloudfest.com>", "x", &[CLOUDFEST], Some("x@gmail.com"));
        assert_eq!(a.verdict, Verdict::Suspicious);
        let b = assess("C <team@cloudfest.com>", "x", &[CLOUDFEST], Some("x@whd.events"));
        assert_eq!(b.verdict, Verdict::Verified);
        assert!(codes(&b).contains(&"reply_to_mismatch"));
    }

    #[test]
    fn lookalike_and_new_sender() {
        let header = "mx.google.com; dmarc=pass header.from=paypa1.com";
        let a = assess("PayPal <service@paypa1.com>", "Konto", &[header], None);
        assert_eq!(a.verdict, Verdict::Suspicious);
        assert!(codes(&a).contains(&"lookalike_domain"));
        let header = "mx.google.com; dmarc=pass header.from=neo4j.com";
        let b = assess("Neo4j <x@neo4j.com>", "Workshop", &[header], None);
        assert_eq!(b.verdict, Verdict::Verified);
        assert!(codes(&b).contains(&"new_sender"));
    }

    #[test]
    fn org_domain_handles_multi_label_suffixes() {
        assert_eq!(org_domain("em.cloudfest.com"), "cloudfest.com");
        assert_eq!(org_domain("mail.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(org_domain("example.com"), "example.com");
    }

    #[test]
    fn comments_with_semicolons_do_not_split_methods() {
        let header = "mx.google.com; spf=pass (a; b) smtp.mailfrom=x@shop.example";
        let results = trusted_results(header).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].property("smtp.mailfrom"), Some("x@shop.example"));
    }

    const FORWARD_TEXT: &str = "Signature\n\nVon: CloudFest Team <team@cloudfest.com>\n\
        Gesendet: Dienstag\nAn: Me <me@work.example>\n\n\
        View in browser<https://em.cloudfest.com/e3t/Ctc/abc>\n\
        GET YOUR PASS NOW! » <https://em.cloudfest.com/e3t/Ctc/def>\n\
        Unsubscribe<https://hs-145724227.s.hubspotemail-eu1.net/hs/preferences-center/x>\n\
        www.employer.example<http://www.employer.example/>";

    fn body(from: &str, subject: &str, html: Option<&str>, text: Option<&str>) -> Assessment {
        assess_body(&BodyFacts {
            from,
            subject,
            html,
            text,
            sender_authenticated: false,
        })
    }

    #[test]
    fn a_forward_is_checked_against_its_original_sender() {
        let a = body("Me <me@work.example>", "WG: Last call", None, Some(FORWARD_TEXT));
        assert_eq!(a.verdict, Verdict::Unverified);
        assert_eq!(codes(&a), ["forwarded_sender", "link_summary"]);
        let summary = &a.findings[1].detail;
        assert!(summary.starts_with("2 link(s) to the sender's domain, 1 through a tracking"), "{summary}");
        assert!(summary.contains("employer.example"));
    }

    #[test]
    fn a_label_showing_another_domain_is_deceptive() {
        let html = r#"<p>Log in at <a class="b" href="https://paypal.login-check.test/x">www.paypal.com</a></p>"#;
        let a = body("PayPal <service@paypal.com>", "Konto", Some(html), None);
        assert_eq!(a.verdict, Verdict::Suspicious);
        assert!(codes(&a).contains(&"deceptive_link_text"));
    }

    #[test]
    fn an_authenticated_sender_showing_its_own_brand_is_not_deceptive() {
        let html = r#"<a href="https://go.brand/x">www.brand.de</a>"#;
        let facts = |sender_authenticated| BodyFacts {
            from: "Brand <news@welcome.brand.com>",
            subject: "Statement",
            html: Some(html),
            text: None,
            sender_authenticated,
        };
        assert_eq!(assess_body(&facts(true)).verdict, Verdict::Verified);
        assert_eq!(assess_body(&facts(false)).verdict, Verdict::Suspicious);
        let other = r#"<a href="https://redirect.example/x">www.paypal.com</a>"#;
        let compromised = BodyFacts {
            html: Some(other),
            ..facts(true)
        };
        assert_eq!(assess_body(&compromised).verdict, Verdict::Unverified);
        let nothing = BodyFacts {
            html: Some("<p>Plain note</p>"),
            ..facts(false)
        };
        assert_eq!(assess_body(&nothing).verdict, Verdict::Verified);
    }

    #[test]
    fn a_tracked_link_with_a_domain_label_is_not_deceptive() {
        let html = r#"<a href="https://click.sendgrid.net/ls/abc">shop.example</a>"#;
        let a = body("Shop <news@shop.example>", "Sale", Some(html), None);
        assert_eq!(a.verdict, Verdict::Verified);
    }

    #[test]
    fn script_ip_and_form_are_flagged() {
        let html = r#"<a href="javascript:alert(1)">x</a><a href='http://192.0.2.7/login'>Login</a>
                      <form action="https://evil.test"><input name=p></form>"#;
        let a = body("Bank <x@bank.example>", "Konto", Some(html), None);
        assert_eq!(a.verdict, Verdict::Failed);
        let codes = codes(&a);
        for code in ["script_link", "ip_link", "form_in_mail"] {
            assert!(codes.contains(&code), "{code} missing from {codes:?}");
        }
    }

    #[test]
    fn anchors_read_entities_and_unquoted_attributes() {
        let links = anchors("<A HREF=https://a.example/?x=1&amp;y=2 target=_blank>Hi <b>there</b></A><abbr>no</abbr>");
        assert_eq!(
            links,
            [Link {
                href: "https://a.example/?x=1&y=2".into(),
                label: "Hi there".into()
            }]
        );
    }
}
