//! Validation and materialization for reviewed PDF partitions.
//!
//! A split assigns each page of a parent to exactly one child range. The document
//! library stores the resulting PDFs as independent content-addressed blobs.

/// Write a page-range PDF containing only the requested source pages.
///
/// # Errors
/// The source PDF is malformed or encrypted, the source page count changed, the
/// output exceeds the library limit, or the selected pages cannot be serialized.
pub(crate) fn materialize_pdf_range(
    source: &[u8],
    range: PageRange,
    expected_source_page_count: u32,
) -> Result<Vec<u8>, String> {
    let mut document = lopdf::Document::load_mem(source)
        .map_err(|error| format!("could not read source PDF: {error}"))?;
    if document.is_encrypted() {
        return Err("encrypted PDFs cannot be split without a password".into());
    }

    let pages = document.get_pages();
    let source_page_count = u32::try_from(pages.len())
        .map_err(|_| "source PDF has too many pages to split".to_owned())?;
    if source_page_count != expected_source_page_count {
        return Err(format!(
            "PDF page count changed: extraction recorded {expected_source_page_count}, source has {source_page_count}"
        ));
    }
    if range.last > source_page_count {
        return Err("page range exceeds the source PDF page count".into());
    }

    let pages_to_remove: Vec<u32> = pages
        .keys()
        .copied()
        .filter(|page| *page < range.first || *page > range.last)
        .collect();
    document.delete_pages(&pages_to_remove);
    document.prune_objects();
    document.renumber_objects();

    let expected_count = usize::try_from(range.last - range.first + 1)
        .map_err(|_| "page range exceeds the platform page-count limit".to_owned())?;
    if document.get_pages().len() != expected_count {
        return Err("PDF page selection did not produce the requested page count".into());
    }

    let mut output = Vec::new();
    document
        .save_to(&mut output)
        .map_err(|error| format!("could not write child PDF: {error}"))?;
    if output.is_empty() {
        return Err("PDF writer produced an empty child".into());
    }
    if output.len() > crate::library::MAX_BYTES {
        return Err("child PDF exceeds the library's 128 MiB file limit".into());
    }
    Ok(output)
}

pub use documents_api::{
    BoundaryEvidence, ChildInput, PageRange, Partition, PartitionError, SplitRequest,
    SplitSuggestion,
};

/// Propose complete ranges from local page signals. Every boundary carries the signal that
/// produced it and a quote, and the operator accepts or edits each cut before a split.
pub fn suggest_ranges(pages: &[sjel_extraction::ExtractedPage]) -> SplitSuggestion {
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let boundaries = pages
        .windows(2)
        .filter_map(|pair| boundary_before(&pair[0], &pair[1]))
        .collect::<Vec<_>>();
    if boundaries.is_empty()
        || pages
            .iter()
            .enumerate()
            .any(|(index, page)| page.page_number != u32::try_from(index + 1).unwrap_or(u32::MAX))
    {
        return SplitSuggestion {
            page_count,
            ranges: Vec::new(),
            boundaries,
            abstention: Some("No page signal marks where a new document starts.".into()),
        };
    }

    let mut first = 1;
    let mut ranges = Vec::with_capacity(boundaries.len() + 1);
    for boundary in &boundaries {
        ranges.push(PageRange {
            first,
            last: boundary.before_page - 1,
        });
        first = boundary.before_page;
    }
    ranges.push(PageRange {
        first,
        last: page_count,
    });
    SplitSuggestion {
        page_count,
        ranges,
        boundaries,
        abstention: None,
    }
}

/// Fraction of shared words below which two full pages read as different documents.
/// My estimate, not measured.
const VOCABULARY_SHIFT_MAX_JACCARD: f64 = 0.08;
/// Pages with fewer words than this say too little to compare.
const MIN_COMPARABLE_WORDS: usize = 30;
/// Only the top of a page can open a document.
const HEAD_LINES: usize = 15;

/// The strongest signal that `page` starts a new document after `previous`.
fn boundary_before(
    previous: &sjel_extraction::ExtractedPage,
    page: &sjel_extraction::ExtractedPage,
) -> Option<BoundaryEvidence> {
    let evidence = |marker: &str, quote: String| BoundaryEvidence {
        before_page: page.page_number,
        marker: marker.to_owned(),
        quote,
    };
    if let Some((marker, quote)) = explicit_page_one_marker(&page.text) {
        return Some(evidence(marker, quote));
    }
    if let Some(quote) = last_page_marker(&previous.text) {
        return Some(evidence("last-page-marker", quote));
    }
    if let Some(quote) = document_opening(&page.text) {
        return Some(evidence("document-opening", quote));
    }
    let date = head(&page.text).find(|line| has_date(line))?;
    let similarity = word_jaccard(&previous.text, &page.text)?;
    (similarity < VOCABULARY_SHIFT_MAX_JACCARD)
        .then(|| evidence("vocabulary-shift-with-date", quote(date)))
}

fn head(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(HEAD_LINES)
}

fn quote(line: &str) -> String {
    line.trim().chars().take(160).collect()
}

/// "Seite 3 von 3" or "Page 2 of 2" on the previous page: that document ended there.
fn last_page_marker(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(HEAD_LINES)
        .find_map(|line| {
            let folded = line.to_lowercase();
            ["seite ", "page "].iter().find_map(|word| {
                let rest = &folded[folded.find(word)? + word.len()..];
                let mut numbers = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|part| !part.is_empty())
                    .take(2)
                    .map(str::parse::<u32>);
                let (Some(Ok(current)), Some(Ok(total))) = (numbers.next(), numbers.next()) else {
                    return None;
                };
                let joined = rest.contains(" von ") || rest.contains(" of ") || rest.contains('/');
                (joined && total > 1 && current == total).then(|| quote(line))
            })
        })
}

/// A salutation, subject line or document title at the top of the page.
fn document_opening(text: &str) -> Option<String> {
    const OPENINGS: &[&str] = &[
        "sehr geehrte",
        "sehr geehrter",
        "liebe ",
        "lieber ",
        "dear ",
        "betreff:",
        "subject:",
        "rechnung nr",
        "rechnungsnummer",
        "invoice no",
        "invoice number",
        "kontoauszug",
        "bescheid für",
        "bescheid über",
    ];
    head(text).find_map(|line| {
        let folded = line.to_lowercase();
        OPENINGS
            .iter()
            .any(|opening| folded.starts_with(opening))
            .then(|| quote(line))
    })
}

/// A `dd.mm.yyyy` or `yyyy-mm-dd` date somewhere in the line.
fn has_date(line: &str) -> bool {
    let bytes = line.as_bytes();
    let digits = |from: usize, count: usize| {
        bytes
            .get(from..from + count)
            .is_some_and(|run| run.iter().all(u8::is_ascii_digit))
    };
    (0..bytes.len()).any(|i| {
        (digits(i, 2)
            && bytes.get(i + 2) == Some(&b'.')
            && digits(i + 3, 2)
            && bytes.get(i + 5) == Some(&b'.')
            && digits(i + 6, 4))
            || (digits(i, 4)
                && bytes.get(i + 4) == Some(&b'-')
                && digits(i + 5, 2)
                && bytes.get(i + 7) == Some(&b'-')
                && digits(i + 8, 2))
    })
}

/// Shared-word fraction of two pages, or `None` when either says too little.
fn word_jaccard(left: &str, right: &str) -> Option<f64> {
    let words = |text: &str| {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 3)
            .map(str::to_owned)
            .collect::<std::collections::HashSet<_>>()
    };
    let (left, right) = (words(left), words(right));
    if left.len() < MIN_COMPARABLE_WORDS || right.len() < MIN_COMPARABLE_WORDS {
        return None;
    }
    let shared = left.intersection(&right).count();
    Some(shared as f64 / (left.len() + right.len() - shared) as f64)
}

fn explicit_page_one_marker(text: &str) -> Option<(&'static str, String)> {
    for line in text.lines().take(12) {
        let folded = line.to_lowercase();
        for marker in ["page 1 of", "page 1/", "seite 1 von", "seite 1/"] {
            if folded.contains(marker) {
                let quote = line.trim().chars().take(160).collect::<String>();
                if !quote.is_empty() {
                    return Some((marker, quote));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(first: u32, last: u32) -> PageRange {
        PageRange::new(first, last).unwrap()
    }

    fn page(page_number: u32, text: &str) -> sjel_extraction::ExtractedPage {
        sjel_extraction::ExtractedPage {
            page_number,
            text: text.into(),
            engine: "test".into(),
            failure: None,
        }
    }

    fn filler(topic: &str) -> String {
        (0..40).map(|i| format!("{topic}{i} ")).collect()
    }

    #[test]
    fn splits_at_last_page_markers_and_document_openings() {
        let pages = [
            page(
                1,
                &format!("Sehr geehrte Damen und Herren\n{}", filler("alpha")),
            ),
            page(2, &format!("{}\nSeite 2 von 2", filler("alpha"))),
            page(3, &format!("Rechnung Nr. 4711\n{}", filler("beta"))),
            page(
                4,
                &format!("Sehr geehrter Herr Muster\n{}", filler("gamma")),
            ),
        ];
        let suggestion = suggest_ranges(&pages);
        assert_eq!(suggestion.ranges, [range(1, 2), range(3, 3), range(4, 4)]);
        let markers: Vec<_> = suggestion
            .boundaries
            .iter()
            .map(|b| b.marker.as_str())
            .collect();
        assert_eq!(markers, ["last-page-marker", "document-opening"]);
    }

    #[test]
    fn a_vocabulary_shift_needs_a_date_at_the_top() {
        let without_date = [page(1, &filler("alpha")), page(2, &filler("beta"))];
        assert!(suggest_ranges(&without_date).ranges.is_empty());
        let with_date = [
            page(1, &filler("alpha")),
            page(2, &format!("Berlin, 03.04.2025\n{}", filler("beta"))),
        ];
        let suggestion = suggest_ranges(&with_date);
        assert_eq!(suggestion.ranges, [range(1, 1), range(2, 2)]);
        assert_eq!(
            suggestion.boundaries[0].marker,
            "vocabulary-shift-with-date"
        );
    }

    #[test]
    fn continuing_pages_of_one_letter_stay_together() {
        let pages = [
            page(1, &format!("Sehr geehrte Frau Muster\n{}", filler("alpha"))),
            page(
                2,
                &format!("03.04.2025\n{}\nSeite 2 von 3", filler("alpha")),
            ),
            page(3, &format!("{}\nSeite 3 von 3", filler("alpha"))),
        ];
        let suggestion = suggest_ranges(&pages);
        assert!(suggestion.ranges.is_empty());
        assert!(suggestion.abstention.is_some());
    }

    #[test]
    fn accepts_a_complete_partition() {
        let partition = Partition::new(6, vec![range(1, 2), range(3, 3), range(4, 6)]).unwrap();
        assert_eq!(partition.page_count(), 6);
        assert_eq!(partition.ranges(), [range(1, 2), range(3, 3), range(4, 6)]);
    }

    #[test]
    fn rejects_empty_parent_or_empty_split() {
        assert_eq!(
            Partition::new(0, vec![range(1, 1)]),
            Err(PartitionError::EmptyParent)
        );
        assert_eq!(Partition::new(3, vec![]), Err(PartitionError::NoRanges));
    }

    #[test]
    fn rejects_invalid_and_out_of_bounds_ranges() {
        assert_eq!(PageRange::new(0, 1), Err(PartitionError::InvalidRange));
        assert_eq!(PageRange::new(3, 2), Err(PartitionError::InvalidRange));
        assert_eq!(
            Partition::new(2, vec![range(1, 3)]),
            Err(PartitionError::OutOfBounds)
        );
    }

    #[test]
    fn rejects_gaps_overlaps_and_out_of_order_ranges() {
        assert_eq!(
            Partition::new(4, vec![range(1, 1), range(3, 4)]),
            Err(PartitionError::NotExactPartition)
        );
        assert_eq!(
            Partition::new(4, vec![range(1, 3), range(3, 4)]),
            Err(PartitionError::NotExactPartition)
        );
        assert_eq!(
            Partition::new(4, vec![range(3, 4), range(1, 2)]),
            Err(PartitionError::NotExactPartition)
        );
    }

    #[test]
    fn rejects_trailing_unassigned_pages() {
        assert_eq!(
            Partition::new(5, vec![range(1, 4)]),
            Err(PartitionError::NotExactPartition)
        );
    }

    #[test]
    fn rejects_overlap_at_the_largest_page_number() {
        assert_eq!(
            Partition::new(
                u32::MAX,
                vec![range(1, u32::MAX), range(u32::MAX, u32::MAX)]
            ),
            Err(PartitionError::NotExactPartition)
        );
    }
}
