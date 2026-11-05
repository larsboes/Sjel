//! Tab enumerations and constants for the Research views.

pub const SUMMARY_MAX: usize = 110;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchTab {
    Articles,
    Projects,
    Benchmarks,
}
