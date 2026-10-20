//! inventory: what the household owns, wants and gave away.
//!
//! One item table for furniture, gear and clothes. Placement in a flat is spatial and belongs to
//! `interior`; the test for the boundary is whether a room is needed to answer the question.
//! This is its own capability because `interior` is on-demand, and trips' packing lists need the
//! rows after every restart.

pub mod api;
pub mod budget;
pub mod import;
pub mod intake;
pub mod store;
