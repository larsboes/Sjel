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

/// Propose complete ranges only when OCR or a text layer exposes explicit page-one markers.
pub fn suggest_ranges(pages: &[sjel_extraction::ExtractedPage]) -> SplitSuggestion {
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let boundaries = pages
        .iter()
        .filter(|page| page.page_number > 1)
        .filter_map(|page| {
            explicit_page_one_marker(&page.text).map(|(marker, quote)| BoundaryEvidence {
                before_page: page.page_number,
                marker: marker.to_owned(),
                quote,
            })
        })
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
            abstention: Some(
                "No complete partition is supported by explicit page-one markers.".into(),
            ),
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
