//! The data layer's GPUI half lives in `sjel_gpui_ui::data`, shared with the phone.

pub use sjel_gpui_ui::data::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// `Repo` lives in `core`, so `cfg(test)` there cannot see this test: the app enables core's
    /// `test-support` feature from its dev-dependencies instead. Without that line these window
    /// tests read this machine's real `axon.db`, which made them slow and flaky rather than red.
    #[test]
    fn these_tests_read_no_local_database() {
        assert!(
            Repo::local_db_path().is_none(),
            "a test build must not resolve a database path"
        );
    }
}
