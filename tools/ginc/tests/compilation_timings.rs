use ginc::cli::Args;
use ginc::compile::GinCompiler;
use ginc::reporting::CheckOptions;
use test_fixtures::TempPackage;

#[test]
fn timing_report_survives_early_failure() {
    let package = TempPackage::new("timing_failure");
    let report = package.join("timings.json");
    let mut args = Args {
        input: package.path().to_path_buf(),
        timings_json: Some(report.clone()),
        ..Default::default()
    };
    assert!(!GinCompiler::compile(&mut args).is_success());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert!(report["object_cache_hit"].is_null());
    assert_eq!(report["phases"][0]["name"], "discover-sources");
    assert!(report["total_seconds"].as_f64().unwrap() >= 0.0);
}

#[test]
fn checking_distinguishes_frontend_from_diagnostic_rendering() {
    let package = TempPackage::new("timing_check");
    let input = package.write(
        "input.gin",
        "Word is in 0 to 4294967295\nidentity(a Word) Word: a\n",
    );
    let report = package.join("timings.json");
    let mut args = Args {
        input,
        timings_json: Some(report.clone()),
        ..Default::default()
    };
    assert!(GinCompiler::check(&mut args, CheckOptions::default()).is_success());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    let names: Vec<_> = report["phases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|phase| phase["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"front-end"));
    assert!(names.contains(&"typecheck-diagnostics"));
    assert!(!names.contains(&"typecheck"));
    assert!(report["object_cache_hit"].is_null());
}
