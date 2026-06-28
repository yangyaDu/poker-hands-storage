use range_store_core::hole_cards::{parse_hole_cards, HandDictError};

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
