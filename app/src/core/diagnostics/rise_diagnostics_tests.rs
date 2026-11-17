use std::path::PathBuf;

use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;

use super::{MAX_STORED_BYTES, MAX_STORED_PAYLOADS, RiseDiagnostics, diagnostics_formatter};

fn scratch_directory() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("rise-diagnostics-{}", Uuid::new_v4().uuid_string()));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

#[test]
fn formatter_writes_colon_free_utc_basic_form() {
    let date = Date::from_ms(1_793_886_545_000);
    assert_eq!(diagnostics_formatter(date), "20261105T134905");
}

#[test]
fn prune_keeps_the_newest_payloads_under_the_count_ceiling() {
    let directory = scratch_directory();
    for index in 0..MAX_STORED_PAYLOADS + 3 {
        std::fs::write(directory.join(format!("{index:03}.json")), b"{}").expect("payload");
    }
    RiseDiagnostics::prune(&directory);
    assert_eq!(RiseDiagnostics::stored_files(&directory).len(), MAX_STORED_PAYLOADS);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn prune_evicts_against_the_byte_ceiling() {
    let directory = scratch_directory();
    let half_and_more = usize::try_from(MAX_STORED_BYTES / 2 + 1).expect("fits");
    std::fs::write(directory.join("a.json"), vec![b' '; half_and_more]).expect("payload");
    std::fs::write(directory.join("b.json"), vec![b' '; half_and_more]).expect("payload");
    RiseDiagnostics::prune(&directory);
    assert_eq!(RiseDiagnostics::stored_files(&directory).len(), 1);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn stored_files_skip_subdirectories_and_other_extensions() {
    let directory = scratch_directory();
    std::fs::create_dir_all(directory.join("AppPerformance.json")).expect("tracer dir");
    std::fs::write(directory.join("a.json.tmp"), b"{}").expect("staging");
    std::fs::write(directory.join(".hidden.json"), b"{}").expect("hidden");
    std::fs::write(directory.join("kept.json"), b"{}").expect("payload");
    let files = RiseDiagnostics::stored_files(&directory);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].url, directory.join("kept.json"));
    let _ = std::fs::remove_dir_all(&directory);
}
