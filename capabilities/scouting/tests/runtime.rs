use scouting::opportunity::{Opportunity, OpportunityType};
use scouting::source::{SearchQuery, SourceAdapter, SourceError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Source(AtomicUsize);
impl SourceAdapter for Source {
    fn name(&self) -> &str { "test" }
    fn opportunity_type(&self) -> OpportunityType { OpportunityType::Event }
    fn rate_limit_per_min(&self) -> u32 { 1 }
    fn search(&self, _: &SearchQuery) -> Result<Vec<Opportunity>, SourceError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }
}

#[test]
fn runtime_child() {
    if std::env::var_os("SJEL_RUNTIME_TEST_CHILD").is_none() { return }
    let source = Source(AtomicUsize::new(0));
    let query = SearchQuery::default();
    let error = scouting::pipeline::run(&source, &query, &[], None, None, None).unwrap_err();
    assert!(error.to_string().starts_with("deferred by runtime profile:"));
    assert_eq!(source.0.load(Ordering::Relaxed), 0);
    assert!(scouting::pipeline::fetch_json(&source, &query).unwrap().is_empty());
    assert_eq!(source.0.load(Ordering::Relaxed), 1, "lightweight collection remains available");
}

#[test]
fn scoring_defers_without_disabling_collection() {
    let root = std::env::temp_dir().join(format!("sjel-scout-runtime-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(root.join("config/runtime")).unwrap();
    std::fs::write(root.join("config/runtime/test.json"), r#"{"selection":"on-the-go","allow":[]}"#).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_child", "--nocapture"])
        .env("SJEL_RUNTIME_TEST_CHILD", &root)
        .env("SJEL_PERSONAL_ROOT", &root)
        .env("SJEL_MACHINE_TOML", root.join("config/machines/test.toml"))
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(root).unwrap();
}
