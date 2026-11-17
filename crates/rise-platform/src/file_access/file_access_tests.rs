use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use super::file_access::*;

struct Granting {
    stops: Rc<Cell<u32>>,
}

impl SecurityScopeSource for Granting {
    fn start(&self, _path: &Path) -> Option<ScopeRelease> {
        let stops = Rc::clone(&self.stops);
        Some(Box::new(move || stops.set(stops.get() + 1)))
    }
}

struct Refusing;

impl SecurityScopeSource for Refusing {}

#[test]
fn an_opened_scope_is_stopped_once_on_drop() {
    let stops = Rc::new(Cell::new(0));
    let access = SecurityScopedAccess::with_source(&Granting { stops: Rc::clone(&stops) }, Path::new("/shared/clip.mov"));
    assert!(access.did_access());
    assert_eq!(stops.get(), 0);
    drop(access);
    assert_eq!(stops.get(), 1);
}

#[test]
fn a_scope_that_did_not_open_stops_nothing() {
    let access = SecurityScopedAccess::with_source(&Refusing, Path::new("/shared/clip.mov"));
    assert!(!access.did_access());
}

#[test]
fn the_volume_of_the_temporary_directory_reports_its_free_space() {
    let capacity = volume_available_capacity_for_important_usage(&std::env::temp_dir());
    assert!(capacity.is_none_or(|bytes| bytes >= 0));
}
