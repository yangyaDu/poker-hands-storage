use poker_hands_storage_tools::verification::float32_precision::{
    check_float32_round_trip, check_nullable_float32_round_trip, format_float32_bits,
    Float32CheckReason, Float32PrecisionStatsAccumulator, NullableFloat32CheckReason,
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
