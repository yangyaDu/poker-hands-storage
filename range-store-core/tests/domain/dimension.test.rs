use range_store_core::dimension::{
    get_bin_file_name, get_concrete_lines_table_name, get_drill_scenario_table_name,
    get_idx_file_name, quote_identifier,
};

#[test]
fn matches_current_file_naming() {
    assert_eq!(
        get_idx_file_name("default", 6, 100),
        "ranges_default_6max_100BB.idx"
    );
    assert_eq!(
        get_bin_file_name("default", 6, 100),
        "ranges_default_6max_100BB.bin"
    );
}

#[test]
fn matches_current_table_naming() {
    assert_eq!(
        get_drill_scenario_table_name("default"),
        "drill_scenario_lines_default"
    );
    assert_eq!(
        get_concrete_lines_table_name("default", 9, 300),
        "concrete_lines_default_9max_300BB"
    );
}

#[test]
fn quote_identifier_matches_typescript_guardrail() {
    assert_eq!(
        quote_identifier("concrete_lines_default_6max_100BB").unwrap(),
        "\"concrete_lines_default_6max_100BB\""
    );
    assert!(quote_identifier("../escape").is_err());
    assert!(quote_identifier("9bad").is_err());
}
