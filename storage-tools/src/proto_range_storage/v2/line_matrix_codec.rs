use std::collections::{BTreeMap, HashSet};

use range_store_core::hole_cards::get_hand_id;

use crate::errors::ToolError;
use crate::proto_range_storage::v2::sqlite_source::SourceRow;

use crate::proto_range_storage::v2::proto::{
    ActionType, CompactActionColumn, CompactLineMatrix, HandEncoding,
};

pub(crate) const HAND_COUNT_169: usize = 169;
pub(crate) const BITMAP_BYTES_169: usize = HAND_COUNT_169.div_ceil(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ActionKey {
    action_type: i32,
    action_size_x10000: u32,
    amount_centi_bb: u32,
}

#[derive(Debug)]
struct NormalizedRow {
    hand_id: usize,
    action: ActionKey,
    frequency_x10000: u32,
    ev_x10000: i32,
}

pub(crate) fn build_compact_line_matrix(
    rows: &[SourceRow],
) -> Result<CompactLineMatrix, ToolError> {
    let mut normalized_rows = Vec::with_capacity(rows.len());
    let mut seen_cells = HashSet::new();
    for row in rows {
        let Some(hand_ev) = row.hand_ev else {
            continue;
        };
        let frequency_x10000 = quantize_frequency(row.frequency)?;
        let action = ActionKey {
            action_type: normalize_action_type(&row.action_name)? as i32,
            action_size_x10000: quantize_u32("action_size", row.action_size, 10_000.0)?,
            amount_centi_bb: quantize_u32("amount_bb", row.amount_bb, 100.0)?,
        };
        let hand_id = usize::from(get_hand_id(&row.hole_cards)?);
        if !seen_cells.insert((hand_id, action)) {
            return Err(ToolError::new(
                "DUPLICATE_ACTION_CELL",
                format!(
                    "Duplicate source row for hand_id={hand_id} and action=({}, {}, {})",
                    action.action_type, action.action_size_x10000, action.amount_centi_bb
                ),
            ));
        }
        normalized_rows.push(NormalizedRow {
            hand_id,
            action,
            frequency_x10000,
            ev_x10000: quantize_ev(hand_ev)?,
        });
    }
    if normalized_rows.is_empty() {
        return Err(ToolError::new(
            "LINE_MATRIX_EMPTY",
            "Cannot encode a line without non-NULL EV action cells",
        ));
    }

    let mut valid_hand_bitmap = vec![0; BITMAP_BYTES_169];
    for row in &normalized_rows {
        set_bit(&mut valid_hand_bitmap, row.hand_id);
    }
    let hand_id_to_global_index = build_compact_index_map(&valid_hand_bitmap, HAND_COUNT_169);
    let valid_hand_count = count_bits(&valid_hand_bitmap);
    let action_bitmap_bytes = valid_hand_count.div_ceil(8);

    let mut rows_by_action = BTreeMap::<ActionKey, Vec<NormalizedRow>>::new();
    for row in normalized_rows {
        rows_by_action.entry(row.action).or_default().push(row);
    }

    let mut actions = Vec::with_capacity(rows_by_action.len());
    for (action, mut rows) in rows_by_action {
        rows.sort_by_key(|row| row.hand_id);
        let mut action_hand_bitmap = vec![0; action_bitmap_bytes];
        let mut frequency_x10000 = Vec::with_capacity(rows.len());
        let mut ev_x10000 = Vec::with_capacity(rows.len());
        for row in rows {
            let global_index = hand_id_to_global_index[row.hand_id];
            debug_assert!(global_index >= 0);
            let global_index = global_index as usize;
            set_bit(&mut action_hand_bitmap, global_index);
            frequency_x10000.push(row.frequency_x10000);
            ev_x10000.push(row.ev_x10000);
        }
        actions.push(CompactActionColumn {
            action_type: action.action_type,
            amount_centi_bb: action.amount_centi_bb,
            action_size_x10000: action.action_size_x10000,
            frequency_x10000,
            ev_x10000,
            action_hand_bitmap,
        });
    }

    let matrix = CompactLineMatrix {
        schema_version: 2,
        hand_encoding: HandEncoding::HandEncoding169 as i32,
        actions,
        valid_hand_bitmap,
    };
    validate_compact_line_matrix(&matrix)?;
    Ok(matrix)
}

pub(crate) fn validate_compact_line_matrix(matrix: &CompactLineMatrix) -> Result<(), ToolError> {
    if matrix.schema_version != 2 {
        return Err(invalid_matrix(format!(
            "schema_version must be 2, got {}",
            matrix.schema_version
        )));
    }
    if matrix.hand_encoding != HandEncoding::HandEncoding169 as i32 {
        return Err(invalid_matrix("Proto exporter requires HAND_ENCODING_169"));
    }
    if matrix.actions.is_empty() {
        return Err(invalid_matrix("actions must not be empty"));
    }
    validate_bitmap(
        "valid_hand_bitmap",
        &matrix.valid_hand_bitmap,
        HAND_COUNT_169,
    )?;
    let valid_hand_count = count_bits(&matrix.valid_hand_bitmap);
    if valid_hand_count == 0 {
        return Err(invalid_matrix(
            "valid_hand_bitmap must contain at least one hand",
        ));
    }
    let mut identities = HashSet::new();
    let mut union = vec![0u8; valid_hand_count.div_ceil(8)];
    for (action_index, action) in matrix.actions.iter().enumerate() {
        if ActionType::try_from(action.action_type).is_err()
            || action.action_type == ActionType::Unspecified as i32
        {
            return Err(invalid_matrix(format!(
                "actions[{action_index}].action_type is invalid"
            )));
        }
        if !identities.insert((
            action.action_type,
            action.action_size_x10000,
            action.amount_centi_bb,
        )) {
            return Err(invalid_matrix(format!(
                "actions[{action_index}] duplicates an action identity"
            )));
        }
        validate_bitmap(
            &format!("actions[{action_index}].action_hand_bitmap"),
            &action.action_hand_bitmap,
            valid_hand_count,
        )?;
        let expected_values = count_bits(&action.action_hand_bitmap);
        if action.frequency_x10000.len() != expected_values
            || action.ev_x10000.len() != expected_values
        {
            return Err(invalid_matrix(format!(
                "actions[{action_index}] frequency/EV arrays must contain {expected_values} elements"
            )));
        }
        for (value_index, frequency) in action.frequency_x10000.iter().enumerate() {
            if *frequency > 10_000 {
                return Err(invalid_matrix(format!(
                    "actions[{action_index}].frequency_x10000[{value_index}] exceeds 10000"
                )));
            }
        }
        for (byte_index, byte) in action.action_hand_bitmap.iter().enumerate() {
            union[byte_index] |= byte;
        }
    }
    if count_bits(&union) != valid_hand_count {
        return Err(invalid_matrix(
            "valid_hand_bitmap must equal the union of action_hand_bitmap coverage",
        ));
    }
    Ok(())
}

pub(crate) fn build_compact_index_map(bitmap: &[u8], total_count: usize) -> Vec<i16> {
    let mut mapping = vec![-1; total_count];
    let mut compact_index = 0i16;
    for (original_index, slot) in mapping.iter_mut().enumerate() {
        if bit_is_set(bitmap, original_index) {
            *slot = compact_index;
            compact_index += 1;
        }
    }
    mapping
}

fn normalize_action_type(value: &str) -> Result<ActionType, ToolError> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect::<String>();
    match normalized.as_str() {
        "fold" => Ok(ActionType::Fold),
        "check" => Ok(ActionType::Check),
        "call" => Ok(ActionType::Call),
        "bet" => Ok(ActionType::Bet),
        "raise" => Ok(ActionType::Raise),
        "allin" => Ok(ActionType::Allin),
        _ => Err(ToolError::new(
            "UNKNOWN_ACTION_TYPE",
            format!("Unknown action name: {value}"),
        )),
    }
}

fn quantize_u32(name: &str, value: f64, scale: f64) -> Result<u32, ToolError> {
    if !value.is_finite() || value < 0.0 {
        return Err(ToolError::new(
            "INVALID_NUMERIC_VALUE",
            format!("{name} must be finite and non-negative, got {value}"),
        ));
    }
    let scaled = (value * scale).round();
    if scaled > f64::from(u32::MAX) {
        return Err(ToolError::new(
            "NUMERIC_OVERFLOW",
            format!("{name}={value} exceeds uint32 after scaling by {scale}"),
        ));
    }
    Ok(scaled as u32)
}

fn quantize_frequency(value: f64) -> Result<u32, ToolError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ToolError::new(
            "INVALID_FREQUENCY",
            format!("frequency must be between 0 and 1, got {value}"),
        ));
    }
    Ok((value * 10_000.0).round() as u32)
}

fn quantize_ev(value: f64) -> Result<i32, ToolError> {
    if !value.is_finite() {
        return Err(ToolError::new(
            "INVALID_NUMERIC_VALUE",
            format!("hand_ev must be finite when present, got {value}"),
        ));
    }
    let scaled = (value * 10_000.0).round();
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return Err(ToolError::new(
            "NUMERIC_OVERFLOW",
            format!("hand_ev={value} exceeds sint32 after scaling"),
        ));
    }
    Ok(scaled as i32)
}

fn validate_bitmap(name: &str, bitmap: &[u8], total_count: usize) -> Result<(), ToolError> {
    let expected_bytes = total_count.div_ceil(8);
    if bitmap.len() != expected_bytes {
        return Err(invalid_matrix(format!(
            "{name} must contain {expected_bytes} bytes, got {}",
            bitmap.len()
        )));
    }
    if !total_count.is_multiple_of(8) && !bitmap.is_empty() {
        let padding_mask = !((1u8 << (total_count % 8)) - 1);
        if bitmap[expected_bytes - 1] & padding_mask != 0 {
            return Err(invalid_matrix(format!("{name} has non-zero padding bits")));
        }
    }
    Ok(())
}

pub(crate) fn bit_is_set(bitmap: &[u8], index: usize) -> bool {
    bitmap[index / 8] & (1u8 << (index % 8)) != 0
}

fn set_bit(bitmap: &mut [u8], index: usize) {
    bitmap[index / 8] |= 1u8 << (index % 8);
}

pub(crate) fn count_bits(bitmap: &[u8]) -> usize {
    bitmap.iter().map(|byte| byte.count_ones() as usize).sum()
}

fn invalid_matrix(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_COMPACT_LINE_MATRIX", message)
}
