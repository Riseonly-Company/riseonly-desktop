use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use crate::contacts::*;
use crate::host_os::HostOs;

/// store answering a fixed status, request_access moves to status_after and answers
struct FakeStore {
    status: Mutex<ContactAuthorization>,
    status_after: ContactAuthorization,
    granted: bool,
    error: Option<ContactsError>,
    change_calls: Mutex<Vec<Vec<u8>>>,
}

impl FakeStore {
    fn leaked(
        status: ContactAuthorization,
        status_after: ContactAuthorization,
        granted: bool,
        error: Option<ContactsError>,
    ) -> &'static Self {
        Box::leak(Box::new(Self {
            status: Mutex::new(status),
            status_after,
            granted,
            error,
            change_calls: Mutex::new(Vec::new()),
        }))
    }
}

impl ContactStore for FakeStore {
    fn authorization_status(&self) -> ContactAuthorization {
        *self.status.lock()
    }

    fn request_access(&self, done: AccessAnswer) -> Result<(), ContactsError> {
        *self.status.lock() = self.status_after;
        done(self.granted, self.error.clone());
        Ok(())
    }

    fn fetch_changes(
        &self,
        starting_token: &[u8],
        _keys: &[ContactKey],
        _excluded_transaction_authors: &[String],
    ) -> Result<ContactChangeResult, ContactsError> {
        self.change_calls.lock().push(starting_token.to_vec());
        Ok(ContactChangeResult {
            changes: vec![ContactChange::Delete { contact_identifier: "a".to_owned() }],
            current_history_token: Some(vec![9]),
        })
    }
}

struct BareStore;

impl ContactStore for BareStore {}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the fake answers synchronously"),
    }
}

fn store_error(code: i64, is_cn_domain: bool) -> ContactsError {
    ContactsError::Store {
        domain: "domain".to_owned(),
        is_cn_domain,
        code,
        description: String::new(),
    }
}

#[test]
fn cn_status_maps_every_raw_value() {
    assert_eq!(ContactAuthorization::from_cn_status(0), ContactAuthorization::NotDetermined);
    assert_eq!(ContactAuthorization::from_cn_status(1), ContactAuthorization::Restricted);
    assert_eq!(ContactAuthorization::from_cn_status(2), ContactAuthorization::Denied);
    assert_eq!(ContactAuthorization::from_cn_status(3), ContactAuthorization::Authorized);
    assert_eq!(ContactAuthorization::from_cn_status(4), ContactAuthorization::Limited);
    assert_eq!(ContactAuthorization::from_cn_status(42), ContactAuthorization::Denied);
    assert_eq!(ContactAuthorization::from_cn_status(-1), ContactAuthorization::Denied);
}

#[test]
fn only_authorized_and_limited_read_and_write() {
    assert!(ContactAuthorization::Authorized.can_read_and_write());
    assert!(ContactAuthorization::Limited.can_read_and_write());
    assert!(!ContactAuthorization::Denied.can_read_and_write());
    assert!(!ContactAuthorization::Restricted.can_read_and_write());
    assert!(!ContactAuthorization::NotDetermined.can_read_and_write());
    assert!(ContactAuthorization::NotDetermined.can_request());
    assert!(!ContactAuthorization::Restricted.can_request());
}

#[test]
fn an_error_counts_only_while_undecided() {
    let error = store_error(100, true);
    assert_eq!(
        access_answer(false, Some(error.clone()), ContactAuthorization::NotDetermined),
        Err(error.clone())
    );
    assert_eq!(access_answer(false, Some(error), ContactAuthorization::Denied), Ok(false));
    assert_eq!(access_answer(true, None, ContactAuthorization::Authorized), Ok(true));
}

#[test]
fn request_authorization_rereads_the_status() {
    let store = FakeStore::leaked(
        ContactAuthorization::NotDetermined,
        ContactAuthorization::Authorized,
        true,
        None,
    );
    assert_eq!(ready(request_authorization_from(store)), Ok(ContactAuthorization::Authorized));
}

#[test]
fn dont_allow_with_an_error_is_an_answer() {
    let store = FakeStore::leaked(
        ContactAuthorization::NotDetermined,
        ContactAuthorization::Denied,
        false,
        Some(store_error(100, true)),
    );
    assert_eq!(ready(request_authorization_from(store)), Ok(ContactAuthorization::Denied));
}

#[test]
fn an_error_before_any_decision_fails_the_request() {
    let error = store_error(1, false);
    let store = FakeStore::leaked(
        ContactAuthorization::NotDetermined,
        ContactAuthorization::NotDetermined,
        false,
        Some(error.clone()),
    );
    assert_eq!(ready(request_authorization_from(store)), Err(error));
}

#[test]
fn an_empty_token_drops_everything_without_the_store() {
    let store = FakeStore::leaked(
        ContactAuthorization::Authorized,
        ContactAuthorization::Authorized,
        true,
        None,
    );
    let result = fetch_changes_from(store, &[], &[ContactKey::Identifier], &[]).expect("answered");
    assert!(result.drops_everything());
    assert_eq!(result.current_history_token, None);
    assert!(store.change_calls.lock().is_empty());
}

#[test]
fn a_token_reaches_the_store() {
    let store = FakeStore::leaked(
        ContactAuthorization::Authorized,
        ContactAuthorization::Authorized,
        true,
        None,
    );
    let result = fetch_changes_from(store, &[1, 2], &[ContactKey::Identifier], &[]).expect("answered");
    assert!(!result.drops_everything());
    assert_eq!(result.current_history_token, Some(vec![9]));
    assert_eq!(*store.change_calls.lock(), vec![vec![1, 2]]);
}

#[test]
fn drop_everything_anywhere_in_the_history_means_a_full_snapshot() {
    let result = ContactChangeResult {
        changes: vec![
            ContactChange::Add(DeviceContact::default()),
            ContactChange::DropEverything,
            ContactChange::Delete { contact_identifier: "b".to_owned() },
        ],
        current_history_token: Some(vec![1]),
    };
    assert!(result.drops_everything());
    assert!(ContactChangeResult::drop_everything().drops_everything());
    assert!(!ContactChangeResult::default().drops_everything());
}

#[test]
fn a_host_without_a_branch_is_restricted_and_unsupported() {
    let store = BareStore;
    assert_eq!(store.authorization_status(), ContactAuthorization::Restricted);
    assert_eq!(store.fetch_all(&[ContactKey::Identifier]), Err(ContactsError::Unsupported(HostOs::current())));
    assert_eq!(store.current_history_token(), None);
    assert_eq!(
        store.execute(&ContactSaveRequest::default()),
        Err(ContactsError::Unsupported(HostOs::current()))
    );
}

#[test]
fn record_does_not_exist_needs_the_contacts_domain() {
    assert!(store_error(CN_ERROR_RECORD_DOES_NOT_EXIST, true).is_record_does_not_exist());
    assert!(!store_error(CN_ERROR_RECORD_DOES_NOT_EXIST, false).is_record_does_not_exist());
    assert!(!store_error(100, true).is_record_does_not_exist());
    assert!(!ContactsError::Unsupported(HostOs::Linux).is_record_does_not_exist());
}

#[test]
fn only_macos_has_an_address_book_and_its_settings_pane() {
    assert!(host_has_address_book(HostOs::MacOs));
    assert!(!host_has_address_book(HostOs::Linux));
    assert!(!host_has_address_book(HostOs::Windows));
    assert!(settings_url(HostOs::MacOs).is_some());
    assert_eq!(settings_url(HostOs::Linux), None);
    assert_eq!(settings_url(HostOs::Windows), None);
}

#[test]
fn a_save_request_keeps_operation_order() {
    let mut request = ContactSaveRequest::new(Some("author".to_owned()));
    let contact = DeviceContact { identifier: "c".to_owned(), ..DeviceContact::default() };
    request.add(contact.clone());
    request.update(contact.clone());
    request.delete("c");
    assert_eq!(request.transaction_author.as_deref(), Some("author"));
    assert_eq!(
        request.operations,
        vec![
            ContactSaveOperation::Add(contact.clone()),
            ContactSaveOperation::Update(contact),
            ContactSaveOperation::Delete { identifier: "c".to_owned() },
        ]
    );
}

#[test]
fn a_contact_knows_its_fetched_keys() {
    let contact = DeviceContact {
        fetched_keys: vec![ContactKey::Identifier, ContactKey::PhoneNumbers],
        ..DeviceContact::default()
    };
    assert!(contact.has_key(ContactKey::PhoneNumbers));
    assert!(!contact.has_key(ContactKey::ImageData));
}

struct WatchingStore {
    sink: Mutex<Option<StoreChangeSink>>,
    unregistered: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct MarkOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for MarkOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl ContactStore for WatchingStore {
    fn watch_changes(&self, sink: StoreChangeSink) -> Result<ContactStoreChangeWatch, ContactsError> {
        *self.sink.lock() = Some(sink);
        Ok(ContactStoreChangeWatch::new(MarkOnDrop(self.unregistered.clone())))
    }
}

#[test]
fn a_store_without_an_observer_answers_unsupported_for_the_host() {
    let store = FakeStore::leaked(
        ContactAuthorization::Authorized,
        ContactAuthorization::Authorized,
        true,
        None,
    );
    let answer = watch_store_changes_from(store, || {});
    assert_eq!(answer.unwrap_err(), ContactsError::Unsupported(HostOs::current()));
}

#[test]
fn each_reported_change_reaches_the_handler_until_the_watch_drops() {
    let store = WatchingStore {
        sink: Mutex::new(None),
        unregistered: std::sync::Arc::default(),
    };
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let watch = watch_store_changes_from(&store, move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("the store registers");
    let sink = store.sink.lock().take().expect("the sink was handed over");
    sink();
    sink();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(!store.unregistered.load(std::sync::atomic::Ordering::SeqCst));
    drop(watch);
    assert!(store.unregistered.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_panicking_handler_does_not_unwind_into_the_notification_center() {
    let sink = store_change_sink(|| panic!("handler failure"));
    sink();
}

#[test]
fn hosts_without_an_address_book_keep_no_observer() {
    if host_has_address_book(HostOs::current()) {
        return;
    }
    let answer = watch_store_changes(|| {});
    assert_eq!(answer.unwrap_err(), ContactsError::Unsupported(HostOs::current()));
}
