//! Validation and materialization for reviewed PDF partitions.
//!
//! A split assigns each page of a parent to exactly one child range. The document
//! library stores the resulting PDFs as independent content-addressed blobs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

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

/// An inclusive, one-based page range for one child document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRange {
    /// First page assigned to this child.
    pub first: u32,
    /// Last page assigned to this child.
    pub last: u32,
}

/// One explicit page-number reset that supports a proposed cut before a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BoundaryEvidence {
    pub before_page: u32,
    pub marker: String,
    pub quote: String,
}

/// A review-only partition proposal derived from explicit page-number markers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SplitSuggestion {
    pub page_count: u32,
    pub ranges: Vec<PageRange>,
    pub boundaries: Vec<BoundaryEvidence>,
    pub abstention: Option<String>,
}

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

/// A complete partition of a parent PDF into ordered child page ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Partition {
    page_count: u32,
    ranges: Vec<PageRange>,
}

/// One child record requested from a reviewed partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChildInput {
    /// Inclusive, one-based first page in the original.
    pub first: u32,
    /// Inclusive, one-based last page in the original.
    pub last: u32,
    /// Operator-facing filename for the child record.
    pub filename: String,
}

/// A complete operator-approved partition request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplitRequest {
    /// Number of pages in the unchanged parent PDF.
    pub page_count: u32,
    /// Ordered child records covering each page exactly once.
    pub children: Vec<ChildInput>,
}

/// A page-range partition is empty, out of bounds, overlapping, or incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionError {
    /// The parent has no pages.
    EmptyParent,
    /// No child ranges were provided.
    NoRanges,
    /// A range starts at page zero or ends before it starts.
    InvalidRange,
    /// A range extends beyond the parent page count.
    OutOfBounds,
    /// Ranges overlap or leave one or more parent pages unassigned.
    NotExactPartition,
}

impl fmt::Display for PartitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyParent => "parent PDF must have at least one page",
            Self::NoRanges => "split must contain at least one child range",
            Self::InvalidRange => "page ranges must be one-based and non-empty",
            Self::OutOfBounds => "page range exceeds the parent page count",
            Self::NotExactPartition => "child ranges must cover every parent page exactly once",
        };
        f.write_str(message)
    }
}

impl std::error::Error for PartitionError {}

impl PageRange {
    /// Create an inclusive, one-based page range.
    ///
    /// # Errors
    /// The first page is zero or the last page precedes the first.
    pub fn new(first: u32, last: u32) -> Result<Self, PartitionError> {
        if first == 0 || last < first {
            return Err(PartitionError::InvalidRange);
        }
        Ok(Self { first, last })
    }
}

impl Partition {
    /// Validate child ranges as an ordered, gap-free, non-overlapping partition.
    ///
    /// Each range represents one child. The parent remains the source of the page
    /// count and is not modified by validation.
    ///
    /// # Errors
    /// The parent has no pages, no ranges were supplied, a range is invalid or
    /// out of bounds, or the ranges overlap or leave pages unassigned.
    pub fn new(page_count: u32, ranges: Vec<PageRange>) -> Result<Self, PartitionError> {
        if page_count == 0 {
            return Err(PartitionError::EmptyParent);
        }
        if ranges.is_empty() {
            return Err(PartitionError::NoRanges);
        }

        let mut next_page = 1_u64;
        for range in &ranges {
            if range.first == 0 || range.last < range.first {
                return Err(PartitionError::InvalidRange);
            }
            if range.last > page_count {
                return Err(PartitionError::OutOfBounds);
            }
            if u64::from(range.first) != next_page {
                return Err(PartitionError::NotExactPartition);
            }
            next_page = u64::from(range.last) + 1;
        }
        if next_page != u64::from(page_count) + 1 {
            return Err(PartitionError::NotExactPartition);
        }

        Ok(Self { page_count, ranges })
    }

    /// Number of pages in the unchanged parent PDF.
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// The ordered, inclusive page ranges assigned to child documents.
    pub fn ranges(&self) -> &[PageRange] {
        &self.ranges
    }
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
