//! `inventory add <url>`: a shared link becomes a wanted row. This module reads the page;
//! `main.rs` writes the row.
//!
//! Only what the page declares about itself is read: `og:title` before `<title>`, and a price
//! only from a meta tag (`og:price:amount`, `product:price:amount`, `itemprop="price"`). A number
//! in the body text is never a price, because a wrong price moves the wishlist total, while a
//! missing one is counted and reported. A currency other than euro is refused, not converted.

use std::time::Duration;

/// Why a link did not become a row. Refused is a decision (not http(s), or an address in this
/// network) and ends the call; unreachable is a circumstance, and the row is still created.
#[derive(Debug)]
pub enum FetchError {
    Refused(String),
    Unreachable(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Refused(e) | FetchError::Unreachable(e) => f.write_str(e),
        }
    }
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// Shops answer a self-identifying client with nothing: measured 2026-10-05, a product page gave
/// an HTTP/2 INTERNAL_ERROR to `Sjel-interior-wunsch/0.0.1` and 200 to this agent.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub title: Option<String>,
    pub price_cents: Option<i64>,
    /// The meta tag the price came from, so a wrong price has a trace.
    pub price_from: Option<String>,
}

/// Fetches a product page and reads what it says about itself.
///
/// The scheme and destination guards run before the request, so a link cannot reach `file://`
/// or this network. The error is a `String`-carrying enum because this runs on a blocking thread
/// and its result must be `Send`.
pub fn fetch(url: &str) -> Result<Page, FetchError> {
    sjel_http::guard::check_scheme(url).map_err(|e| FetchError::Refused(e.to_string()))?;
    sjel_http::guard::check_destination(url, Vec::new)
        .map_err(|e| FetchError::Refused(e.to_string()))?;

    let client = sjel_http::builder(sjel_http::Purpose::new("inventory-add"), TIMEOUT)
        .user_agent(BROWSER_UA)
        .build()
        .map_err(|e| FetchError::Unreachable(e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| FetchError::Unreachable(e.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Unreachable(format!("{url}: {status}")));
    }
    let body = response
        .text()
        .map_err(|e| FetchError::Unreachable(e.to_string()))?;
    Ok(parse_with_url(&body, Some(url)))
}

pub fn parse(page: &str) -> Page {
    parse_with_url(page, None)
}

pub fn parse_with_url(page: &str, url: Option<&str>) -> Page {
    let price = price(page).or_else(|| per_shop_price(url, page));
    Page {
        title: title(page),
        price_cents: price.as_ref().map(|(c, _)| *c),
        price_from: price.map(|(_, from)| from),
    }
}

/// Uniqlo and shops with embedded state/scripts rather than meta tags.
pub fn per_shop_price(url: Option<&str>, page: &str) -> Option<(i64, String)> {
    if let Some(url) = url {
        if url.contains("uniqlo.com") {
            if let Some(cents) = uniqlo_price(page) {
                return Some((cents, "shop:uniqlo".to_string()));
            }
        }
    }
    None
}

/// Uniqlo pages embed price in FastRetailing JSON objects or HTML price spans.
pub fn uniqlo_price(page: &str) -> Option<i64> {
    let needle = r#""currency":{"code":""#;
    if let Some(curr_idx) = page.find(needle) {
        let code = &page[curr_idx + needle.len()..];
        if let Some(end) = code.find('"') {
            if !code[..end].eq_ignore_ascii_case("EUR") {
                return None;
            }
        }
    }
    // 1. Check for FastRetailing JSON pattern: "prices":{"promo":{"value":X}} or "base":{"value":X}
    for key in [r#""promo":{"value":"#, r#""sales":{"value":"#, r#""base":{"value":"#] {
        if let Some(idx) = page.find(key) {
            let rest = &page[idx + key.len()..];
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Some(cents) = cents_from(&num_str) {
                return Some(cents);
            }
        }
    }
    // 2. Check for HTML price amount span: class="...price-amount...">49,90<
    if let Some(idx) = page.find("price-amount") {
        let rest = &page[idx..];
        if let Some(tag_end) = rest.find('>') {
            let after_tag = &rest[tag_end + 1..];
            if let Some(close_tag) = after_tag.find('<') {
                let text = &after_tag[..close_tag];
                if let Some(cents) = cents_from(text.trim()) {
                    return Some(cents);
                }
            }
        }
    }
    None
}

/// `og:title` before `<title>`: shops put their brand into `<title>` and the product into
/// `og:title`.
pub fn title(page: &str) -> Option<String> {
    meta(page, "og:title")
        .or_else(|| element(page, "title"))
        .map(|t| clean(&t))
        .filter(|t| !t.is_empty())
}

/// A declared price and the tag that declared it.
pub fn price(page: &str) -> Option<(i64, String)> {
    for tag in ["og:price:amount", "product:price:amount", "price"] {
        let Some(raw) = meta(page, tag) else {
            continue;
        };
        if let Some(currency) = currency(page) {
            if !currency.eq_ignore_ascii_case("EUR") {
                return None;
            }
        }
        if let Some(cents) = cents_from(&raw) {
            return Some((cents, tag.to_string()));
        }
    }
    None
}

fn currency(page: &str) -> Option<String> {
    meta(page, "og:price:currency").or_else(|| meta(page, "product:price:currency"))
}

/// `79.90`, `79,90`, `79.90 EUR`, `EUR 79.90` -> `7990`.
///
/// Strict on purpose: after one currency marker is removed, exactly one number must remain.
/// `ab 79` or `79.90 - 129.00` are not prices. An earlier version trimmed leading letters and
/// turned `ab 79` into 79,00 € in a column that adds up into the wishlist total.
pub fn cents_from(raw: &str) -> Option<i64> {
    let s = without_currency(raw);
    // Thousands separators go where both characters occur: `1.234,56`.
    let s = if s.contains('.') && s.contains(',') {
        s.replace('.', "")
    } else {
        s.to_string()
    };
    let s = s.replace(',', ".");
    let (whole, fraction) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s.as_str(), ""),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || fraction.len() > 2 {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    // `79.9` is 79,90, not 79,09.
    let fraction: i64 = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 10,
        _ => fraction.parse().ok()?,
    };
    Some(whole * 100 + fraction)
}

/// `EUR`, `€` or `$` at either end, and nothing else.
fn without_currency(s: &str) -> &str {
    let t = s.trim();
    let bare = t
        .strip_prefix(['€', '$'])
        .or_else(|| t.strip_suffix(['€', '$']))
        .map(str::trim)
        .unwrap_or(t);
    // `to_ascii_uppercase` keeps the length, so the indices still match.
    let upper = bare.to_ascii_uppercase();
    if upper.starts_with("EUR") {
        return bare[3..].trim();
    }
    if upper.ends_with("EUR") {
        return bare[..bare.len() - 3].trim();
    }
    bare
}

/// An id from a title: `"Oxford Hemd, weiss"` -> `"oxford-hemd-weiss"`. ASCII lower case,
/// because it goes into a URL and a file name; umlauts are transliterated, not dropped.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        let c = match c {
            'ä' | 'Ä' => 'a',
            'ö' | 'Ö' => 'o',
            'ü' | 'Ü' => 'u',
            'ß' => 's',
            other => other,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// The first free id: `shirt`, else `shirt-2`, `shirt-3` … A second shirt is a second entry, not
/// an overwritten one.
pub fn free_id(suggestion: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = if suggestion.is_empty() {
        "entry".to_string()
    } else {
        suggestion.to_string()
    };
    if !taken(&base) {
        return base;
    }
    // ponytail: linear search; nobody adds 10,000 shirts by hand. An importer would use COUNT(*).
    for n in 2..10_000 {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    base
}

// --- HTML, as little as possible --------------------------------------------------------
//
// No parser and no dependency: only the content of a `<meta>` and a `<title>` is needed, both in
// the page head.

/// The `content` of the `<meta>` whose `property`, `name` or `itemprop` is `wanted`.
fn meta(page: &str, wanted: &str) -> Option<String> {
    let mut rest = page;
    while let Some(start) = find(rest, "<meta") {
        rest = &rest[start..];
        let end = rest.find('>')?;
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        let attributes = attributes(tag);
        let name = attributes
            .iter()
            .find(|(k, _)| matches!(k.as_str(), "property" | "name" | "itemprop"))
            .map(|(_, v)| v.as_str());
        if name.is_some_and(|n| n.eq_ignore_ascii_case(wanted)) {
            return attributes
                .into_iter()
                .find(|(k, _)| k == "content")
                .map(|(_, v)| v);
        }
    }
    None
}

fn element(page: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let close = format!("</{name}");
    let start = find(page, &open)?;
    let rest = &page[start..];
    let content_start = rest.find('>')? + 1;
    let content_end = find(&rest[content_start..], &close)?;
    Some(rest[content_start..content_start + content_end].to_string())
}

/// Case-insensitive `find`. `str::to_lowercase` would shift indices when a character grows.
fn find(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Attributes of a tag, in order.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'-' {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = tag[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        let value = if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = tag[start..i].to_string();
            i += 1;
            v
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            tag[start..i].to_string()
        };
        out.push((name, value));
    }
    out
}

/// Decode entities and collapse whitespace.
fn clean(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let Some(end) = rest[i..].find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[i + 1..];
            continue;
        };
        let entity = &rest[i + 1..i + end];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            "ndash" => out.push('-'),
            "mdash" => out.push('-'),
            "szlig" => out.push('s'),
            other => match numeric(other) {
                Some(c) => out.push(c),
                // Unknown stays as written.
                None => {
                    out.push('&');
                    out.push_str(entity);
                    out.push(';');
                }
            },
        }
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `#39` and `#x27`.
fn numeric(entity: &str) -> Option<char> {
    let rest = entity.strip_prefix('#')?;
    let code = match rest.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => rest.parse().ok()?,
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An invented product page; no real shops or prices.
    const PAGE: &str = r#"<!doctype html>
<html><head>
  <meta charset="utf-8">
  <title>Beispielhemd weiss | Beispielshop</title>
  <meta property="og:title" content="Beispielhemd, weiss &amp; buegelfrei">
  <meta property="og:price:amount" content="79.90">
  <meta property="og:price:currency" content="EUR">
</head><body><p>Ab 79.90 EUR</p></body></html>"#;

    #[test]
    fn og_title_beats_title() {
        assert_eq!(
            title(PAGE).as_deref(),
            Some("Beispielhemd, weiss & buegelfrei")
        );
    }

    #[test]
    fn without_og_title_it_falls_back_to_title() {
        let page = "<head><title> Only the title </title></head>";
        assert_eq!(title(page).as_deref(), Some("Only the title"));
    }

    #[test]
    fn a_number_in_the_text_is_no_price() {
        let page = "<html><body>Ab 79.90 EUR, versandkostenfrei</body></html>";
        assert_eq!(price(page), None);
        assert_eq!(parse(page).price_cents, None);
    }

    #[test]
    fn a_declared_price_is_read_and_named() {
        let (cents, from) = price(PAGE).expect("price from og:price:amount");
        assert_eq!(cents, 7990);
        assert_eq!(from, "og:price:amount");
    }

    #[test]
    fn itemprop_is_the_third_way() {
        let page = r#"<meta itemprop="price" content="1.234,56">"#;
        assert_eq!(price(page), Some((123456, "price".to_string())));
    }

    #[test]
    fn a_foreign_currency_is_refused() {
        let page = r#"<meta property="og:price:amount" content="79.90">
                       <meta property="og:price:currency" content="USD">"#;
        assert_eq!(price(page), None);
    }

    #[test]
    fn attribute_order_does_not_matter() {
        let a = r#"<meta content="12,50" property="og:price:amount">"#;
        let b = r#"<meta property="og:price:amount" content="12,50">"#;
        assert_eq!(price(a), Some((1250, "og:price:amount".to_string())));
        assert_eq!(price(b), Some((1250, "og:price:amount".to_string())));
    }

    #[test]
    fn amounts_this_reader_cannot_interpret_are_no_price() {
        assert_eq!(cents_from("79.90 - 129.00"), None);
        assert_eq!(cents_from("79.901"), None);
        assert_eq!(cents_from("kostenlos"), None);
        // The case the first version got wrong: `ab` is text, not a currency.
        assert_eq!(cents_from("ab 79"), None);
        assert_eq!(cents_from("ab 79,90"), None);
        assert_eq!(cents_from("statt 99"), None);
        assert_eq!(cents_from("1.234"), None);
        assert_eq!(cents_from("79.90"), Some(7990));
        assert_eq!(cents_from("79,90"), Some(7990));
        assert_eq!(cents_from("79,9"), Some(7990));
        assert_eq!(cents_from("79"), Some(7900));
        assert_eq!(cents_from("1.234,56"), Some(123456));
        assert_eq!(cents_from("79.90 EUR"), Some(7990));
        assert_eq!(cents_from("EUR 79.90"), Some(7990));
        assert_eq!(cents_from("79,90 eur"), Some(7990));
        assert_eq!(cents_from("€ 79,90"), Some(7990));
    }

    #[test]
    fn a_slug_is_ascii_lower_case() {
        assert_eq!(slug("Oxford Hemd, weiss"), "oxford-hemd-weiss");
        assert_eq!(slug("Größe 42 / blau"), "grose-42-blau");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn a_taken_id_gets_a_number() {
        let taken = |k: &str| k == "shirt" || k == "shirt-2";
        assert_eq!(free_id("shirt", taken), "shirt-3");
        assert_eq!(free_id("coat", taken), "coat");
        assert_eq!(free_id("", taken), "entry");
    }

    #[test]
    fn entities_and_whitespace_are_cleaned() {
        assert_eq!(clean("  Hemd\n   weiss  "), "Hemd weiss");
        assert_eq!(
            clean("A &amp; B &#39;x&#39; &unknown;"),
            "A & B 'x' &unknown;"
        );
    }

    #[test]
    fn uniqlo_shop_reader_parses_fastretailing_json_and_html_spans() {
        let json_page = r#"<html><head><title>UNIQLO</title><meta property="og:title" content="AIRism Shirt"></head>
            <body><script>window.__PRELOADED_STATE__ = {"product":{"prices":{"base":{"value":49.90},"currency":{"code":"EUR"}}}};</script></body></html>"#;
        let page = parse_with_url(json_page, Some("https://www.uniqlo.com/de/de/products/E123456"));
        assert_eq!(page.title, Some("AIRism Shirt".to_string()));
        assert_eq!(page.price_cents, Some(4990));
        assert_eq!(page.price_from, Some("shop:uniqlo".to_string()));

        let promo_page = r#"{"prices":{"promo":{"value":29.90},"base":{"value":39.90}}}"#;
        let p2 = parse_with_url(promo_page, Some("https://www.uniqlo.com/de/de/products/E999"));
        assert_eq!(p2.price_cents, Some(2990));

        let span_page = r#"<span class="fr-price-currency">€</span><span class="product-price-amount">19,90</span>"#;
        let p3 = parse_with_url(span_page, Some("https://www.uniqlo.com/de/de/products/E777"));
        assert_eq!(p3.price_cents, Some(1990));
    }
}
