use std::path::Path;

use range_store_core::sqlite::{Connection, Value};

#[test]
fn opens_in_memory_database() {
    let connection = Connection::open(Path::new(":memory:"), false).unwrap();
    connection
        .exec("CREATE TABLE test(id INTEGER PRIMARY KEY, value TEXT)")
        .unwrap();
    connection
        .execute("INSERT INTO test(value) VALUES (?1)", &[Value::from("ok")])
        .unwrap();
    let mut statement = connection.prepare("SELECT id, value FROM test").unwrap();
    statement.start(&[]).unwrap();
    assert!(statement.step_row().unwrap());
    assert_eq!(statement.column_i64(0), 1);
    assert_eq!(statement.column_text(1).unwrap(), "ok");
    assert!(!statement.step_row().unwrap());
}
