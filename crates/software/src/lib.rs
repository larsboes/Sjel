#![deny(unsafe_code)]

pub mod discover;
pub mod leftovers;
pub mod store;
pub mod tooling;
pub mod uninstall;

pub use store::{App, Decision, SoftwareStore};
