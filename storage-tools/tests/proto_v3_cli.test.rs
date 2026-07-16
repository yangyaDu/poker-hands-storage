use std::fs;
use std::path::Path;
use std::process::Command;

use range_store_core::sqlite::Connection;

#[test]
fn v3_cli_exports_verifies_cross_checks_and_benchmarks_without_v2() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.db");
    let root = temp.path().join("v3-root");
    let archive = root.join("default_6max_100BB");
    let verify_report = temp.path().join("verify.json");
    let cross_report = temp.path().join("cross.json");
    let benchmark_report = temp.path().join("benchmark.json");
    let benchmark_markdown = temp.path().join("benchmark.md");
    build_source_fixture(&source);

    run_ok(&[
        "v3-export",
        "--source",
        path(&source),
        "--out",
        path(&archive),
        "--dimension",
        "default:6:100",
    ]);
    run_ok(&[
        "v3-verify",
        "--archive",
        path(&archive),
        "--out",
        path(&verify_report),
    ]);
    run_ok(&[
        "v3-cross-verify",
        "--source",
        path(&source),
        "--archive",
        path(&archive),
        "--out",
        path(&cross_report),
    ]);
    run_ok(&[
        "v3-benchmark",
        "--source",
        path(&source),
        "--archive-root",
        path(&root),
        "--dimension",
        "default:6:100",
        "--iterations",
        "3",
        "--warmup-iterations",
        "1",
        "--out",
        path(&benchmark_report),
        "--md",
        path(&benchmark_markdown),
    ]);

    for report_path in [&verify_report, &cross_report] {
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
        assert_eq!(report["ok"], true);
        assert_eq!(report["failureCount"], 0);
    }
    let benchmark: serde_json::Value =
        serde_json::from_slice(&fs::read(&benchmark_report).unwrap()).unwrap();
    assert_eq!(benchmark["enginePair"], "sqlite-v3");
    assert_eq!(benchmark["correctnessVerified"], true);
    assert!(benchmark["metadataSummary"]["p95Ms"].is_number());
    assert!(benchmark["strategySummary"]["p95Ms"].is_number());
    assert!(benchmark["cache"]["metadata"]["residentEstimatedBytes"].is_number());
    assert!(benchmark["memory"].get("before").is_some());
    let names = benchmark["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    for expected in [
        "v3_cold_open",
        "sqlite_metadata",
        "v3_metadata_hit",
        "v3_first_strategy_decode",
        "sqlite_strategy",
        "v3_strategy_hit",
        "v3_batch",
        "v3_hands_by_actions",
        "v3_handle_reopen",
    ] {
        assert!(
            names.contains(&expected),
            "missing benchmark case {expected}"
        );
    }
    assert!(benchmark_markdown.is_file());

    let all_root = temp.path().join("all-v3");
    run_ok(&[
        "v3-export-all",
        "--source",
        path(&source),
        "--out-root",
        path(&all_root),
    ]);
    assert!(all_root
        .join("default_6max_100BB")
        .join("manifest.json")
        .is_file());
}

fn run_ok(args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "args={args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn build_source_fixture(path: &Path) {
    Connection::open(path, false)
        .unwrap()
        .exec(
            "CREATE TABLE concrete_lines_default_6max_100BB(
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE drill_scenario_lines_default(
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             CREATE TABLE range_data_default_6max_100BB(
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL
             );
             INSERT INTO concrete_lines_default_6max_100BB VALUES
               (10, 'A', 'A-1'),
               (20, 'A', 'A-2');
             INSERT INTO drill_scenario_lines_default VALUES
               (1, 'rfi', 'A', 6, 100);
             INSERT INTO range_data_default_6max_100BB VALUES
               (10, 'AA', 'fold', 0.0, 0.0, 0.0, NULL),
               (10, 'AA', 'raise', 2.5, 2.5, 1.0, 1.25),
               (20, 'KK', 'call', 1.0, 1.0, 1.0, -0.5);",
        )
        .unwrap();
}
