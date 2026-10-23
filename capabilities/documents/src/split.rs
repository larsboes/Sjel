//! Validation for reviewed, metadata-only PDF partitions.
//!
//! A split assigns each page of a parent to exactly one child range. This module
//! does not create child bytes or persist decisions; callers supply an authoritative
//! page count and store the validated ranges with the parent reference.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// An inclusive, one-based page range for one child document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRange {
    /// First page assigned to this child.
    pub first: u32,
    /// Last page assigned to this child.
    pub last: u32,
}

/// A complete partition of a parent PDF into ordered child page ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Partition {
    page_count: u32,
    ranges: Vec<PageRange>,
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
