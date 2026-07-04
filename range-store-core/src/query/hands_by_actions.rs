use crate::action_schema::{ActionDef, ActionName};
use crate::hole_cards::hand_code_from_id;
use crate::types::DecodedPack;

use super::store_query_service::DEFAULT_HANDS_BY_ACTIONS_FREQUENCY;

/// Parsed action filter for hands-by-actions queries.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionFilter {
    pub raw: String,
    pub action_name: ActionName,
    pub amount_bb: Option<f32>,
}

/// Parsed frequency threshold. Matching is always strict greater-than.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrequencyFilter {
    threshold: f64,
}

/// Error kinds returned when parsing an action filter string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionFilterParseError {
    /// The string does not start with any recognized action name.
    UnknownAction,
    /// A no-amount action (fold/check/call) has a trailing suffix.
    UnexpectedSuffix,
    /// An amount-bearing action has an invalid numeric suffix.
    InvalidAmount,
}

impl FrequencyFilter {
    pub fn from_request(frequency: Option<f64>) -> Self {
        Self {
            threshold: frequency.unwrap_or(DEFAULT_HANDS_BY_ACTIONS_FREQUENCY),
        }
    }

    pub fn matches(&self, value: f64) -> bool {
        value > self.threshold
    }

    pub fn description(&self) -> String {
        if self.threshold == 0.0 {
            ">0".to_owned()
        } else {
            format!(">{}", self.threshold)
        }
    }
}

impl std::fmt::Display for ActionFilterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionFilterParseError::UnknownAction => {
                write!(f, "must be one of fold, check, call, bet, raise, allin")
            }
            ActionFilterParseError::UnexpectedSuffix => {
                write!(f, "must not have a numeric suffix")
            }
            ActionFilterParseError::InvalidAmount => {
                write!(f, "must have a valid numeric suffix (e.g. bet2.5)")
            }
        }
    }
}

impl std::error::Error for ActionFilterParseError {}

/// Parse a string like "raise2.5" or "call" into an [`ActionFilter`].
pub fn parse_action_filter(raw: &str) -> Result<ActionFilter, ActionFilterParseError> {
    // Known action names in descending length order to avoid ambiguous prefixes.
    const NAMES: &[ActionName] = &[
        ActionName::Allin,
        ActionName::Check,
        ActionName::Raise,
        ActionName::Fold,
        ActionName::Call,
        ActionName::Bet,
    ];

    for &name in NAMES {
        let prefix = name.as_str();
        if let Some(remainder) = raw.strip_prefix(prefix) {
            let amount = match name {
                ActionName::Fold | ActionName::Call | ActionName::Check => {
                    if !remainder.is_empty() {
                        return Err(ActionFilterParseError::UnexpectedSuffix);
                    }
                    None
                }
                ActionName::Bet | ActionName::Raise | ActionName::Allin => {
                    if remainder.is_empty() {
                        None
                    } else {
                        let amount: f32 = remainder
                            .parse()
                            .map_err(|_| ActionFilterParseError::InvalidAmount)?;
                        if !amount.is_finite() {
                            return Err(ActionFilterParseError::InvalidAmount);
                        }
                        Some(amount)
                    }
                }
            };
            return Ok(ActionFilter {
                raw: raw.to_owned(),
                action_name: name,
                amount_bb: amount,
            });
        }
    }

    Err(ActionFilterParseError::UnknownAction)
}

/// Parse a list of raw action filter strings.
pub fn parse_action_filters(
    raw_filters: Vec<String>,
) -> Result<Vec<ActionFilter>, ActionFilterParseError> {
    raw_filters
        .into_iter()
        .map(|raw| parse_action_filter(&raw))
        .collect()
}

pub fn format_action_filters(filters: &[ActionFilter]) -> String {
    if filters.is_empty() {
        "[]".to_owned()
    } else {
        filters
            .iter()
            .map(|filter| filter.raw.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Return matching 169-hand codes for a decoded pack.
///
/// Empty `filters` means all hands with at least one existing action above the
/// frequency threshold. Non-empty filters use OR semantics: any requested
/// filter can include the hand.
pub fn match_hands_by_actions(
    pack: DecodedPack,
    action_schema: &[ActionDef],
    filters: &[ActionFilter],
    frequency_filter: &FrequencyFilter,
) -> Vec<String> {
    let filter_mask = resolve_action_filter_mask(action_schema, filters);

    let mut hand_masks = [0u32; 169];
    for cell in &pack.cells {
        if !cell.exists || !frequency_filter.matches(cell.frequency) || cell.action_id >= 32 {
            continue;
        }
        hand_masks[cell.hand_id as usize] |= 1u32 << cell.action_id;
    }

    pack.hand_ids
        .into_iter()
        .filter(|hand_id| {
            let hand_mask = hand_masks[*hand_id as usize];
            if filters.is_empty() {
                hand_mask != 0
            } else {
                hand_mask & filter_mask != 0
            }
        })
        .map(hand_code_from_id)
        .collect()
}

fn resolve_action_filter_mask(action_schema: &[ActionDef], filters: &[ActionFilter]) -> u32 {
    filters
        .iter()
        .flat_map(|filter| {
            action_schema.iter().filter(move |action| {
                action.action_id < 32 && action_matches_filter(action, filter)
            })
        })
        .fold(0u32, |mask, action| mask | (1u32 << action.action_id))
}

fn action_matches_filter(action: &ActionDef, filter: &ActionFilter) -> bool {
    action.action_name == filter.action_name
        && match filter.amount_bb {
            Some(amount_bb) => (action.amount_bb - amount_bb).abs() <= f32::EPSILON,
            None => true,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DecodedPackCell;

    fn action(action_id: u32, action_name: ActionName, amount_bb: f32) -> ActionDef {
        ActionDef {
            action_id,
            action_name,
            action_size: amount_bb,
            amount_bb,
        }
    }

    fn fixture_pack() -> DecodedPack {
        DecodedPack {
            hand_ids: vec![0, 14, 162],
            action_masks: vec![0b11, 0b10, 0b01],
            cells: vec![
                DecodedPackCell {
                    hand_id: 0,
                    action_id: 0,
                    exists: true,
                    frequency: 0.25,
                    hand_ev: None,
                },
                DecodedPackCell {
                    hand_id: 0,
                    action_id: 1,
                    exists: true,
                    frequency: 0.75,
                    hand_ev: Some(1.0),
                },
                DecodedPackCell {
                    hand_id: 14,
                    action_id: 0,
                    exists: false,
                    frequency: 0.0,
                    hand_ev: None,
                },
                DecodedPackCell {
                    hand_id: 14,
                    action_id: 1,
                    exists: true,
                    frequency: 0.6,
                    hand_ev: Some(0.8),
                },
                DecodedPackCell {
                    hand_id: 162,
                    action_id: 0,
                    exists: true,
                    frequency: 0.0,
                    hand_ev: None,
                },
                DecodedPackCell {
                    hand_id: 162,
                    action_id: 1,
                    exists: false,
                    frequency: 0.0,
                    hand_ev: None,
                },
            ],
        }
    }

    #[test]
    fn parses_amount_aware_filters() {
        let raise = parse_action_filter("raise2.5").unwrap();
        assert_eq!(raise.action_name, ActionName::Raise);
        assert_eq!(raise.amount_bb, Some(2.5));

        let call = parse_action_filter("call").unwrap();
        assert_eq!(call.action_name, ActionName::Call);
        assert_eq!(call.amount_bb, None);
    }

    #[test]
    fn rejects_invalid_filters() {
        assert_eq!(
            parse_action_filter("fold123").unwrap_err(),
            ActionFilterParseError::UnexpectedSuffix
        );
        assert_eq!(
            parse_action_filter("raiseabc").unwrap_err(),
            ActionFilterParseError::InvalidAmount
        );
        assert_eq!(
            parse_action_filter("noop").unwrap_err(),
            ActionFilterParseError::UnknownAction
        );
    }

    #[test]
    fn empty_filters_match_any_action_above_threshold() {
        let schema = vec![
            action(0, ActionName::Fold, 0.0),
            action(1, ActionName::Raise, 2.5),
        ];
        let hands = match_hands_by_actions(
            fixture_pack(),
            &schema,
            &[],
            &FrequencyFilter::from_request(None),
        );
        assert_eq!(hands, vec!["AA", "KK"]);
    }

    #[test]
    fn multiple_filters_use_or_semantics() {
        let schema = vec![
            action(0, ActionName::Fold, 0.0),
            action(1, ActionName::Raise, 2.5),
        ];
        let filters = parse_action_filters(vec!["fold".to_owned(), "raise".to_owned()]).unwrap();
        let hands = match_hands_by_actions(
            fixture_pack(),
            &schema,
            &filters,
            &FrequencyFilter::from_request(None),
        );
        assert_eq!(hands, vec!["AA", "KK"]);
    }

    #[test]
    fn absent_filter_does_not_suppress_other_matches() {
        let schema = vec![
            action(0, ActionName::Fold, 0.0),
            action(1, ActionName::Raise, 2.5),
        ];
        let filters = parse_action_filters(vec!["raise".to_owned(), "check".to_owned()]).unwrap();
        let hands = match_hands_by_actions(
            fixture_pack(),
            &schema,
            &filters,
            &FrequencyFilter::from_request(None),
        );
        assert_eq!(hands, vec!["AA", "KK"]);
    }

    #[test]
    fn amount_suffix_requires_exact_amount_match() {
        let schema = vec![
            action(0, ActionName::Fold, 0.0),
            action(1, ActionName::Raise, 2.5),
        ];
        let filters = parse_action_filters(vec!["raise3".to_owned()]).unwrap();
        let hands = match_hands_by_actions(
            fixture_pack(),
            &schema,
            &filters,
            &FrequencyFilter::from_request(None),
        );
        assert!(hands.is_empty());
    }
}
