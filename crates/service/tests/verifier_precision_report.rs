use poker_hands_storage_service::verifier::precision::{
    check_float32_round_trip, check_nullable_float32_round_trip, format_float32_bits,
    Float32CheckReason, Float32PrecisionStatsAccumulator, NullableFloat32CheckReason,
};
use poker_hands_storage_service::verifier::report::{
    render_markdown, DimensionVerifyDetail, RangeStrataVerifyReport, VerifyFailure, VerifyLayer,
    VerifyMode, VerifyOptionsSummary,
};

#[test]
fn float32_round_trip_matches_exact_bits() {
    let check = check_float32_round_trip(0.1, f32::from_bits(0x3dcc_cccd) as f64);

    assert!(check.ok);
    assert_eq!(check.reason, Float32CheckReason::Ok);
    assert_eq!(check.expected_bits, check.actual_bits);
}

#[test]
fn float32_round_trip_rejects_legacy_tolerance_mismatch() {
    let check = check_float32_round_trip(0.5000000596046448, 0.5);

    assert!(!check.ok);
    assert_eq!(check.reason, Float32CheckReason::Float32ValueMismatch);
    assert_eq!(format_float32_bits(check.expected_bits), "0x3f000001");
    assert_eq!(format_float32_bits(check.actual_bits), "0x3f000000");
}

#[test]
fn float32_round_trip_preserves_signed_zero() {
    let check = check_float32_round_trip(-0.0, 0.0);

    assert!(!check.ok);
    assert_eq!(format_float32_bits(check.expected_bits), "0x80000000");
    assert_eq!(format_float32_bits(check.actual_bits), "0x00000000");
}

#[test]
fn nullable_float32_handles_null_before_numeric_comparison() {
    assert!(check_nullable_float32_round_trip(None, None).ok);
    assert_eq!(
        check_nullable_float32_round_trip(None, Some(0.0)).reason,
        NullableFloat32CheckReason::NullMismatch
    );
    assert_eq!(
        check_nullable_float32_round_trip(Some(1.25), None).reason,
        NullableFloat32CheckReason::NullMismatch
    );
    assert!(check_nullable_float32_round_trip(Some(1.25), Some(1.25)).ok);
}

#[test]
fn accumulator_counts_matches_mismatches_and_nulls() {
    let mut stats = Float32PrecisionStatsAccumulator::new(2, 10);
    stats.add(
        check_float32_round_trip(0.1, f32::from_bits(0x3dcc_cccd) as f64),
        "a",
    );
    stats.add(check_float32_round_trip(0.5000000596046448, 0.5), "b");
    stats.add_null();

    let summary = stats.to_summary();
    assert_eq!(summary.checked_values, 2);
    assert_eq!(summary.null_values, 1);
    assert_eq!(summary.bit_exact_values, 1);
    assert_eq!(summary.mismatch_values, 1);
    assert_eq!(summary.top_quantization_errors.len(), 2);
}

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
