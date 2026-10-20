//! Benchmark evaluation for the specialist agent claim (ISA.md: libs/agent Wave 2).
//!
//! Evaluates whether the specialist configuration (with `skills` and `cargo` diagnostic tool)
//! produces code with fewer clippy warnings across a fixed set of Rust tasks compared to baseline.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SpecialistTask {
    pub id: String,
    pub title: String,
    pub initial_code: String,
    pub instruction: String,
    pub expected_fixed_code: String,
    pub clippy_lint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskResult {
    pub task_id: String,
    pub passed_clippy: bool,
    pub warnings_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpecialistMetrics {
    pub total_tasks: usize,
    pub baseline_clean_share: f64,
    pub specialist_clean_share: f64,
    pub delta: f64,
}

pub fn calculate_metrics(baseline: &[TaskResult], specialist: &[TaskResult]) -> SpecialistMetrics {
    assert_eq!(baseline.len(), specialist.len());
    let total = baseline.len();
    if total == 0 {
        return SpecialistMetrics {
            total_tasks: 0,
            baseline_clean_share: 0.0,
            specialist_clean_share: 0.0,
            delta: 0.0,
        };
    }
    let baseline_clean = baseline.iter().filter(|r| r.passed_clippy).count();
    let specialist_clean = specialist.iter().filter(|r| r.passed_clippy).count();

    let baseline_share = baseline_clean as f64 / total as f64;
    let specialist_share = specialist_clean as f64 / total as f64;

    SpecialistMetrics {
        total_tasks: total,
        baseline_clean_share: baseline_share,
        specialist_clean_share: specialist_share,
        delta: specialist_share - baseline_share,
    }
}

#[test]
fn specialist_task_set_is_valid_and_non_empty() {
    let raw = include_str!("specialist_tasks.json");
    let tasks: Vec<SpecialistTask> = serde_json::from_str(raw).expect("valid specialist tasks JSON");
    assert_eq!(tasks.len(), 5, "the specialist task suite defines exactly 5 canonical tasks");

    for task in &tasks {
        assert!(!task.id.is_empty(), "task id must not be empty");
        assert!(!task.instruction.is_empty(), "task instruction must not be empty");
        assert!(!task.clippy_lint.is_empty(), "task must name target clippy lint");
    }
}

#[test]
fn metrics_calculation_scores_pass_shares_and_delta_accurately() {
    let baseline = vec![
        TaskResult { task_id: "t1".into(), passed_clippy: true, warnings_count: 0 },
        TaskResult { task_id: "t2".into(), passed_clippy: false, warnings_count: 1 },
        TaskResult { task_id: "t3".into(), passed_clippy: false, warnings_count: 2 },
    ];
    let specialist = vec![
        TaskResult { task_id: "t1".into(), passed_clippy: true, warnings_count: 0 },
        TaskResult { task_id: "t2".into(), passed_clippy: true, warnings_count: 0 },
        TaskResult { task_id: "t3".into(), passed_clippy: true, warnings_count: 0 },
    ];

    let metrics = calculate_metrics(&baseline, &specialist);
    assert_eq!(metrics.total_tasks, 3);
    assert!((metrics.baseline_clean_share - 0.3333).abs() < 0.01);
    assert!((metrics.specialist_clean_share - 1.0).abs() < 0.01);
    assert!((metrics.delta - 0.6666).abs() < 0.01);
}
