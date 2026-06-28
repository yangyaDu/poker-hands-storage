use poker_hands_storage_tools::verification::report::{
    DimensionVerifyDetail, RangeStrataVerifyReport, VerifyFailure, VerifyLayer, VerifyMode,
    VerifyOptionsSummary,
};

#[test]
fn report_totals_reflect_structural_failures() {
    let failures = vec![VerifyFailure {
        layer: VerifyLayer::IndexPackCross,
        check: "dimension:default:6max:100BB".to_owned(),
        reason: "PACK_SIZE_MISMATCH".to_owned(),
        message: "bad pack size".to_owned(),
        context: None,
    }];
    let report = RangeStrataVerifyReport::new(
        VerifyMode::Standalone,
        "data/range-strata".to_owned(),
        None,
        true,
        VerifyOptionsSummary::default(),
        vec![DimensionVerifyDetail {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
            checked: true,
            index_records: 1,
            bin_file_size_bytes: 16,
            idx_file_size_bytes: 38,
            header_failures: 0,
            index_pack_cross_failures: 1,
            source_cross_failures: None,
            source_cross_records: None,
        }],
        failures,
    );

    assert_eq!(report.totals.index_pack_cross_failures, 1);
    assert_eq!(report.totals.index_files_failed, 1);
    assert_eq!(report.repair_suggestions.len(), 1);
}
