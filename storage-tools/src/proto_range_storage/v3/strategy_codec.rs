use std::collections::{BTreeMap, HashSet};

use range_store_core::hole_cards::get_hand_id;

use crate::errors::ToolError;

use super::proto::{ActionStrategyColumn, ActionType, HandEncoding, HandStrategy};
use super::source::SourceStrategyRow;

pub const HAND_COUNT_PREFLOP: usize = 169;
pub const HAND_BITMAP_BYTES_PREFLOP: usize = HAND_COUNT_PREFLOP.div_ceil(8);
pub const NULL_EV_FREQUENCY_SENTINEL: u32 = 20_000;

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
    hand_ev_x10000: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedActionValue {
    pub frequency_x10000: u32,
    pub hand_ev_x10000: i32,
    pub hand_ev_is_null: bool,
}

#[derive(Debug, Clone)]
pub struct DecodedHandStrategy {
    strategy: HandStrategy,
    hand_id_to_global_index: Vec<i16>,
    action_offsets: Vec<usize>,
    action_global_to_local_index: Vec<i16>,
}

pub(crate) fn build_hand_strategy(rows: &[SourceStrategyRow]) -> Result<HandStrategy, ToolError> {
    let mut normalized_rows = Vec::with_capacity(rows.len());
    let mut seen_cells = HashSet::new();
    for row in rows {
        let (frequency_x10000, hand_ev_x10000) = match row.hand_ev {
            Some(hand_ev) => (quantize_frequency(row.frequency)?, quantize_ev(hand_ev)?),
            None if row.frequency == 0.0 => (NULL_EV_FREQUENCY_SENTINEL, 0),
            None => {
                return Err(ToolError::new(
                    "NULL_EV_WITH_NONZERO_FREQUENCY",
                    format!(
                        "hand_ev is NULL but frequency is {} for hand {} action {}",
                        row.frequency, row.hole_cards, row.action_name
                    ),
                ));
            }
        };
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
            hand_ev_x10000,
        });
    }
    if normalized_rows.is_empty() {
        return Err(ToolError::new(
            "V3_HAND_STRATEGY_EMPTY",
            "Cannot encode an empty hand strategy",
        ));
    }

    let mut available_hand_bitmap = vec![0; HAND_BITMAP_BYTES_PREFLOP];
    for row in &normalized_rows {
        set_bit(&mut available_hand_bitmap, row.hand_id);
    }
    let hand_id_to_global_index =
        build_compact_index_map(&available_hand_bitmap, HAND_COUNT_PREFLOP);
    let available_hand_count = count_bits(&available_hand_bitmap);
    let action_bitmap_bytes = available_hand_count.div_ceil(8);
    let mut rows_by_action = BTreeMap::<ActionKey, Vec<NormalizedRow>>::new();
    for row in normalized_rows {
        rows_by_action.entry(row.action).or_default().push(row);
    }

    let mut actions = Vec::with_capacity(rows_by_action.len());
    for (action, mut rows) in rows_by_action {
        rows.sort_by_key(|row| row.hand_id);
        let mut action_hand_bitmap = vec![0; action_bitmap_bytes];
        let mut frequency_x10000 = Vec::with_capacity(rows.len());
        let mut hand_ev_x10000 = Vec::with_capacity(rows.len());
        for row in rows {
            let global_index = hand_id_to_global_index[row.hand_id];
            debug_assert!(global_index >= 0);
            set_bit(&mut action_hand_bitmap, global_index as usize);
            frequency_x10000.push(row.frequency_x10000);
            hand_ev_x10000.push(row.hand_ev_x10000);
        }
        actions.push(ActionStrategyColumn {
            action_type: action.action_type,
            amount_centi_bb: action.amount_centi_bb,
            action_size_x10000: action.action_size_x10000,
            frequency_x10000,
            hand_ev_x10000,
            action_hand_bitmap,
        });
    }
    let strategy = HandStrategy {
        schema_version: 3,
        hand_encoding: HandEncoding::Preflop as i32,
        actions,
        available_hand_bitmap,
    };
    validate_hand_strategy(&strategy)?;
    Ok(strategy)
}

pub fn validate_hand_strategy(strategy: &HandStrategy) -> Result<(), ToolError> {
    if strategy.schema_version != 3 {
        return Err(invalid_strategy(format!(
            "schema_version must be 3, got {}",
            strategy.schema_version
        )));
    }
    if strategy.hand_encoding != HandEncoding::Preflop as i32 {
        return Err(invalid_strategy(
            "V3 first release requires HAND_ENCODING_PREFLOP",
        ));
    }
    if strategy.actions.is_empty() {
        return Err(invalid_strategy("actions must not be empty"));
    }
    validate_bitmap(
        "available_hand_bitmap",
        &strategy.available_hand_bitmap,
        HAND_COUNT_PREFLOP,
    )?;
    let available_hand_count = count_bits(&strategy.available_hand_bitmap);
    if available_hand_count == 0 {
        return Err(invalid_strategy(
            "available_hand_bitmap must contain at least one hand",
        ));
    }
    let mut identities = HashSet::new();
    let mut union = vec![0u8; available_hand_count.div_ceil(8)];
    for (action_index, action) in strategy.actions.iter().enumerate() {
        if ActionType::try_from(action.action_type).is_err()
            || action.action_type == ActionType::Unspecified as i32
        {
            return Err(invalid_strategy(format!(
                "actions[{action_index}].action_type is invalid"
            )));
        }
        if !identities.insert((
            action.action_type,
            action.action_size_x10000,
            action.amount_centi_bb,
        )) {
            return Err(invalid_strategy(format!(
                "actions[{action_index}] duplicates an action identity"
            )));
        }
        validate_bitmap(
            &format!("actions[{action_index}].action_hand_bitmap"),
            &action.action_hand_bitmap,
            available_hand_count,
        )?;
        let expected_values = count_bits(&action.action_hand_bitmap);
        if action.frequency_x10000.len() != expected_values
            || action.hand_ev_x10000.len() != expected_values
        {
            return Err(invalid_strategy(format!(
                "actions[{action_index}] frequency/EV arrays must contain {expected_values} elements"
            )));
        }
        for (value_index, (&frequency, &hand_ev)) in action
            .frequency_x10000
            .iter()
            .zip(&action.hand_ev_x10000)
            .enumerate()
        {
            match frequency {
                0..=10_000 => {}
                NULL_EV_FREQUENCY_SENTINEL if hand_ev == 0 => {}
                NULL_EV_FREQUENCY_SENTINEL => {
                    return Err(invalid_strategy(format!(
                        "actions[{action_index}] null EV sentinel at {value_index} requires hand_ev_x10000=0"
                    )));
                }
                _ => {
                    return Err(invalid_strategy(format!(
                        "actions[{action_index}].frequency_x10000[{value_index}] is invalid"
                    )));
                }
            }
        }
        for (byte_index, byte) in action.action_hand_bitmap.iter().enumerate() {
            union[byte_index] |= byte;
        }
    }
    if union != all_set_prefix_bitmap(available_hand_count) {
        return Err(invalid_strategy(
            "available_hand_bitmap must equal the union of action hand coverage",
        ));
    }
    Ok(())
}

impl DecodedHandStrategy {
    pub fn new(strategy: HandStrategy) -> Result<Self, ToolError> {
        validate_hand_strategy(&strategy)?;
        let hand_id_to_global_index =
            build_compact_index_map(&strategy.available_hand_bitmap, HAND_COUNT_PREFLOP);
        let available_hand_count = count_bits(&strategy.available_hand_bitmap);
        let mut action_offsets = Vec::with_capacity(strategy.actions.len() + 1);
        let mut action_global_to_local_index =
            Vec::with_capacity(strategy.actions.len() * available_hand_count);
        for action in &strategy.actions {
            action_offsets.push(action_global_to_local_index.len());
            action_global_to_local_index.extend(build_compact_index_map(
                &action.action_hand_bitmap,
                available_hand_count,
            ));
        }
        action_offsets.push(action_global_to_local_index.len());
        Ok(Self {
            strategy,
            hand_id_to_global_index,
            action_offsets,
            action_global_to_local_index,
        })
    }

    pub fn strategy(&self) -> &HandStrategy {
        &self.strategy
    }

    pub fn action_value(&self, action_index: usize, hand_id: usize) -> Option<DecodedActionValue> {
        let global_index = *self.hand_id_to_global_index.get(hand_id)?;
        if global_index < 0 {
            return None;
        }
        let action = self.strategy.actions.get(action_index)?;
        let start = *self.action_offsets.get(action_index)?;
        let end = *self.action_offsets.get(action_index + 1)?;
        let position = start.checked_add(global_index as usize)?;
        if position >= end {
            return None;
        }
        let local_index = *self.action_global_to_local_index.get(position)?;
        if local_index < 0 {
            return None;
        }
        let raw_frequency = *action.frequency_x10000.get(local_index as usize)?;
        let hand_ev_x10000 = *action.hand_ev_x10000.get(local_index as usize)?;
        let hand_ev_is_null = raw_frequency == NULL_EV_FREQUENCY_SENTINEL;
        Some(DecodedActionValue {
            frequency_x10000: if hand_ev_is_null { 0 } else { raw_frequency },
            hand_ev_x10000,
            hand_ev_is_null,
        })
    }
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
        return Err(invalid_strategy(format!(
            "{name} must contain {expected_bytes} bytes, got {}",
            bitmap.len()
        )));
    }
    if !total_count.is_multiple_of(8) && !bitmap.is_empty() {
        let padding_mask = !((1u8 << (total_count % 8)) - 1);
        if bitmap[expected_bytes - 1] & padding_mask != 0 {
            return Err(invalid_strategy(format!(
                "{name} has non-zero padding bits"
            )));
        }
    }
    Ok(())
}

fn build_compact_index_map(bitmap: &[u8], total_count: usize) -> Vec<i16> {
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

fn all_set_prefix_bitmap(bit_count: usize) -> Vec<u8> {
    let mut bitmap = vec![0xff; bit_count.div_ceil(8)];
    if !bit_count.is_multiple_of(8) {
        bitmap[bit_count / 8] = (1u8 << (bit_count % 8)) - 1;
    }
    bitmap
}

fn bit_is_set(bitmap: &[u8], index: usize) -> bool {
    bitmap[index / 8] & (1u8 << (index % 8)) != 0
}

fn set_bit(bitmap: &mut [u8], index: usize) {
    bitmap[index / 8] |= 1u8 << (index % 8);
}

fn count_bits(bitmap: &[u8]) -> usize {
    bitmap.iter().map(|byte| byte.count_ones() as usize).sum()
}

fn invalid_strategy(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_V3_HAND_STRATEGY", message)
}
