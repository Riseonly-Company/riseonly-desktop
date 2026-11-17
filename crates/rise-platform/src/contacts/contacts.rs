use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// authorization status for the contacts entity
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ContactAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
    Limited,
}

impl ContactAuthorization {
    /// raw values of CNContactStore.h
    /// not determined 0 and restricted 1 and denied 2 and authorized 3 and limited 4
    /// an unknown future value is denied
    pub const fn from_cn_status(raw: i64) -> Self {
        match raw {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            3 => Self::Authorized,
            4 => Self::Limited,
            _ => Self::Denied,
        }
    }

    pub const fn can_read_and_write(self) -> bool {
        matches!(self, Self::Authorized | Self::Limited)
    }

    /// true when asking shows the system prompt, otherwise the feature opens system settings
    pub const fn can_request(self) -> bool {
        matches!(self, Self::NotDetermined)
    }
}

/// keys the device book fetches
/// a property of a fetched contact is read only when its key was fetched, contacts raises on any other
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ContactKey {
    Identifier,
    GivenName,
    FamilyName,
    PhoneNumbers,
    ImageData,
    UrlAddresses,
    InstantMessageAddresses,
}

/// label of a labeled value, system labels are named
/// any other string is kept raw so it round trips on save
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ContactLabel {
    Home,
    Work,
    Other,
    PhoneNumberMobile,
    PhoneNumberIPhone,
    PhoneNumberMain,
    PhoneNumberPager,
    Custom(String),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct InstantMessageAddress {
    pub username: String,
    pub service: String,
}

/// labeled value, identifier is the store id of an entry read from a contact and a new entry has none
/// saving keeps an entry with an identifier as the same entry
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LabeledValue<T> {
    pub identifier: Option<String>,
    pub label: Option<ContactLabel>,
    pub value: T,
}

impl<T> LabeledValue<T> {
    pub fn new(label: Option<ContactLabel>, value: T) -> Self {
        Self { identifier: None, label, value }
    }
}

/// contact with the keys it was fetched with
/// properties of keys not fetched stay empty and an update writes back only the fetched ones
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DeviceContact {
    pub identifier: String,
    pub given_name: String,
    pub family_name: String,
    pub phone_numbers: Vec<LabeledValue<String>>,
    pub url_addresses: Vec<LabeledValue<String>>,
    pub instant_message_addresses: Vec<LabeledValue<InstantMessageAddress>>,
    pub image_data: Option<Vec<u8>>,
    pub fetched_keys: Vec<ContactKey>,
}

impl DeviceContact {
    pub fn has_key(&self, key: ContactKey) -> bool {
        self.fetched_keys.contains(&key)
    }
}

/// the unified book and the history token read with it
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContactFetchResult {
    pub contacts: Vec<DeviceContact>,
    pub current_history_token: Option<Vec<u8>>,
}

/// one change history event
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContactChange {
    DropEverything,
    Add(DeviceContact),
    Update(DeviceContact),
    Delete { contact_identifier: String },
}

/// events in store order and the token after them
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContactChangeResult {
    pub changes: Vec<ContactChange>,
    pub current_history_token: Option<Vec<u8>>,
}

impl ContactChangeResult {
    /// answer for a history the store cant continue, one drop everything and no token
    pub fn drop_everything() -> Self {
        Self { changes: vec![ContactChange::DropEverything], current_history_token: None }
    }

    pub fn drops_everything(&self) -> bool {
        self.changes.iter().any(|change| matches!(change, ContactChange::DropEverything))
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContactSaveOperation {
    Add(DeviceContact),
    Update(DeviceContact),
    Delete { identifier: String },
}

/// save request with its transaction author, change history fetches can exclude it
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContactSaveRequest {
    pub transaction_author: Option<String>,
    pub operations: Vec<ContactSaveOperation>,
}

impl ContactSaveRequest {
    pub fn new(transaction_author: Option<String>) -> Self {
        Self { transaction_author, operations: Vec::new() }
    }

    pub fn add(&mut self, contact: DeviceContact) {
        self.operations.push(ContactSaveOperation::Add(contact));
    }

    pub fn update(&mut self, contact: DeviceContact) {
        self.operations.push(ContactSaveOperation::Update(contact));
    }

    pub fn delete(&mut self, identifier: impl Into<String>) {
        self.operations.push(ContactSaveOperation::Delete { identifier: identifier.into() });
    }
}

pub const CN_ERROR_RECORD_DOES_NOT_EXIST: i64 = 200;

/// why the address book could not be read or written
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ContactsError {
    #[error("the {0:?} host has no system address book an unpackaged app may use")]
    Unsupported(HostOs),
    #[error("the system framework class {0} is not available")]
    FrameworkMissing(&'static str),
    #[error("contacts store error {code} in {domain}: {description}")]
    Store { domain: String, is_cn_domain: bool, code: i64, description: String },
}

impl ContactsError {
    /// true when the error is the record does not exist one
    pub fn is_record_does_not_exist(&self) -> bool {
        matches!(self, Self::Store { is_cn_domain: true, code: CN_ERROR_RECORD_DOES_NOT_EXIST, .. })
    }
}

/// raw answer of the prompt, granted and whether the store passed an error with it
pub type AccessAnswer = Box<dyn FnOnce(bool, Option<ContactsError>) + Send>;

/// continuation of the authorization request, dont allow also arrives as an error
/// so an error counts only while the status is undecided, once decided the answer is granted
pub fn access_answer(
    granted: bool,
    error: Option<ContactsError>,
    status_after: ContactAuthorization,
) -> Result<bool, ContactsError> {
    match error {
        Some(error) if status_after == ContactAuthorization::NotDetermined => Err(error),
        _ => Ok(granted),
    }
}

/// os side, every method defaults to unsupported and the status to restricted
/// sync so the book can be read from the account runtime, reads and saves are synchronous
pub trait ContactStore: Sync {
    fn authorization_status(&self) -> ContactAuthorization {
        ContactAuthorization::Restricted
    }

    fn request_access(&self, _done: AccessAnswer) -> Result<(), ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn fetch_all(&self, _keys: &[ContactKey]) -> Result<ContactFetchResult, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn fetch_changes(
        &self,
        _starting_token: &[u8],
        _keys: &[ContactKey],
        _excluded_transaction_authors: &[String],
    ) -> Result<ContactChangeResult, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn current_history_token(&self) -> Option<Vec<u8>> {
        None
    }

    /// unified contact by identifier with the keys to fetch
    fn unified_contact(&self, _identifier: &str, _keys: &[ContactKey]) -> Result<DeviceContact, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn unified_contacts_matching_phone(
        &self,
        _phone: &str,
        _keys: &[ContactKey],
    ) -> Result<Vec<DeviceContact>, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn first_contact(
        &self,
        _keys: &[ContactKey],
        _matches: &mut dyn FnMut(&DeviceContact) -> bool,
    ) -> Result<Option<DeviceContact>, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    /// executes the request, gives the contact identifier of each operation in order (an added
    /// contact gets a new one)
    fn execute(&self, _request: &ContactSaveRequest) -> Result<Vec<String>, ContactsError> {
        Err(ContactsError::Unsupported(HostOs::current()))
    }

    fn watch_changes(&self, sink: StoreChangeSink) -> Result<ContactStoreChangeWatch, ContactsError> {
        drop(sink);
        Err(ContactsError::Unsupported(HostOs::current()))
    }
}

/// where an os layer reports an address book change, may run on a system thread
/// the app hops to the main thread itself
pub type StoreChangeSink = Box<dyn Fn() + Send + Sync>;

/// keeps the address book observer registered, dropping it stops the reports
#[must_use = "dropping the watch stops address book change reports"]
pub struct ContactStoreChangeWatch {
    _registration: Box<dyn Send>,
}

impl ContactStoreChangeWatch {
    /// registration unregisters the system observer in its own drop
    pub(crate) fn new(registration: impl Send + 'static) -> Self {
        Self {
            _registration: Box::new(registration),
        }
    }
}

impl std::fmt::Debug for ContactStoreChangeWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ContactStoreChangeWatch")
    }
}

/// sink that never lets a panic of on_change unwind into the system notification center (that aborts)
/// the report is lost and logged instead
pub fn store_change_sink(on_change: impl Fn() + Send + Sync + 'static) -> StoreChangeSink {
    Box::new(move || {
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&on_change));
        if delivered.is_err() {
            tracing::error!(target: "riseonly", "contact store change handler panicked");
        }
    })
}

pub fn watch_store_changes_from(
    store: &dyn ContactStore,
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<ContactStoreChangeWatch, ContactsError> {
    store.watch_changes(store_change_sink(on_change))
}

/// changes since starting_token
/// an empty token has no history to continue so it answers drop everything without touching the store
pub fn fetch_changes_from(
    store: &dyn ContactStore,
    starting_token: &[u8],
    keys: &[ContactKey],
    excluded_transaction_authors: &[String],
) -> Result<ContactChangeResult, ContactsError> {
    if starting_token.is_empty() {
        return Ok(ContactChangeResult::drop_everything());
    }
    store.fetch_changes(starting_token, keys, excluded_transaction_authors)
}

#[derive(Default)]
struct AnswerSlot {
    answer: Option<Result<bool, ContactsError>>,
    waker: Option<Waker>,
}

fn fill(slot: &Mutex<AnswerSlot>, answer: Result<bool, ContactsError>) {
    let waker = {
        let mut slot = slot.lock();
        if slot.answer.is_some() {
            return;
        }
        slot.answer = Some(answer);
        slot.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// the prompt answer, a continuation around the access request
/// runs on any executor and the os answer wakes it
pub struct ContactAccessRequest {
    slot: Arc<Mutex<AnswerSlot>>,
}

impl Future for ContactAccessRequest {
    type Output = Result<bool, ContactsError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.lock();
        match slot.answer.take() {
            Some(answer) => Poll::Ready(answer),
            None => {
                slot.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// asks store, the entry points of the seam pass the os file
pub fn request_access_from(store: &'static dyn ContactStore) -> ContactAccessRequest {
    let slot = Arc::new(Mutex::new(AnswerSlot::default()));
    let answered = Arc::clone(&slot);
    let done: AccessAnswer = Box::new(move |granted, error| {
        fill(&answered, access_answer(granted, error, store.authorization_status()));
    });
    if let Err(reason) = store.request_access(done) {
        fill(&slot, Err(reason));
    }
    ContactAccessRequest { slot }
}

/// the prompt and then the status read again
pub async fn request_authorization_from(
    store: &'static dyn ContactStore,
) -> Result<ContactAuthorization, ContactsError> {
    request_access_from(store).await?;
    Ok(store.authorization_status())
}

static NATIVE: super::os::NativeContactStore = super::os::NativeContactStore;

fn store() -> &'static dyn ContactStore {
    &NATIVE
}

pub fn authorization_status() -> ContactAuthorization {
    store().authorization_status()
}

pub fn request_access() -> ContactAccessRequest {
    request_access_from(store())
}

pub async fn request_authorization() -> Result<ContactAuthorization, ContactsError> {
    request_authorization_from(store()).await
}

pub fn fetch_all(keys: &[ContactKey]) -> Result<ContactFetchResult, ContactsError> {
    store().fetch_all(keys)
}

pub fn fetch_changes(
    starting_token: &[u8],
    keys: &[ContactKey],
    excluded_transaction_authors: &[String],
) -> Result<ContactChangeResult, ContactsError> {
    fetch_changes_from(store(), starting_token, keys, excluded_transaction_authors)
}

pub fn current_history_token() -> Option<Vec<u8>> {
    store().current_history_token()
}

pub fn unified_contact(identifier: &str, keys: &[ContactKey]) -> Result<DeviceContact, ContactsError> {
    store().unified_contact(identifier, keys)
}

pub fn unified_contacts_matching_phone(phone: &str, keys: &[ContactKey]) -> Result<Vec<DeviceContact>, ContactsError> {
    store().unified_contacts_matching_phone(phone, keys)
}

pub fn first_contact(
    keys: &[ContactKey],
    matches: &mut dyn FnMut(&DeviceContact) -> bool,
) -> Result<Option<DeviceContact>, ContactsError> {
    store().first_contact(keys, matches)
}

pub fn execute(request: &ContactSaveRequest) -> Result<Vec<String>, ContactsError> {
    store().execute(request)
}

/// observes the address book and schedules a sync
/// on_change may run on a system thread so never assume main
/// on err the app keeps no observer and sync stays off
pub fn watch_store_changes(
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<ContactStoreChangeWatch, ContactsError> {
    watch_store_changes_from(store(), on_change)
}

/// true when this host has a system address book at all
pub const fn host_has_address_book(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// performed when this host has an address book, the sync screen shows no access otherwise
pub fn contacts_support() -> PlatformSupport {
    if host_has_address_book(HostOs::current()) {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

/// contacts privacy pane, only macos has one that lists an unpackaged app
pub const fn settings_url(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::MacOs => Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Contacts"),
        HostOs::Linux | HostOs::Windows => None,
    }
}

pub fn open_settings(app: &mut gpui::App) -> PlatformSupport {
    match settings_url(HostOs::current()) {
        Some(url) => {
            app.open_url(url);
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}
