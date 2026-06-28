use range_store_core::action_schema::{decode_action_blob, ActionName, ActionSchemaError};

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
