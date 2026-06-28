use poker_hands_storage_tools::verification::report::{
    render_markdown, RangeStrataVerifyReport, VerifyMode, VerifyOptionsSummary,
};

#[test]
fn empty_standalone_report_has_upstream_shape() {
    let report = RangeStrataVerifyReport::new(
        VerifyMode::Standalone,
        "data/range-strata".to_owned(),
        None,
        false,
        VerifyOptionsSummary::default(),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(report.mode, VerifyMode::Standalone);
    assert!(report.totals.manifest_ok);
    assert!(report.totals.catalog_ok);
    assert_eq!(report.failures.len(), 0);
    assert_eq!(report.precision_policy.numeric_fields, "float32-bit-exact");

    let markdown = render_markdown(&report);
    assert!(markdown.contains("Range Strata Binary Integrity Report"));
    assert!(markdown.contains("All checks passed"));
}
