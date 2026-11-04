//! The Sjel node this process talks to, resolved once from the launch environment.
//!
//! `SJEL_STATUS_URL` names the shell endpoint; `SJEL_ENV=dev` marks the seeded demo node that
//! `sjel demo app` starts the app against (docs/plans/gpui.md, "Dev and prod").

use std::sync::OnceLock;

#[derive(Debug)]
pub struct Environment {
    pub endpoint: String,
    pub dev: bool,
}

pub fn current() -> &'static Environment {
    static CURRENT: OnceLock<Environment> = OnceLock::new();
    CURRENT.get_or_init(|| Environment {
        endpoint: std::env::var("SJEL_STATUS_URL")
            .unwrap_or_else(|_| crate::shell::DEFAULT_ENDPOINT.to_owned()),
        dev: std::env::var("SJEL_ENV").is_ok_and(|v| v == "dev"),
    })
}
