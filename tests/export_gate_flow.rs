mod common;

use std::fs;

use agent_ruler::export_gate::{build_export_plan, commit_export};

use common::TestRuntimeDir;

#[test]
fn export_plan_for_file_includes_diff_and_summary() {
    let temp = TestRuntimeDir::new("export-plan");
    let src = temp.path().join("src.txt");
    let dst = temp.path().join("dst.txt");

    fs::write(&src, "new content\n").expect("write src");
    fs::write(&dst, "old content\n").expect("write dst");

    let plan = build_export_plan(&src, &dst).expect("build export plan");
    assert_eq!(plan.summary.files_changed, 1);
    assert!(!plan.diff_preview.is_empty());
}

#[test]
fn commit_export_copies_file_to_destination() {
    let temp = TestRuntimeDir::new("export-commit");
    let src = temp.path().join("artifact.txt");
    let dst = temp.path().join("nested").join("artifact.txt");

    fs::write(&src, "artifact-data").expect("write src");
    let plan = build_export_plan(&src, &dst).expect("plan");

    commit_export(&plan).expect("commit export");
    let copied = fs::read_to_string(&dst).expect("read dst");
    assert_eq!(copied, "artifact-data");
}
