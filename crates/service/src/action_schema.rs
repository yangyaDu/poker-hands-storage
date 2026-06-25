#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionName {
    Fold,
    Call,
    Check,
    Bet,
    Raise,
    Allin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionDef {
    pub action_id: u32,
    pub action_name: ActionName,
    pub action_size: f32,
    pub amount_bb: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionSchemaError {
    InvalidLength { expected: usize, got: usize },
    InvalidActionCount(u32),
    UnknownActionType(u8),
}

pub fn decode_action_blob(
    blob: &[u8],
    action_count: u32,
) -> Result<Vec<ActionDef>, ActionSchemaError> {
    if !(1..=32).contains(&action_count) {
        return Err(ActionSchemaError::InvalidActionCount(action_count));
    }

    let expected = action_count as usize * 9;
    if blob.len() != expected {
        return Err(ActionSchemaError::InvalidLength {
            expected,
            got: blob.len(),
        });
    }

    let mut actions = Vec::with_capacity(action_count as usize);
    let mut cursor = 0usize;
    for action_id in 0..action_count {
        let action_type = blob[cursor];
        cursor += 1;
        let action_size = f32::from_le_bytes([
            blob[cursor],
            blob[cursor + 1],
            blob[cursor + 2],
            blob[cursor + 3],
        ]);
        cursor += 4;
        let amount_bb = f32::from_le_bytes([
            blob[cursor],
            blob[cursor + 1],
            blob[cursor + 2],
            blob[cursor + 3],
        ]);
        cursor += 4;

        let action_name = action_name_by_type(action_type)
            .ok_or(ActionSchemaError::UnknownActionType(action_type))?;
        actions.push(ActionDef {
            action_id,
            action_name,
            action_size,
            amount_bb,
        });
    }

    Ok(actions)
}

fn action_name_by_type(action_type: u8) -> Option<ActionName> {
    match action_type {
        0 => Some(ActionName::Fold),
        1 => Some(ActionName::Call),
        2 => Some(ActionName::Check),
        3 => Some(ActionName::Bet),
        4 => Some(ActionName::Raise),
        5 => Some(ActionName::Allin),
        _ => None,
    }
}

impl ActionName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fold => "fold",
            Self::Call => "call",
            Self::Check => "check",
            Self::Bet => "bet",
            Self::Raise => "raise",
            Self::Allin => "allin",
        }
    }
}

impl std::fmt::Display for ActionSchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength { expected, got } => {
                write!(
                    f,
                    "Invalid action schema length: expected {expected}, got {got}"
                )
            }
            Self::InvalidActionCount(count) => {
                write!(f, "Invalid action count: {count}, expected 1..=32")
            }
            Self::UnknownActionType(action_type) => {
                write!(f, "Unknown action type: {action_type}")
            }
        }
    }
}

impl std::error::Error for ActionSchemaError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_action(bytes: &mut Vec<u8>, action_type: u8, action_size: f32, amount_bb: f32) {
        bytes.push(action_type);
        bytes.extend_from_slice(&action_size.to_le_bytes());
        bytes.extend_from_slice(&amount_bb.to_le_bytes());
    }

    #[test]
    fn decodes_action_blob() {
        let mut blob = Vec::new();
        push_action(&mut blob, 0, 0.0, 0.0);
        push_action(&mut blob, 4, 2.5, 2.5);

        let actions = decode_action_blob(&blob, 2).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, 0);
        assert_eq!(actions[0].action_name, ActionName::Fold);
        assert_eq!(actions[1].action_id, 1);
        assert_eq!(actions[1].action_name, ActionName::Raise);
        assert_eq!(actions[1].action_name.as_str(), "raise");
        assert!((actions[1].action_size - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_invalid_length() {
        let err = decode_action_blob(&[0; 8], 1).unwrap_err();
        assert_eq!(
            err,
            ActionSchemaError::InvalidLength {
                expected: 9,
                got: 8
            }
        );
    }

    #[test]
    fn rejects_unknown_action_type() {
        let mut blob = Vec::new();
        push_action(&mut blob, 99, 0.0, 0.0);
        let err = decode_action_blob(&blob, 1).unwrap_err();
        assert_eq!(err, ActionSchemaError::UnknownActionType(99));
    }
}
