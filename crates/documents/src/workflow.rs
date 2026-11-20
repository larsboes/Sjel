//! Derive the next operator step from stored document evidence.

use crate::Document;

/// Current next step for a document. This is derived, so retries and reanalysis cannot leave a
/// stale workflow flag behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Analyze,
    ReviewSplit,
    Split,
    ReviewEnrichment,
    File,
    Filed,
    Archived,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Analyze => "analyze",
            Self::ReviewSplit => "review_split",
            Self::Split => "split",
            Self::ReviewEnrichment => "review_enrichment",
            Self::File => "file",
            Self::Filed => "filed",
            Self::Archived => "archived",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Analyze => "Needs analysis",
            Self::ReviewSplit => "Review split",
            Self::Split => "Split into children",
            Self::ReviewEnrichment => "Review enrichment",
            Self::File => "Ready to file",
            Self::Filed => "Filed",
            Self::Archived => "Archived",
        }
    }
}

/// Derive the next stage from document evidence and the newest review state.
///
/// A pending proposal is an unreviewed category from the newest proposal run. `has_children`
/// identifies an original that has already been split into stored child PDFs.
pub fn stage(document: &Document, has_pending_proposal: bool, has_children: bool) -> Stage {
    stage_from(
        document,
        &crate::Derived::of(document),
        has_pending_proposal,
        has_children,
    )
}

/// [`stage`] from cached derived values, for a record listed without its extracted text.
pub fn stage_from(
    document: &Document,
    derived: &crate::Derived,
    has_pending_proposal: bool,
    has_children: bool,
) -> Stage {
    if document.status == "archived" {
        return Stage::Archived;
    }
    if has_children {
        return Stage::Split;
    }
    if document.status == "filed" {
        return Stage::Filed;
    }

    if derived.analysis_gap {
        return Stage::Analyze;
    }

    if derived.split_proposed {
        return Stage::ReviewSplit;
    }

    if has_pending_proposal {
        return Stage::ReviewEnrichment;
    }

    Stage::File
}

/// A PDF without page evidence, with a failed page, or without any extracted text.
pub fn has_analysis_gap(document: &Document) -> bool {
    document.media_type == "application/pdf"
        && (document.pages.is_empty()
            || document.pages.iter().any(|page| page.failure.is_some())
            || document
                .extracted_text
                .as_deref()
                .is_none_or(|text| text.chars().all(char::is_whitespace)))
}
