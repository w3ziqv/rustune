use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

#[test]
fn version_exchange_precedes_preparation_even_without_credentials() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustune-audio"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let hello = br#"{"command":"hello","args":{"version":2}}"#;
    input
        .write_all(&(hello.len() as u32).to_be_bytes())
        .unwrap();
    input.write_all(hello).unwrap();
    drop(input);
    let mut output = child.stdout.take().unwrap();
    let mut header = [0; 4];
    output.read_exact(&mut header).unwrap();
    let size = u32::from_be_bytes(header) as usize;
    assert!(size <= 4096);
    let mut frame = vec![0; size];
    output.read_exact(&mut frame).unwrap();
    child.wait().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&frame).unwrap(),
        serde_json::json!({"event":"hello","data":{"version":2}})
    );
}
