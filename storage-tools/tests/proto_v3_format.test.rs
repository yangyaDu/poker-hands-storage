use poker_hands_storage_tools::proto_range_storage::v3::format::{
    decode_hash_locator, decode_header, decode_payload_locator, decode_section_descriptor,
    encode_hash_locator, encode_header, encode_payload_locator, encode_section_descriptor,
    equal_hash_range, stable_hash64, FileHeader, FileKind, HashLocator, PayloadLocator,
    SectionDescriptor, SectionKind, HASH_LOCATOR_SIZE, HEADER_SIZE, NO_VALUE_INDEX,
    PAYLOAD_LOCATOR_SIZE, SECTION_DESCRIPTOR_SIZE,
};
use poker_hands_storage_tools::proto_range_storage::v3::manifest::{
    AbstractActionPathsManifest, ArchiveManifest, DrillScenariosManifest, HandStrategiesManifest,
    ManifestFile, ARCHIVE_FORMAT, ARCHIVE_VERSION, PAYLOAD_SCHEMA, PREFLOP_HAND_ENCODING,
};
use poker_hands_storage_tools::proto_range_storage::v3::proto::{
    AbstractActionPathEntry, AbstractActionPathPage, ActionStrategyColumn, ActionType,
    ConcreteActionPathRef, DrillScenarioEntry, DrillScenarioPage, HandEncoding, HandStrategy,
};
use prost::Message;

#[test]
fn v3_proto_messages_round_trip() {
    let drill_page = DrillScenarioPage {
        entries: vec![DrillScenarioEntry {
            drill_name: "rfi".to_owned(),
            abstract_action_paths: vec!["F-F-R".to_owned()],
        }],
    };
    assert_eq!(
        DrillScenarioPage::decode(drill_page.encode_to_vec().as_slice()).unwrap(),
        drill_page
    );

    let action_path_page = AbstractActionPathPage {
        entries: vec![AbstractActionPathEntry {
            abstract_action_path: "F-F-R".to_owned(),
            concrete_action_paths: vec![ConcreteActionPathRef {
                concrete_action_path_id: 1,
                concrete_action_path: "F-F-R2.5".to_owned(),
            }],
        }],
    };
    assert_eq!(
        AbstractActionPathPage::decode(action_path_page.encode_to_vec().as_slice()).unwrap(),
        action_path_page
    );

    let strategy = HandStrategy {
        schema_version: 3,
        hand_encoding: HandEncoding::Preflop as i32,
        actions: vec![ActionStrategyColumn {
            action_type: ActionType::Raise as i32,
            amount_centi_bb: 250,
            action_size_x10000: 25_000,
            frequency_x10000: vec![10_000, 20_000],
            hand_ev_x10000: vec![12_345, 0],
            action_hand_bitmap: vec![0b0000_0011],
        }],
        available_hand_bitmap: vec![0b0000_0011],
    };
    assert_eq!(
        HandStrategy::decode(strategy.encode_to_vec().as_slice()).unwrap(),
        strategy
    );
}

#[test]
fn v3_header_has_stable_golden_bytes() {
    let header = FileHeader::new(FileKind::DrillScenariosIndex, 3, 2, 1);
    let encoded = encode_header(header);
    assert_eq!(encoded.len(), HEADER_SIZE);
    assert_eq!(
        encoded,
        [
            b'V', b'3', b'D', b'I', 3, 0, 32, 0, 3, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1,
            0, 0, 0, 0, 0, 0, 0,
        ]
    );
    assert_eq!(
        decode_header(&encoded, FileKind::DrillScenariosIndex).unwrap(),
        header
    );
}

#[test]
fn v3_header_rejects_truncation_magic_version_and_data_sections() {
    let header = FileHeader::new(FileKind::HandStrategiesData, 1, 0, 0);
    let encoded = encode_header(header);
    assert!(decode_header(&encoded[..HEADER_SIZE - 1], header.kind).is_err());
    assert!(decode_header(&encoded, FileKind::HandStrategiesIndex).is_err());

    let mut wrong_version = encoded;
    wrong_version[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert!(decode_header(&wrong_version, header.kind).is_err());

    let data_with_sections = encode_header(FileHeader::new(header.kind, 1, 0, 1));
    assert!(decode_header(&data_with_sections, header.kind).is_err());
}

#[test]
fn v3_fixed_width_records_round_trip_and_validate_reserved_bytes() {
    let section = SectionDescriptor::new(
        SectionKind::PrimaryHashLocators,
        HASH_LOCATOR_SIZE as u16,
        128,
        3,
    )
    .unwrap();
    let encoded_section = encode_section_descriptor(section);
    assert_eq!(encoded_section.len(), SECTION_DESCRIPTOR_SIZE);
    assert_eq!(
        decode_section_descriptor(&encoded_section).unwrap(),
        section
    );

    let payload = PayloadLocator {
        offset: 4096,
        byte_length: 123,
        crc32c: 0x1234_5678,
    };
    let encoded_payload = encode_payload_locator(payload);
    assert_eq!(encoded_payload.len(), PAYLOAD_LOCATOR_SIZE);
    assert_eq!(decode_payload_locator(&encoded_payload).unwrap(), payload);

    let hash = HashLocator::value(42, 3, 4, 5);
    let encoded_hash = encode_hash_locator(hash);
    assert_eq!(encoded_hash.len(), HASH_LOCATOR_SIZE);
    assert_eq!(decode_hash_locator(&encoded_hash).unwrap(), hash);

    let mut invalid_reserved = encoded_hash;
    invalid_reserved[20] = 1;
    assert!(decode_hash_locator(&invalid_reserved).is_err());
}

#[test]
fn stable_hash_and_collision_ranges_are_deterministic() {
    assert_eq!(stable_hash64(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(stable_hash64("hello"), 0xa430_d846_80aa_bd0b);

    let locators = vec![
        HashLocator::entry(10, 1, 0),
        HashLocator::entry(20, 1, 1),
        HashLocator::value(20, 2, 0, 3),
        HashLocator::entry(30, 2, 1),
    ];
    assert_eq!(equal_hash_range(&locators, 20), 1..3);
    assert_eq!(equal_hash_range(&locators, 25), 3..3);
    assert_eq!(locators[0].value_index, NO_VALUE_INDEX);
}

#[test]
fn v3_manifest_round_trips_and_enforces_cross_dataset_counts() {
    let manifest = fixture_manifest();
    manifest.validate().unwrap();
    let json = serde_json::to_vec_pretty(&manifest).unwrap();
    let decoded: ArchiveManifest = serde_json::from_slice(&json).unwrap();
    assert_eq!(decoded, manifest);
    decoded.validate().unwrap();

    let mut mismatched = manifest;
    mismatched.hand_strategies.record_count = 3;
    assert_eq!(
        mismatched.validate().unwrap_err().code(),
        "INVALID_V3_MANIFEST"
    );
}

fn fixture_manifest() -> ArchiveManifest {
    ArchiveManifest {
        format: ARCHIVE_FORMAT.to_owned(),
        version: ARCHIVE_VERSION,
        payload_schema: PAYLOAD_SCHEMA.to_owned(),
        strategy: "default".to_owned(),
        player_count: 9,
        depth_bb: 200,
        hand_encoding: PREFLOP_HAND_ENCODING.to_owned(),
        complete: true,
        drill_scenarios: DrillScenariosManifest {
            data: manifest_file("drill-scenarios.pb", 1, 1),
            index: manifest_file("drill-scenarios.idx", 1, 1),
            page_count: 1,
            drill_count: 1,
            hash_record_count: 1,
        },
        abstract_action_paths: AbstractActionPathsManifest {
            data: manifest_file("abstract-action-paths.pb", 1, 1),
            index: manifest_file("abstract-action-paths.idx", 1, 3),
            page_count: 1,
            abstract_path_count: 1,
            concrete_path_count: 2,
            abstract_hash_record_count: 1,
            concrete_hash_record_count: 2,
        },
        hand_strategies: HandStrategiesManifest {
            data: manifest_file("hand-strategies.pb", 2, 0),
            index: manifest_file("hand-strategies.idx", 2, 0),
            record_count: 2,
        },
    }
}

fn manifest_file(file_name: &str, primary_count: u64, secondary_count: u64) -> ManifestFile {
    ManifestFile {
        file_name: file_name.to_owned(),
        size_bytes: 128,
        crc32c: 123,
        primary_count,
        secondary_count,
    }
}
