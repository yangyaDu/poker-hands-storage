use std::collections::{BTreeSet, HashMap, HashSet};

use range_store_core::hole_cards::get_hand_id;

use crate::errors::ToolError;

use super::proto::{ActionColumn, ActionType, HandEncoding, LineMatrix};
use super::source::SourceRow;

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
    hand_idx: usize,
    action: ActionKey,
    frequency_x10000: u32,
    ev_x10000: Option<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct MatrixStats {
    pub source_row_count: usize,
    pub present_action_cell_count: usize,
    pub null_ev_count: usize,
    pub hands_with_actions: usize,
    pub frequency_sum_mismatch_hand_count: usize,
    pub max_frequency_error_x10000: u32,
}

pub(crate) fn build_line_matrix(
    rows: &[SourceRow],
    gto_data_version: &str,
) -> Result<(LineMatrix, MatrixStats), ToolError> {
    if gto_data_version.trim().is_empty() {
        return Err(ToolError::invalid_argument(
            "--gto-data-version must not be empty",
        ));
    }

    let mut actions = BTreeSet::new();
    let mut normalized_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let action = ActionKey {
            action_type: normalize_action_type(&row.action_name)? as i32,
            action_size_x10000: quantize_u32("action_size", row.action_size, 10_000.0)?,
            amount_centi_bb: quantize_u32("amount_bb", row.amount_bb, 100.0)?,
        };
        let normalized = NormalizedRow {
            hand_idx: usize::from(get_hand_id(&row.hole_cards)?),
            action,
            frequency_x10000: quantize_frequency(row.frequency)?,
            ev_x10000: row.hand_ev.map(quantize_ev).transpose()?,
        };
        actions.insert(action);
        normalized_rows.push(normalized);
    }
    if actions.is_empty() {
        return Err(ToolError::new(
            "LINE_MATRIX_EMPTY",
            "Cannot encode a line without actions",
        ));
    }

    let actions = actions.into_iter().collect::<Vec<_>>();
    let action_indexes = actions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, action)| (action, index))
        .collect::<HashMap<_, _>>();
    let mut columns = actions
        .iter()
        .map(|action| ActionColumn {
            action_type: action.action_type,
            amount_centi_bb: action.amount_centi_bb,
            action_size_x10000: action.action_size_x10000,
            frequency_x10000: vec![0; HAND_COUNT_169],
            ev_x10000: vec![0; HAND_COUNT_169],
            action_hand_bitmap: vec![0; BITMAP_BYTES_169],
            ev_null_bitmap: vec![0; BITMAP_BYTES_169],
        })
        .collect::<Vec<_>>();

    let mut null_ev_count = 0usize;
    for row in normalized_rows {
        let action_index = action_indexes[&row.action];
        let column = &mut columns[action_index];
        if bit_is_set(&column.action_hand_bitmap, row.hand_idx) {
            return Err(ToolError::new(
                "DUPLICATE_ACTION_CELL",
                format!(
                    "Duplicate source row for hand_idx={} and action=({}, {}, {})",
                    row.hand_idx,
                    action_type_name(row.action.action_type),
                    row.action.action_size_x10000,
                    row.action.amount_centi_bb
                ),
            ));
        }
        set_bit(&mut column.action_hand_bitmap, row.hand_idx);
        column.frequency_x10000[row.hand_idx] = row.frequency_x10000;
        match row.ev_x10000 {
            Some(ev) => column.ev_x10000[row.hand_idx] = ev,
            None => {
                set_bit(&mut column.ev_null_bitmap, row.hand_idx);
                null_ev_count += 1;
            }
        }
    }

    let matrix = LineMatrix {
        schema_version: 1,
        gto_data_version: gto_data_version.to_owned(),
        hand_encoding: HandEncoding::HandEncoding169 as i32,
        actions: columns,
        invalid_hand_bitmap: vec![0; BITMAP_BYTES_169],
    };
    let validation = validate_line_matrix(&matrix)?;
    Ok((
        matrix,
        MatrixStats {
            source_row_count: rows.len(),
            present_action_cell_count: validation.present_action_cell_count,
            null_ev_count,
            hands_with_actions: validation.hands_with_actions,
            frequency_sum_mismatch_hand_count: validation.frequency_sum_mismatch_hand_count,
            max_frequency_error_x10000: validation.max_frequency_error_x10000,
        },
    ))
}

#[derive(Debug)]
struct ValidationStats {
    present_action_cell_count: usize,
    hands_with_actions: usize,
    frequency_sum_mismatch_hand_count: usize,
    max_frequency_error_x10000: u32,
}

pub(crate) fn validate_line_matrix(matrix: &LineMatrix) -> Result<MatrixStats, ToolError> {
    let validation = validate(matrix)?;
    Ok(MatrixStats {
        source_row_count: validation.present_action_cell_count,
        present_action_cell_count: validation.present_action_cell_count,
        null_ev_count: matrix
            .actions
            .iter()
            .map(|action| count_bits(&action.ev_null_bitmap))
            .sum(),
        hands_with_actions: validation.hands_with_actions,
        frequency_sum_mismatch_hand_count: validation.frequency_sum_mismatch_hand_count,
        max_frequency_error_x10000: validation.max_frequency_error_x10000,
    })
}

fn validate(matrix: &LineMatrix) -> Result<ValidationStats, ToolError> {
    if matrix.schema_version != 1 {
        return Err(invalid_matrix(format!(
            "schema_version must be 1, got {}",
            matrix.schema_version
        )));
    }
    if matrix.gto_data_version.trim().is_empty() {
        return Err(invalid_matrix("gto_data_version must not be empty"));
    }
    if matrix.hand_encoding != HandEncoding::HandEncoding169 as i32 {
        return Err(invalid_matrix("V1 exporter requires HAND_ENCODING_169"));
    }
    if matrix.actions.is_empty() {
        return Err(invalid_matrix("actions must not be empty"));
    }
    validate_bitmap("invalid_hand_bitmap", &matrix.invalid_hand_bitmap)?;

    let mut identities = HashSet::new();
    let mut present_action_cell_count = 0usize;
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
        if action.frequency_x10000.len() != HAND_COUNT_169
            || action.ev_x10000.len() != HAND_COUNT_169
        {
            return Err(invalid_matrix(format!(
                "actions[{action_index}] frequency/EV arrays must contain {HAND_COUNT_169} elements"
            )));
        }
        validate_bitmap(
            &format!("actions[{action_index}].action_hand_bitmap"),
            &action.action_hand_bitmap,
        )?;
        validate_bitmap(
            &format!("actions[{action_index}].ev_null_bitmap"),
            &action.ev_null_bitmap,
        )?;

        for hand_idx in 0..HAND_COUNT_169 {
            let present = bit_is_set(&action.action_hand_bitmap, hand_idx);
            let ev_is_null = bit_is_set(&action.ev_null_bitmap, hand_idx);
            if !present {
                if ev_is_null
                    || action.frequency_x10000[hand_idx] != 0
                    || action.ev_x10000[hand_idx] != 0
                {
                    return Err(invalid_matrix(format!(
                        "actions[{action_index}] has data for absent hand_idx={hand_idx}"
                    )));
                }
            } else {
                present_action_cell_count += 1;
                if action.frequency_x10000[hand_idx] > 10_000 {
                    return Err(invalid_matrix(format!(
                        "actions[{action_index}].frequency_x10000[{hand_idx}] exceeds 10000"
                    )));
                }
            }
        }
    }

    let mut hands_with_actions = 0usize;
    let mut frequency_sum_mismatch_hand_count = 0usize;
    let mut max_frequency_error_x10000 = 0u32;
    for hand_idx in 0..HAND_COUNT_169 {
        let invalid = bit_is_set(&matrix.invalid_hand_bitmap, hand_idx);
        let present_actions = matrix
            .actions
            .iter()
            .filter(|action| bit_is_set(&action.action_hand_bitmap, hand_idx))
            .collect::<Vec<_>>();
        if invalid {
            if !present_actions.is_empty() {
                return Err(invalid_matrix(format!(
                    "invalid hand_idx={hand_idx} must not contain actions"
                )));
            }
            continue;
        }
        if present_actions.is_empty() {
            continue;
        }
        hands_with_actions += 1;
        let sum = present_actions
            .iter()
            .map(|action| action.frequency_x10000[hand_idx])
            .sum::<u32>();
        let error = sum.abs_diff(10_000);
        max_frequency_error_x10000 = max_frequency_error_x10000.max(error);
        if error > present_actions.len() as u32 {
            frequency_sum_mismatch_hand_count += 1;
        }
    }

    Ok(ValidationStats {
        present_action_cell_count,
        hands_with_actions,
        frequency_sum_mismatch_hand_count,
        max_frequency_error_x10000,
    })
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

fn validate_bitmap(name: &str, bitmap: &[u8]) -> Result<(), ToolError> {
    if bitmap.len() != BITMAP_BYTES_169 {
        return Err(invalid_matrix(format!(
            "{name} must contain {BITMAP_BYTES_169} bytes, got {}",
            bitmap.len()
        )));
    }
    let used_bits_in_last_byte = HAND_COUNT_169 % 8;
    let padding_mask = !((1u8 << used_bits_in_last_byte) - 1);
    if bitmap[BITMAP_BYTES_169 - 1] & padding_mask != 0 {
        return Err(invalid_matrix(format!("{name} has non-zero padding bits")));
    }
    Ok(())
}

pub(crate) fn bit_is_set(bitmap: &[u8], hand_idx: usize) -> bool {
    bitmap[hand_idx / 8] & (1u8 << (hand_idx % 8)) != 0
}

fn set_bit(bitmap: &mut [u8], hand_idx: usize) {
    bitmap[hand_idx / 8] |= 1u8 << (hand_idx % 8);
}

fn count_bits(bitmap: &[u8]) -> usize {
    bitmap.iter().map(|byte| byte.count_ones() as usize).sum()
}

fn action_type_name(action_type: i32) -> &'static str {
    ActionType::try_from(action_type)
        .map(|value| value.as_str_name())
        .unwrap_or("ACTION_TYPE_INVALID")
}

fn invalid_matrix(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_LINE_MATRIX", message)
}
