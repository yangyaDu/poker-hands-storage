const RANKS: [char; 13] = [
    'A', 'K', 'Q', 'J', 'T', '9', '8', '7', '6', '5', '4', '3', '2',
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHand {
    pub input: String,
    pub hand_code: String,
    pub hand_id: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandDictError {
    UnknownHand(String),
    InvalidCardFormat(String),
    DuplicateCard(String),
}

pub fn parse_hole_cards(input: &str) -> Result<ParsedHand, HandDictError> {
    let trimmed = input.trim();
    if let Some((hand_code, hand_id)) = parse_standard_hand_code(trimmed) {
        return Ok(ParsedHand {
            input: input.to_owned(),
            hand_code,
            hand_id,
        });
    }

    let (hand_code, hand_id) = parse_two_cards(trimmed)?;
    Ok(ParsedHand {
        input: input.to_owned(),
        hand_code,
        hand_id,
    })
}

pub fn get_hand_id(hand_code: &str) -> Result<u8, HandDictError> {
    parse_standard_hand_code(hand_code)
        .map(|(_, hand_id)| hand_id)
        .ok_or_else(|| HandDictError::UnknownHand(hand_code.to_owned()))
}

fn parse_standard_hand_code(value: &str) -> Option<(String, u8)> {
    let chars: Vec<char> = value.chars().collect();
    match chars.as_slice() {
        [left, right] => {
            let left_idx = rank_index(*left)?;
            let right_idx = rank_index(*right)?;
            if left_idx != right_idx {
                return None;
            }
            let code = format!("{left}{right}");
            Some((code, hand_id_for_pair(left_idx)))
        }
        [high, low, suffix] => {
            let high_idx = rank_index(*high)?;
            let low_idx = rank_index(*low)?;
            if high_idx >= low_idx {
                return None;
            }
            match *suffix {
                's' | 'S' => {
                    let code = format!("{high}{low}s");
                    Some((code, hand_id_for_suited(high_idx, low_idx)))
                }
                'o' | 'O' => {
                    let code = format!("{high}{low}o");
                    Some((code, hand_id_for_offsuit(high_idx, low_idx)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_two_cards(value: &str) -> Result<(String, u8), HandDictError> {
    let chars: Vec<char> = value.chars().collect();
    let [rank_a, suit_a, rank_b, suit_b] = chars.as_slice() else {
        return Err(HandDictError::InvalidCardFormat(value.to_owned()));
    };

    let suit_a = normalize_suit(*suit_a)
        .ok_or_else(|| HandDictError::InvalidCardFormat(value.to_owned()))?;
    let suit_b = normalize_suit(*suit_b)
        .ok_or_else(|| HandDictError::InvalidCardFormat(value.to_owned()))?;
    let rank_a_idx =
        rank_index(*rank_a).ok_or_else(|| HandDictError::InvalidCardFormat(value.to_owned()))?;
    let rank_b_idx =
        rank_index(*rank_b).ok_or_else(|| HandDictError::InvalidCardFormat(value.to_owned()))?;

    if rank_a_idx == rank_b_idx && suit_a == suit_b {
        return Err(HandDictError::DuplicateCard(value.to_owned()));
    }

    if rank_a_idx == rank_b_idx {
        let rank = RANKS[rank_a_idx];
        return Ok((format!("{rank}{rank}"), hand_id_for_pair(rank_a_idx)));
    }

    let (high_idx, low_idx) = if rank_a_idx < rank_b_idx {
        (rank_a_idx, rank_b_idx)
    } else {
        (rank_b_idx, rank_a_idx)
    };
    let high = RANKS[high_idx];
    let low = RANKS[low_idx];

    if suit_a == suit_b {
        Ok((
            format!("{high}{low}s"),
            hand_id_for_suited(high_idx, low_idx),
        ))
    } else {
        Ok((
            format!("{high}{low}o"),
            hand_id_for_offsuit(high_idx, low_idx),
        ))
    }
}

fn rank_index(rank: char) -> Option<usize> {
    let rank = rank.to_ascii_uppercase();
    RANKS.iter().position(|candidate| *candidate == rank)
}

fn normalize_suit(suit: char) -> Option<char> {
    match suit.to_ascii_lowercase() {
        's' | 'h' | 'd' | 'c' => Some(suit.to_ascii_lowercase()),
        _ => None,
    }
}

fn hand_id_for_pair(rank_idx: usize) -> u8 {
    (rank_idx * 13 + rank_idx) as u8
}

fn hand_id_for_suited(high_idx: usize, low_idx: usize) -> u8 {
    debug_assert!(high_idx < low_idx);
    (high_idx * 13 + low_idx) as u8
}

fn hand_id_for_offsuit(high_idx: usize, low_idx: usize) -> u8 {
    debug_assert!(high_idx < low_idx);
    (low_idx * 13 + high_idx) as u8
}

impl std::fmt::Display for HandDictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHand(value) => write!(f, "Unknown hole cards: {value}"),
            Self::InvalidCardFormat(value) => write!(f, "Invalid card format: {value}"),
            Self::DuplicateCard(value) => write!(f, "Duplicate card: {value}"),
        }
    }
}

impl std::error::Error for HandDictError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_existing_169_hand_codes() {
        assert_eq!(parse_hole_cards("AA").unwrap().hand_id, 0);
        assert_eq!(parse_hole_cards("AKs").unwrap().hand_id, 1);
        assert_eq!(parse_hole_cards("AKo").unwrap().hand_id, 13);
        assert_eq!(parse_hole_cards("22").unwrap().hand_id, 168);
    }

    #[test]
    fn normalizes_two_card_inputs() {
        let offsuit = parse_hole_cards("AsKh").unwrap();
        assert_eq!(offsuit.hand_code, "AKo");
        assert_eq!(offsuit.hand_id, 13);

        let reversed = parse_hole_cards("KhAs").unwrap();
        assert_eq!(reversed.hand_code, "AKo");
        assert_eq!(reversed.hand_id, 13);

        let suited = parse_hole_cards("asKs").unwrap();
        assert_eq!(suited.hand_code, "AKs");
        assert_eq!(suited.hand_id, 1);

        let pair = parse_hole_cards("AcAd").unwrap();
        assert_eq!(pair.hand_code, "AA");
        assert_eq!(pair.hand_id, 0);
    }

    #[test]
    fn rejects_invalid_hands() {
        assert!(parse_hole_cards("AX").is_err());
        assert!(parse_hole_cards("AsXh").is_err());
        assert!(matches!(
            parse_hole_cards("AsAs"),
            Err(HandDictError::DuplicateCard(_))
        ));
    }
}
