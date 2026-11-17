use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use tracing_subscriber::fmt::MakeWriter;

use super::{LOG_FILE_REDACTED, RiseLogFile, log_file_path, redact_private_fields};

fn scratch_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("rise-log-file-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn write_line(log_file: &RiseLogFile, line: &str) {
    let mut writer = log_file.make_writer();
    writer.write_all(line.as_bytes()).expect("buffered write");
}

#[test]
fn lines_reach_the_file_after_flush() {
    let directory = scratch_directory("flush");
    let log_file = RiseLogFile::start_in(directory.clone(), 1024, 3).expect("log file");

    write_line(&log_file, "WARN riseonly::X2373: first\n");
    write_line(&log_file, "WARN riseonly::X2373: second\n");
    assert!(log_file.flush(Duration::from_secs(5)));

    let written = std::fs::read_to_string(log_file_path(&directory, 0)).expect("read log");
    assert_eq!(written, "WARN riseonly::X2373: first\nWARN riseonly::X2373: second\n");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_full_file_rotates_and_keeps_only_the_newest_files() {
    let directory = scratch_directory("rotate");
    let log_file = RiseLogFile::start_in(directory.clone(), 16, 3).expect("log file");

    for index in 0..6 {
        write_line(&log_file, &format!("line {index} 123456\n"));
    }
    assert!(log_file.flush(Duration::from_secs(5)));

    assert_eq!(std::fs::read_to_string(log_file_path(&directory, 0)).expect("current"), "line 5 123456\n");
    assert_eq!(std::fs::read_to_string(log_file_path(&directory, 1)).expect("first rotated"), "line 4 123456\n");
    assert_eq!(std::fs::read_to_string(log_file_path(&directory, 2)).expect("second rotated"), "line 3 123456\n");
    assert!(!log_file_path(&directory, 3).exists());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn rotated_names_keep_the_log_extension() {
    let directory = PathBuf::from("logs");
    assert_eq!(log_file_path(&directory, 0), directory.join("riseonly.log"));
    assert_eq!(log_file_path(&directory, 4), directory.join("riseonly.4.log"));
}

#[test]
fn private_field_values_are_hidden() {
    let line = "WARN riseonly: send failed access_token=abc123 phone=\"+7 700 000\" chat_id=42 accessToken=xyz\n";
    assert_eq!(
        redact_private_fields(line),
        format!(
            "WARN riseonly: send failed access_token={LOG_FILE_REDACTED} phone={LOG_FILE_REDACTED} chat_id=42 accessToken={LOG_FILE_REDACTED}\n"
        )
    );
}

#[test]
fn message_texts_are_hidden_but_context_fields_stay() {
    let line = "WARN riseonly::X2373: render text=\"hello there\" context=modal bounds=Bounds { a: 1 }\n";
    assert_eq!(
        redact_private_fields(line),
        format!("WARN riseonly::X2373: render text={LOG_FILE_REDACTED} context=modal bounds=Bounds {{ a: 1 }}\n")
    );
}

#[test]
fn bearer_tokens_are_hidden() {
    let line = "WARN rise_engine: header Authorization: Bearer eyJhbGci.payload rejected\n";
    assert_eq!(
        redact_private_fields(line),
        format!("WARN rise_engine: header Authorization: Bearer {LOG_FILE_REDACTED} rejected\n")
    );
}

#[test]
fn lines_without_fields_are_unchanged() {
    let line = "WARN riseonly: a = b, nothing private\n";
    assert_eq!(redact_private_fields(line), line);
}
