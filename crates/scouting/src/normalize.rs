//! Unused scaffold, ported for parity with the original service.
//!
//! Every shipped adapter (see `adapters/`) implements its own `normalize()`
//! inline against its source's raw shape -- this top-level generic function
//! was scaffolded early and never wired into the pipeline. Kept as-is rather
//! than deleted so a future adapter that wants a shared normalize path has
//! a documented starting point; it is not called anywhere today.

use crate::opportunity::Opportunity;
use crate::source::SourceError;

#[allow(dead_code)]
pub fn normalize(_raw: &serde_json::Value) -> Result<Vec<Opportunity>, SourceError> {
    Err(SourceError::Parse(
        "normalize: scaffold, not implemented yet".into(),
    ))
}
