//! The arXiv adapter, and the only one with a stand-in.
//!
//! Three rungs, tried in order -- the ar5iv HTML, arXiv's own HTML, then the PDF --
//! because arXiv is the one source that can still answer with an abstract when the
//! document itself cannot be read.

use super::*;

/// arXiv identifier from an abs/ or pdf/ URL, version suffix kept (it is part of
/// the identity — v1 and v2 are different papers to a reader). Handles both the
/// modern `2501.12345` form and the legacy `cs/0112017` archive form.
pub(super) fn arxiv_id(url: &str) -> Option<String> {
    if !host_of(url).ends_with("arxiv.org") {
        return None;
    }
    let seg = path_segments(url);
    let rest = match seg.split_first() {
        Some((first, rest)) if *first == "abs" || *first == "pdf" => rest,
        _ => return None,
    };
    if rest.is_empty() {
        return None;
    }
    let id = rest.join("/");
    let id = id.strip_suffix(".pdf").unwrap_or(&id);
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// First `<tag>…</tag>` payload inside `body`, entity-decoded and whitespace-
/// collapsed. Enough for arXiv's Atom, which is machine-generated and flat — a
/// real XML parser would be a dependency bought for one endpoint.
pub(super) fn xml_field(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    let text = collapse_ws(&decode_basic_entities(body[start..end].trim()));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// An arXiv paper via the Atom export API: title, authors and abstract.
///
/// The abstract is all of it for now, which is what #78 is about. The PDF branch
/// that used to sit here called the `xberg` CLI, which is not installed, so it
/// never once ran — an inert branch that made the capability read as if it
/// pulled full text. It comes back when there is a PDF extractor that actually
/// works (#77), and not before.
pub(super) fn fetch_arxiv(
    id: &str,
) -> Result<(Option<String>, Option<String>, String, TranscriptSource)> {
    let http = http_client()?;
    let resp = http
        .get(format!(
            "https://export.arxiv.org/api/query?id_list={id}&max_results=1"
        ))
        .send()?;
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "arXiv API HTTP {}",
            resp.status()
        )));
    }
    let body = resp.text()?;
    // The feed carries its own <title>/<id> before the first <entry>; slice past
    // it so the fields below can't come from the envelope.
    let entry = body
        .find("<entry>")
        .map(|i| &body[i..])
        .ok_or_else(|| CommsError::Other(format!("arXiv: no entry for {id}")))?;

    let title = xml_field(entry, "title");
    let abstract_text = xml_field(entry, "summary").unwrap_or_default();

    let mut authors: Vec<String> = Vec::new();
    let mut rest = entry;
    while let Some(i) = rest.find("<author>") {
        rest = &rest[i + "<author>".len()..];
        if let Some(name) = xml_field(rest, "name") {
            authors.push(name);
        }
    }
    let author = match authors.len() {
        0 => None,
        1..=3 => Some(authors.join(", ")),
        _ => Some(format!("{} et al.", authors[..3].join(", "))),
    };

    // The paper itself, when it can be read, with the abstract as the
    // fallback and a record of which was used (#78).
    let (text, source) = match arxiv_full_text(&http, id) {
        Some(full) => (full, TranscriptSource::FullText),
        None => (abstract_text, TranscriptSource::Abstract),
    };

    Ok((title, author, cap_text(text), source))
}

/// The paper's own text, or `None` when neither route can read it.
///
/// **HTML first, and not as a workaround.** arXiv renders LaTeX submissions to
/// HTML at `/html/<id>` — including backfilled classics — and that beats a PDF
/// for this purpose on its own merits: LaTeXML keeps document structure, while
/// PDF text extraction has to reconstruct reading order out of a two-column
/// layout and routinely mangles maths and ligatures doing it. A paper without
/// LaTeX source answers 404 there, which is a clean signal rather than a
/// judgement call about a bad conversion.
///
/// **ar5iv second, which is most of what a PDF reader was going to be for.**
/// Measured 2026-08-04 over 24 newest cs.AI/cs.LG/cs.CL papers: arxiv.org
/// served HTML for 21. ar5iv answered all three misses, plus a 2007 paper
/// arxiv.org has no HTML for, with real body text rather than a stub. It is
/// the older LaTeXML pipeline with far wider backfill, which is exactly the
/// shape of the remaining gap, so it is tried only on a 404 and never in
/// preference to the canonical host.
///
/// PDF is the third attempt, for papers with no LaTeX source at all: scans
/// and PDF-only submissions, mostly old. It is unreachable today because
/// nothing is registered for the class, and starts working when xberg lands
/// (#77) without this function changing.
///
/// Every `None` here is ordinary rather than an error to report: no HTML
/// anywhere, no PDF reader, or a fetch that failed on this one paper. The
/// caller has an abstract and records that it used it. No raw PDF is
/// persisted.
pub(super) fn arxiv_full_text(http: &reqwest::blocking::Client, id: &str) -> Option<String> {
    arxiv_html(http, ARXIV_HTML_HOSTS[0], id)
        .or_else(|| arxiv_html(http, ARXIV_HTML_HOSTS[1], id))
        .or_else(|| arxiv_pdf(http, id))
}

/// Canonical host first, wider-backfill mirror second. Order is the policy:
/// where both have a paper, arxiv.org's is the newer conversion.
pub(super) const ARXIV_HTML_HOSTS: [&str; 2] =
    ["https://arxiv.org", "https://ar5iv.labs.arxiv.org"];

pub(super) fn arxiv_html(http: &reqwest::blocking::Client, host: &str, id: &str) -> Option<String> {
    let extractor = extraction::for_class(InputClass::Html)?;
    let resp = http.get(format!("{host}/html/{id}")).send().ok()?;
    if !resp.status().is_success() {
        return None; // 404: no LaTeX-derived HTML for this paper on this host.
    }
    let body = resp.text().ok()?;
    let out = extractor.extract(&Document::html(body.as_bytes())).ok()?;
    Some(out.text).filter(|t| !t.trim().is_empty())
}

pub(super) fn arxiv_pdf(http: &reqwest::blocking::Client, id: &str) -> Option<String> {
    let extractor = extraction::for_class(InputClass::Pdf)?;
    let resp = http
        .get(format!("https://arxiv.org/pdf/{id}"))
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    let out = extractor.extract(&Document::pdf(&bytes)).ok()?;
    Some(out.text).filter(|t| !t.trim().is_empty())
}
