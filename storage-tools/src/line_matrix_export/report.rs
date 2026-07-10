use serde::Serialize;

use super::convert::{MatrixStats, BITMAP_BYTES_169, HAND_COUNT_169};
use super::proto::{ActionType, HandEncoding, LineMatrix};
use super::source::ResolvedLine;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugDocument<'a> {
    concrete_line_id: u32,
    abstract_line: &'a str,
    concrete_line: &'a str,
    schema_version: u32,
    gto_data_version: &'a str,
    hand_encoding: &'static str,
    hand_count: usize,
    invalid_hand_bitmap_hex: String,
    actions: Vec<DebugAction<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugAction<'a> {
    action_type: &'static str,
    amount_centi_bb: u32,
    action_size_x10000: u32,
    frequency_x10000: &'a [u32],
    ev_x10000: &'a [i32],
    action_hand_bitmap_hex: String,
    ev_null_bitmap_hex: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyDocument<'a> {
    pass: bool,
    concrete_line_id: u32,
    abstract_line: &'a str,
    concrete_line: &'a str,
    schema_version: u32,
    gto_data_version: &'a str,
    hand_encoding: &'static str,
    hand_count: usize,
    action_count: usize,
    source_row_count: usize,
    present_action_cell_count: usize,
    null_ev_count: usize,
    hands_with_actions: usize,
    hands_without_actions: usize,
    frequency_sum_mismatch_hand_count: usize,
    bitmap_bytes: usize,
    protobuf_bytes: usize,
    max_frequency_error_x10000: u32,
    checks: VerifyChecks,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyChecks {
    protobuf_roundtrip: bool,
    fixed_array_lengths: bool,
    fixed_bitmap_lengths: bool,
    unique_action_identities: bool,
    action_cells_match_source_rows: bool,
    source_frequency_sums_within_rounding_tolerance: bool,
}

pub(crate) fn debug_json(
    line: &ResolvedLine,
    matrix: &LineMatrix,
) -> Result<Vec<u8>, serde_json::Error> {
    let document = DebugDocument {
        concrete_line_id: line.concrete_line_id,
        abstract_line: &line.abstract_line,
        concrete_line: &line.concrete_line,
        schema_version: matrix.schema_version,
        gto_data_version: &matrix.gto_data_version,
        hand_encoding: hand_encoding_name(matrix.hand_encoding),
        hand_count: HAND_COUNT_169,
        invalid_hand_bitmap_hex: encode_hex(&matrix.invalid_hand_bitmap),
        actions: matrix
            .actions
            .iter()
            .map(|action| DebugAction {
                action_type: action_type_name(action.action_type),
                amount_centi_bb: action.amount_centi_bb,
                action_size_x10000: action.action_size_x10000,
                frequency_x10000: &action.frequency_x10000,
                ev_x10000: &action.ev_x10000,
                action_hand_bitmap_hex: encode_hex(&action.action_hand_bitmap),
                ev_null_bitmap_hex: encode_hex(&action.ev_null_bitmap),
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&document)
}

pub(crate) fn verify_json(
    line: &ResolvedLine,
    matrix: &LineMatrix,
    stats: &MatrixStats,
    protobuf_bytes: usize,
) -> Result<Vec<u8>, serde_json::Error> {
    let document = VerifyDocument {
        pass: true,
        concrete_line_id: line.concrete_line_id,
        abstract_line: &line.abstract_line,
        concrete_line: &line.concrete_line,
        schema_version: matrix.schema_version,
        gto_data_version: &matrix.gto_data_version,
        hand_encoding: hand_encoding_name(matrix.hand_encoding),
        hand_count: HAND_COUNT_169,
        action_count: matrix.actions.len(),
        source_row_count: stats.source_row_count,
        present_action_cell_count: stats.present_action_cell_count,
        null_ev_count: stats.null_ev_count,
        hands_with_actions: stats.hands_with_actions,
        hands_without_actions: HAND_COUNT_169 - stats.hands_with_actions,
        frequency_sum_mismatch_hand_count: stats.frequency_sum_mismatch_hand_count,
        bitmap_bytes: BITMAP_BYTES_169,
        protobuf_bytes,
        max_frequency_error_x10000: stats.max_frequency_error_x10000,
        checks: VerifyChecks {
            protobuf_roundtrip: true,
            fixed_array_lengths: true,
            fixed_bitmap_lengths: true,
            unique_action_identities: true,
            action_cells_match_source_rows: stats.source_row_count
                == stats.present_action_cell_count,
            source_frequency_sums_within_rounding_tolerance: stats
                .frequency_sum_mismatch_hand_count
                == 0,
        },
        warnings: frequency_warnings(stats),
    };
    serde_json::to_vec_pretty(&document)
}

fn frequency_warnings(stats: &MatrixStats) -> Vec<String> {
    if stats.frequency_sum_mismatch_hand_count == 0 {
        Vec::new()
    } else {
        vec![format!(
            "{} hand(s) have source frequency sums outside rounding tolerance; maximum x10000 error is {}",
            stats.frequency_sum_mismatch_hand_count, stats.max_frequency_error_x10000
        )]
    }
}

fn action_type_name(value: i32) -> &'static str {
    ActionType::try_from(value)
        .map(|action| action.as_str_name())
        .unwrap_or("ACTION_TYPE_INVALID")
}

fn hand_encoding_name(value: i32) -> &'static str {
    HandEncoding::try_from(value)
        .map(|encoding| encoding.as_str_name())
        .unwrap_or("HAND_ENCODING_INVALID")
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
