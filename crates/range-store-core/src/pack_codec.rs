//! Range pack decoder — the hot-path.
//!
//! Decodes a subset of a range pack for a single target hand_id.
//! This is the equivalent of `decodeRangePackForHandDirect` in the JS codebase.
//!
//! Pack layout per hand (all little-endian):
//!   hand_id    : u8  (1 byte)
//!   action_mask: u32 (4 bytes, bit N set = action N exists)
//!   cells[action_count]: f32 frequency + f32 EV  (8 bytes per action)
//!
//! Total pack size = hand_count * (5 + action_count * 8) bytes.

use crate::types::DecodedCellResult;

/// Compute the total byte length of a range pack.
#[inline]
pub fn pack_byte_length(hand_count: u16, action_count: u16) -> u32 {
    hand_count as u32 * (5u32 + action_count as u32 * 8u32)
}

/// Decode the cells for a specific hand_id from a raw pack slice.
///
/// `action_count` is derived from the pack byte length:
///   action_count = (byte_length / hand_count - 5) / 8
///
/// Returns an empty Vec if the hand_id is not found in the pack.
///
/// # Safety
///
/// Caller must ensure `pack` references valid memory for the entire pack.
/// This function performs bounds checks via slicing.
pub fn decode_pack_for_hand(
    pack: &[u8],
    hand_count: u16,
    action_count: u16,
    target_hand_id: u8,
) -> Vec<DecodedCellResult> {
    let hand_count = hand_count as usize;
    let action_count_u = action_count as usize;

    // Validate pack length
    let expected_len = hand_count * (5 + action_count_u * 8);
    if pack.len() != expected_len {
        return Vec::new();
    }

    if action_count_u == 0 {
        return Vec::new();
    }

    // Step 1: binary search hand_id in the hand_ids segment (sorted ascending)
    let hand_ids = &pack[0..hand_count];
    let hand_idx = match binary_search_u8(hand_ids, target_hand_id) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    // Step 2: read action_mask for this hand
    let mask_offset = hand_count + hand_idx * 4;
    let mask_bytes = &pack[mask_offset..mask_offset + 4];
    let mask = u32::from_le_bytes([mask_bytes[0], mask_bytes[1], mask_bytes[2], mask_bytes[3]]);

    // Step 3: read cell data for this hand
    // cells start after hand_ids (hand_count) + action_masks (hand_count * 4)
    let cells_start = hand_count + hand_count * 4;
    let floats_per_hand = action_count_u * 2;
    let cell_offset = cells_start + hand_idx * floats_per_hand * 4;
    let cell_data = &pack[cell_offset..cell_offset + floats_per_hand * 4];

    // Decode cells — only emit cells where mask bit is 1
    let mut result = Vec::with_capacity(action_count_u);
    for action_id in 0..action_count_u {
        if (mask >> action_id) & 1 == 0 {
            continue;
        }

        let cell_base = action_id * 8;
        let freq = f32::from_le_bytes([
            cell_data[cell_base],
            cell_data[cell_base + 1],
            cell_data[cell_base + 2],
            cell_data[cell_base + 3],
        ]);
        let ev_raw = f32::from_le_bytes([
            cell_data[cell_base + 4],
            cell_data[cell_base + 5],
            cell_data[cell_base + 6],
            cell_data[cell_base + 7],
        ]);

        result.push(DecodedCellResult {
            action_id: action_id as u32,
            frequency: freq as f64,
            hand_ev: if ev_raw.is_nan() {
                None
            } else {
                Some(ev_raw as f64)
            },
        });
    }

    result
}

/// Compute action_count from hand_count and pack byte_length.
#[inline]
pub fn action_count_from_pack(hand_count: u16, byte_length: u32) -> u16 {
    // byte_length = hand_count * (5 + action_count * 8)
    // action_count = (byte_length / hand_count - 5) / 8
    if hand_count == 0 {
        return 0;
    }
    ((byte_length / hand_count as u32).saturating_sub(5) / 8) as u16
}

/// Binary search for `target` in a sorted `u8` slice.
#[inline]
fn binary_search_u8(slice: &[u8], target: u8) -> Option<usize> {
    let mut lo: usize = 0;
    let mut hi: usize = slice.len();

    while lo < hi {
        let mid = (lo + hi) / 2;
        let val = slice[mid];
        if val < target {
            lo = mid + 1;
        } else if val > target {
            hi = mid;
        } else {
            return Some(mid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_pack(hand_ids: &[u8], action_count: u16, data: &[f32]) -> Vec<u8> {
        let hand_count = hand_ids.len();
        let total_actions = hand_count * action_count as usize;
        assert_eq!(data.len(), total_actions * 2);

        let byte_len = hand_count * (5 + action_count as usize * 8);
        let mut buf = vec![0u8; byte_len];
        let mut cursor = 0;

        // hand_ids
        for &id in hand_ids {
            buf[cursor] = id;
            cursor += 1;
        }

        // action_masks: assume all actions exist (all ones)
        let full_mask: u32 = if action_count == 0 {
            0
        } else if action_count >= 32 {
            u32::MAX
        } else {
            (1u32 << action_count) - 1
        };
        for _ in 0..hand_count {
            buf[cursor..cursor + 4].copy_from_slice(&full_mask.to_le_bytes());
            cursor += 4;
        }

        // cell data
        for chunk in data.chunks(action_count as usize * 2) {
            for cell in chunk.chunks(2) {
                buf[cursor..cursor + 4].copy_from_slice(&cell[0].to_le_bytes());
                cursor += 4;
                buf[cursor..cursor + 4].copy_from_slice(&cell[1].to_le_bytes());
                cursor += 4;
            }
        }

        buf
    }

    #[test]
    fn test_decode_simple() {
        // 2 hands (id=0, id=1), 2 actions each
        // hand0: action0=(freq=0.5, ev=1.0), action1=(freq=0.3, ev=2.0)
        // hand1: action0=(freq=0.7, ev=3.0), action1=(freq=0.1, ev=4.0)
        let data = vec![
            0.5_f32, 1.0, 0.3, 2.0, // hand 0
            0.7, 3.0, 0.1, 4.0, // hand 1
        ];

        let pack = make_test_pack(&[0, 1], 2, &data);

        let cells = decode_pack_for_hand(&pack, 2, 2, 0);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].action_id, 0);
        assert!((cells[0].frequency - 0.5).abs() < 0.001);
        assert!((cells[0].hand_ev.unwrap() - 1.0).abs() < 0.001);

        assert_eq!(cells[1].action_id, 1);
        assert!((cells[1].frequency - 0.3).abs() < 0.001);
        assert!((cells[1].hand_ev.unwrap() - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_hand_not_found() {
        let data = vec![0.5_f32, 1.0];
        let pack = make_test_pack(&[10], 1, &data);
        let cells = decode_pack_for_hand(&pack, 1, 1, 99);
        assert!(cells.is_empty());
    }

    #[test]
    fn test_nan_ev_decodes_to_none() {
        let data = vec![0.5_f32, f32::NAN, 0.25, 0.0];
        let pack = make_test_pack(&[0], 2, &data);

        let cells = decode_pack_for_hand(&pack, 1, 2, 0);

        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].hand_ev, None);
        assert_eq!(cells[1].hand_ev, Some(0.0));
    }

    #[test]
    fn test_max_pack_layout_decodes() {
        let hand_ids: Vec<u8> = (0..169).map(|id| id as u8).collect();
        let action_count = 32;
        let mut data = Vec::with_capacity(hand_ids.len() * action_count as usize * 2);
        for _hand_id in &hand_ids {
            for action_id in 0..action_count {
                data.push(action_id as f32 / 100.0);
                data.push(action_id as f32);
            }
        }
        let pack = make_test_pack(&hand_ids, action_count, &data);

        assert_eq!(pack.len(), pack_byte_length(169, action_count) as usize);

        let cells = decode_pack_for_hand(&pack, 169, action_count, 168);

        assert_eq!(cells.len(), action_count as usize);
        assert_eq!(cells[31].action_id, 31);
        assert!((cells[31].frequency - 0.31).abs() < 0.001);
        assert_eq!(cells[31].hand_ev, Some(31.0));
    }

    #[test]
    fn test_binary_search_u8() {
        assert_eq!(binary_search_u8(&[], 5), None);
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 5), Some(2));
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 1), Some(0));
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 9), Some(4));
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 4), None);
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 0), None);
        assert_eq!(binary_search_u8(&[1, 3, 5, 7, 9], 10), None);
    }

    #[test]
    fn test_action_count_from_pack() {
        assert_eq!(action_count_from_pack(100, 500), 0); // 500/100-5=0, /8=0
        assert_eq!(action_count_from_pack(100, 1300), 1); // 1300/100-5=8, /8=1
        assert_eq!(action_count_from_pack(169, 845), 0); // too small
        assert_eq!(action_count_from_pack(169, (169 * (5 + 10 * 8)) as u32), 10);
    }

    #[test]
    fn test_pack_byte_length() {
        assert_eq!(pack_byte_length(100, 0), 500); // 100 * 5
        assert_eq!(pack_byte_length(100, 1), 1300); // 100 * 13
        assert_eq!(pack_byte_length(169, 10), 169 * 85); // 169 * (5+80)
    }
}
