use std::process::Command;

#[test]
fn binary_exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_hive")).status().unwrap();
    assert!(status.success());
}
