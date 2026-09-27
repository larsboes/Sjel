//! Empirical rail punctuality.
//!
//! What this capability owns: how reliable a train actually is, measured rather than
//! predicted. It folds Deutsche Bahn's own published stop history (`upstreams.toml`
//! [deutsche-bahn-data]) into per-station statistics and serves them to whoever asks —
//! today `punctuality stats`, next `capabilities/transit`, which carries a
//! `delay_risk_score` field that has been `None` since the port.
//!
//! It is deliberately the bottom rung of the intelligence ladder (CONTRIBUTING.md#implementation-languages-and-intelligence). A
//! lookup table over two years of observations is not a placeholder for a model; it is
//! the number a model has to beat, and without it "our prediction is good" is not a
//! claim anyone can check.

pub mod config;
pub mod dataset;
pub mod ingest;
pub mod ride;
pub mod stats;
pub mod store;
