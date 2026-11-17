use std::collections::HashMap;
use std::ffi::{c_int, c_ulong, c_void};
use std::sync::OnceLock;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{AnyThread as _, DefinedClass, Message, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSData, NSError, NSNotification, NSNotificationCenter, NSString};
use parking_lot::Mutex;

use super::contacts::*;

#[link(name = "Contacts", kind = "framework")]
unsafe extern "C" {
    static CNContactIdentifierKey: &'static NSString;
    static CNContactGivenNameKey: &'static NSString;
    static CNContactFamilyNameKey: &'static NSString;
    static CNContactPhoneNumbersKey: &'static NSString;
    static CNContactImageDataKey: &'static NSString;
    static CNContactUrlAddressesKey: &'static NSString;
    static CNContactInstantMessageAddressesKey: &'static NSString;
    static CNLabelHome: &'static NSString;
    static CNLabelWork: &'static NSString;
    static CNLabelOther: &'static NSString;
    static CNLabelPhoneNumberMobile: &'static NSString;
    static CNLabelPhoneNumberiPhone: &'static NSString;
    static CNLabelPhoneNumberMain: &'static NSString;
    static CNLabelPhoneNumberPager: &'static NSString;
    static CNErrorDomain: &'static NSString;
    static CNContactStoreDidChangeNotification: &'static NSString;
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
}

const CN_ENTITY_TYPE_CONTACTS: isize = 0;

const ALL_WRITABLE_KEYS: [ContactKey; 6] = [
    ContactKey::GivenName,
    ContactKey::FamilyName,
    ContactKey::PhoneNumbers,
    ContactKey::UrlAddresses,
    ContactKey::InstantMessageAddresses,
    ContactKey::ImageData,
];

fn class(name: &'static std::ffi::CStr, label: &'static str) -> Result<&'static AnyClass, ContactsError> {
    AnyClass::get(name).ok_or(ContactsError::FrameworkMissing(label))
}

fn store_error(error: &NSError) -> ContactsError {
    let domain = error.domain().to_string();
    let cn_domain = unsafe { CNErrorDomain }.to_string();
    ContactsError::Store {
        is_cn_domain: domain == cn_domain,
        domain,
        code: error.code() as i64,
        description: error.localizedDescription().to_string(),
    }
}

/// the one store of the process
struct SharedStore(Retained<AnyObject>);

unsafe impl Send for SharedStore {}
unsafe impl Sync for SharedStore {}

static STORE: OnceLock<Option<SharedStore>> = OnceLock::new();

fn store() -> Result<&'static AnyObject, ContactsError> {
    let shared = STORE.get_or_init(|| {
        let store_class = AnyClass::get(c"CNContactStore")?;
        let store: Retained<AnyObject> = unsafe { msg_send![store_class, new] };
        Some(SharedStore(store))
    });
    shared.as_ref().map(|shared| &*shared.0).ok_or(ContactsError::FrameworkMissing("CNContactStore"))
}

fn key_name(key: ContactKey) -> &'static NSString {
    unsafe {
        match key {
            ContactKey::Identifier => CNContactIdentifierKey,
            ContactKey::GivenName => CNContactGivenNameKey,
            ContactKey::FamilyName => CNContactFamilyNameKey,
            ContactKey::PhoneNumbers => CNContactPhoneNumbersKey,
            ContactKey::ImageData => CNContactImageDataKey,
            ContactKey::UrlAddresses => CNContactUrlAddressesKey,
            ContactKey::InstantMessageAddresses => CNContactInstantMessageAddressesKey,
        }
    }
}

fn key_descriptors(keys: &[ContactKey]) -> Retained<NSArray<NSString>> {
    let names: Vec<&NSString> = keys.iter().map(|key| key_name(*key)).collect();
    NSArray::from_slice(&names)
}

fn system_labels() -> [(ContactLabel, String); 7] {
    unsafe {
        [
            (ContactLabel::Home, CNLabelHome.to_string()),
            (ContactLabel::Work, CNLabelWork.to_string()),
            (ContactLabel::Other, CNLabelOther.to_string()),
            (ContactLabel::PhoneNumberMobile, CNLabelPhoneNumberMobile.to_string()),
            (ContactLabel::PhoneNumberIPhone, CNLabelPhoneNumberiPhone.to_string()),
            (ContactLabel::PhoneNumberMain, CNLabelPhoneNumberMain.to_string()),
            (ContactLabel::PhoneNumberPager, CNLabelPhoneNumberPager.to_string()),
        ]
    }
}

fn read_label(raw: Option<Retained<NSString>>) -> Option<ContactLabel> {
    let raw = raw?.to_string();
    let named = system_labels().into_iter().find(|(_, system)| *system == raw);
    Some(named.map_or(ContactLabel::Custom(raw), |(label, _)| label))
}

fn label_string(label: &ContactLabel) -> Retained<NSString> {
    if let ContactLabel::Custom(raw) = label {
        return NSString::from_str(raw);
    }
    let system = system_labels().into_iter().find(|(named, _)| named == label).map(|(_, raw)| raw);
    NSString::from_str(&system.unwrap_or_default())
}

fn string_property(object: &AnyObject, read: impl FnOnce(&AnyObject) -> Retained<NSString>) -> String {
    read(object).to_string()
}

fn entry_head(entry: &AnyObject) -> (Option<String>, Option<ContactLabel>) {
    let (identifier, label): (Retained<NSString>, Option<Retained<NSString>>) =
        unsafe { (msg_send![entry, identifier], msg_send![entry, label]) };
    (Some(identifier.to_string()), read_label(label))
}

fn read_contact(contact: &AnyObject, keys: &[ContactKey]) -> DeviceContact {
    let mut read = DeviceContact {
        identifier: string_property(contact, |contact| unsafe { msg_send![contact, identifier] }),
        fetched_keys: keys.to_vec(),
        ..DeviceContact::default()
    };
    if keys.contains(&ContactKey::GivenName) {
        read.given_name = string_property(contact, |contact| unsafe { msg_send![contact, givenName] });
    }
    if keys.contains(&ContactKey::FamilyName) {
        read.family_name = string_property(contact, |contact| unsafe { msg_send![contact, familyName] });
    }
    if keys.contains(&ContactKey::PhoneNumbers) {
        let entries: Retained<NSArray<AnyObject>> = unsafe { msg_send![contact, phoneNumbers] };
        read.phone_numbers = entries
            .to_vec()
            .into_iter()
            .map(|entry| {
                let (identifier, label) = entry_head(&entry);
                let value: String = unsafe {
                    let number: Retained<AnyObject> = msg_send![&*entry, value];
                    let text: Retained<NSString> = msg_send![&*number, stringValue];
                    text.to_string()
                };
                LabeledValue { identifier, label, value }
            })
            .collect();
    }
    if keys.contains(&ContactKey::UrlAddresses) {
        let entries: Retained<NSArray<AnyObject>> = unsafe { msg_send![contact, urlAddresses] };
        read.url_addresses = entries
            .to_vec()
            .into_iter()
            .map(|entry| {
                let (identifier, label) = entry_head(&entry);
                let value: Retained<NSString> = unsafe { msg_send![&*entry, value] };
                LabeledValue { identifier, label, value: value.to_string() }
            })
            .collect();
    }
    if keys.contains(&ContactKey::InstantMessageAddresses) {
        let entries: Retained<NSArray<AnyObject>> = unsafe { msg_send![contact, instantMessageAddresses] };
        read.instant_message_addresses = entries
            .to_vec()
            .into_iter()
            .map(|entry| {
                let (identifier, label) = entry_head(&entry);
                let value = unsafe {
                    let address: Retained<AnyObject> = msg_send![&*entry, value];
                    let username: Retained<NSString> = msg_send![&*address, username];
                    let service: Retained<NSString> = msg_send![&*address, service];
                    InstantMessageAddress { username: username.to_string(), service: service.to_string() }
                };
                LabeledValue { identifier, label, value }
            })
            .collect();
    }
    if keys.contains(&ContactKey::ImageData) {
        let data: Option<Retained<NSData>> = unsafe { msg_send![contact, imageData] };
        read.image_data = data.map(|data| data.to_vec());
    }
    read
}

/// labeled value for value, the existing entry with that identifier relabelled in place
/// (store keeps it as the same entry) or a new one
fn labeled_entry<T: Message>(
    existing: &[Retained<AnyObject>],
    identifier: Option<&str>,
    label: Option<&ContactLabel>,
    value: &T,
) -> Result<Retained<AnyObject>, ContactsError> {
    let label = label.map(label_string);
    let label = label.as_deref();
    let kept = identifier.and_then(|wanted| {
        existing.iter().find(|entry| entry_head(entry).0.as_deref() == Some(wanted))
    });
    if let Some(entry) = kept {
        return Ok(unsafe { msg_send![&**entry, labeledValueBySettingLabel: label, value: value] });
    }
    let labeled_class = class(c"CNLabeledValue", "CNLabeledValue")?;
    Ok(unsafe { msg_send![labeled_class, labeledValueWithLabel: label, value: value] })
}

fn existing_entries(contact: &AnyObject, key: ContactKey, fetched: bool) -> Vec<Retained<AnyObject>> {
    if !fetched {
        return Vec::new();
    }
    let entries: Retained<NSArray<AnyObject>> = unsafe {
        match key {
            ContactKey::PhoneNumbers => msg_send![contact, phoneNumbers],
            ContactKey::UrlAddresses => msg_send![contact, urlAddresses],
            _ => msg_send![contact, instantMessageAddresses],
        }
    };
    entries.to_vec()
}

/// writes the properties of keys from contact into the mutable contact
fn write_contact(
    mutable: &AnyObject,
    contact: &DeviceContact,
    keys: &[ContactKey],
    source_fetched: bool,
) -> Result<(), ContactsError> {
    for key in keys {
        match key {
            ContactKey::Identifier => {}
            ContactKey::GivenName => {
                let name = NSString::from_str(&contact.given_name);
                let _: () = unsafe { msg_send![mutable, setGivenName: &*name] };
            }
            ContactKey::FamilyName => {
                let name = NSString::from_str(&contact.family_name);
                let _: () = unsafe { msg_send![mutable, setFamilyName: &*name] };
            }
            ContactKey::PhoneNumbers => {
                let existing = existing_entries(mutable, *key, source_fetched);
                let number_class = class(c"CNPhoneNumber", "CNPhoneNumber")?;
                let mut entries = Vec::with_capacity(contact.phone_numbers.len());
                for phone in &contact.phone_numbers {
                    let text = NSString::from_str(&phone.value);
                    let number: Retained<AnyObject> =
                        unsafe { msg_send![number_class, phoneNumberWithStringValue: &*text] };
                    entries.push(labeled_entry(&existing, phone.identifier.as_deref(), phone.label.as_ref(), &*number)?);
                }
                let entries = NSArray::from_retained_slice(&entries);
                let _: () = unsafe { msg_send![mutable, setPhoneNumbers: &*entries] };
            }
            ContactKey::UrlAddresses => {
                let existing = existing_entries(mutable, *key, source_fetched);
                let mut entries = Vec::with_capacity(contact.url_addresses.len());
                for url in &contact.url_addresses {
                    let text = NSString::from_str(&url.value);
                    entries.push(labeled_entry(&existing, url.identifier.as_deref(), url.label.as_ref(), &*text)?);
                }
                let entries = NSArray::from_retained_slice(&entries);
                let _: () = unsafe { msg_send![mutable, setUrlAddresses: &*entries] };
            }
            ContactKey::InstantMessageAddresses => {
                let existing = existing_entries(mutable, *key, source_fetched);
                let address_class = class(c"CNInstantMessageAddress", "CNInstantMessageAddress")?;
                let mut entries = Vec::with_capacity(contact.instant_message_addresses.len());
                for address in &contact.instant_message_addresses {
                    let username = NSString::from_str(&address.value.username);
                    let service = NSString::from_str(&address.value.service);
                    let value: Retained<AnyObject> = unsafe {
                        let allocated: Allocated<AnyObject> = msg_send![address_class, alloc];
                        msg_send![allocated, initWithUsername: &*username, service: &*service]
                    };
                    entries.push(labeled_entry(
                        &existing,
                        address.identifier.as_deref(),
                        address.label.as_ref(),
                        &*value,
                    )?);
                }
                let entries = NSArray::from_retained_slice(&entries);
                let _: () = unsafe { msg_send![mutable, setInstantMessageAddresses: &*entries] };
            }
            ContactKey::ImageData => {
                if let Some(bytes) = &contact.image_data {
                    let data = NSData::with_bytes(bytes);
                    let _: () = unsafe { msg_send![mutable, setImageData: &*data] };
                }
            }
        }
    }
    Ok(())
}

fn unified_contact_object(identifier: &str, keys: &[ContactKey]) -> Result<Retained<AnyObject>, ContactsError> {
    let store = store()?;
    let identifier = NSString::from_str(identifier);
    let keys = key_descriptors(keys);
    let found: Result<Retained<AnyObject>, Retained<NSError>> =
        unsafe { msg_send![store, unifiedContactWithIdentifier: &*identifier, keysToFetch: &*keys, error: _] };
    found.map_err(|error| store_error(&error))
}

fn mutable_copy(contact: &AnyObject) -> Retained<AnyObject> {
    unsafe { msg_send![contact, mutableCopy] }
}

fn contact_identifier(contact: &AnyObject) -> String {
    string_property(contact, |contact| unsafe { msg_send![contact, identifier] })
}

fn history_token(result: &AnyObject) -> Option<Vec<u8>> {
    let token: Option<Retained<NSData>> = unsafe { msg_send![result, currentHistoryToken] };
    token.map(|token| token.to_vec())
}

/// walks the enumerator of a fetch result until visit says stop
fn walk(result: &AnyObject, mut visit: impl FnMut(&AnyObject) -> bool) {
    let enumerator: Retained<AnyObject> = unsafe { msg_send![result, value] };
    loop {
        let next: Option<Retained<AnyObject>> = unsafe { msg_send![&*enumerator, nextObject] };
        let Some(next) = next else {
            return;
        };
        if !visit(&next) {
            return;
        }
    }
}

fn contact_enumerator(keys: &[ContactKey]) -> Result<Retained<AnyObject>, ContactsError> {
    let store = store()?;
    let request_class = class(c"CNContactFetchRequest", "CNContactFetchRequest")?;
    let keys = key_descriptors(keys);
    let result: Result<Retained<AnyObject>, Retained<NSError>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![request_class, alloc];
        let request: Retained<AnyObject> = msg_send![allocated, initWithKeysToFetch: &*keys];
        let _: () = msg_send![&*request, setUnifyResults: Bool::YES];
        msg_send![store, enumeratorForContactFetchRequest: &*request, error: _]
    };
    result.map_err(|error| store_error(&error))
}

fn is_kind(object: &AnyObject, kind: &AnyClass) -> bool {
    let is: Bool = unsafe { msg_send![object, isKindOfClass: kind] };
    is.as_bool()
}

pub(super) struct NativeContactStore;

impl ContactStore for NativeContactStore {
    fn authorization_status(&self) -> ContactAuthorization {
        let Some(store_class) = AnyClass::get(c"CNContactStore") else {
            return ContactAuthorization::Restricted;
        };
        let raw: isize = unsafe { msg_send![store_class, authorizationStatusForEntityType: CN_ENTITY_TYPE_CONTACTS] };
        ContactAuthorization::from_cn_status(raw as i64)
    }

    fn request_access(&self, done: AccessAnswer) -> Result<(), ContactsError> {
        let store = store()?;
        let block = AnswerBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: access_answered,
            descriptor: &raw const ANSWER_DESCRIPTOR,
            request: park(done),
        };
        unsafe {
            let _: () = msg_send![
                store,
                requestAccessForEntityType: CN_ENTITY_TYPE_CONTACTS,
                completionHandler: &raw const block
            ];
        }
        Ok(())
    }

    fn fetch_all(&self, keys: &[ContactKey]) -> Result<ContactFetchResult, ContactsError> {
        autoreleasepool(|_| {
            let result = contact_enumerator(keys)?;
            let mut contacts = Vec::new();
            walk(&result, |contact| {
                contacts.push(read_contact(contact, keys));
                true
            });
            Ok(ContactFetchResult { contacts, current_history_token: history_token(&result) })
        })
    }

    fn fetch_changes(
        &self,
        starting_token: &[u8],
        keys: &[ContactKey],
        excluded_transaction_authors: &[String],
    ) -> Result<ContactChangeResult, ContactsError> {
        autoreleasepool(|_| {
            let store = store()?;
            let request_class = class(c"CNChangeHistoryFetchRequest", "CNChangeHistoryFetchRequest")?;
            let drop_everything = class(c"CNChangeHistoryDropEverythingEvent", "CNChangeHistoryDropEverythingEvent")?;
            let add = class(c"CNChangeHistoryAddContactEvent", "CNChangeHistoryAddContactEvent")?;
            let update = class(c"CNChangeHistoryUpdateContactEvent", "CNChangeHistoryUpdateContactEvent")?;
            let delete = class(c"CNChangeHistoryDeleteContactEvent", "CNChangeHistoryDeleteContactEvent")?;
            let token = NSData::with_bytes(starting_token);
            let descriptors = key_descriptors(keys);
            let authors: Vec<Retained<NSString>> =
                excluded_transaction_authors.iter().map(|author| NSString::from_str(author)).collect();
            let authors = NSArray::from_retained_slice(&authors);
            let result: Result<Retained<AnyObject>, Retained<NSError>> = unsafe {
                let request: Retained<AnyObject> = msg_send![request_class, new];
                let _: () = msg_send![&*request, setStartingToken: &*token];
                let _: () = msg_send![&*request, setAdditionalContactKeyDescriptors: &*descriptors];
                let _: () = msg_send![&*request, setShouldUnifyResults: Bool::YES];
                let _: () = msg_send![&*request, setMutableObjects: Bool::NO];
                let _: () = msg_send![&*request, setIncludeGroupChanges: Bool::NO];
                let _: () = msg_send![&*request, setExcludedTransactionAuthors: &*authors];
                msg_send![store, enumeratorForChangeHistoryFetchRequest: &*request, error: _]
            };
            let result = result.map_err(|error| store_error(&error))?;
            let mut changes = Vec::new();
            walk(&result, |event| {
                if is_kind(event, drop_everything) {
                    changes.push(ContactChange::DropEverything);
                } else if is_kind(event, add) || is_kind(event, update) {
                    let contact: Retained<AnyObject> = unsafe { msg_send![event, contact] };
                    let contact = read_contact(&contact, keys);
                    changes.push(if is_kind(event, add) {
                        ContactChange::Add(contact)
                    } else {
                        ContactChange::Update(contact)
                    });
                } else if is_kind(event, delete) {
                    let identifier: Retained<NSString> = unsafe { msg_send![event, contactIdentifier] };
                    changes.push(ContactChange::Delete { contact_identifier: identifier.to_string() });
                }
                true
            });
            Ok(ContactChangeResult { changes, current_history_token: history_token(&result) })
        })
    }

    fn current_history_token(&self) -> Option<Vec<u8>> {
        store().ok().and_then(history_token)
    }

    fn unified_contact(&self, identifier: &str, keys: &[ContactKey]) -> Result<DeviceContact, ContactsError> {
        autoreleasepool(|_| unified_contact_object(identifier, keys).map(|contact| read_contact(&contact, keys)))
    }

    fn unified_contacts_matching_phone(
        &self,
        phone: &str,
        keys: &[ContactKey],
    ) -> Result<Vec<DeviceContact>, ContactsError> {
        autoreleasepool(|_| {
            let store = store()?;
            let number_class = class(c"CNPhoneNumber", "CNPhoneNumber")?;
            let contact_class = class(c"CNContact", "CNContact")?;
            let text = NSString::from_str(phone);
            let descriptors = key_descriptors(keys);
            let found: Result<Retained<NSArray<AnyObject>>, Retained<NSError>> = unsafe {
                let number: Retained<AnyObject> = msg_send![number_class, phoneNumberWithStringValue: &*text];
                let predicate: Retained<AnyObject> =
                    msg_send![contact_class, predicateForContactsMatchingPhoneNumber: &*number];
                msg_send![store, unifiedContactsMatchingPredicate: &*predicate, keysToFetch: &*descriptors, error: _]
            };
            let found = found.map_err(|error| store_error(&error))?;
            Ok(found.to_vec().iter().map(|contact| read_contact(contact, keys)).collect())
        })
    }

    fn first_contact(
        &self,
        keys: &[ContactKey],
        matches: &mut dyn FnMut(&DeviceContact) -> bool,
    ) -> Result<Option<DeviceContact>, ContactsError> {
        autoreleasepool(|_| {
            let result = contact_enumerator(keys)?;
            let mut found = None;
            walk(&result, |contact| {
                let contact = read_contact(contact, keys);
                if matches(&contact) {
                    found = Some(contact);
                    return false;
                }
                true
            });
            Ok(found)
        })
    }

    fn execute(&self, request: &ContactSaveRequest) -> Result<Vec<String>, ContactsError> {
        autoreleasepool(|_| {
            let store = store()?;
            let save_class = class(c"CNSaveRequest", "CNSaveRequest")?;
            let mutable_class = class(c"CNMutableContact", "CNMutableContact")?;
            let save: Retained<AnyObject> = unsafe { msg_send![save_class, new] };
            if let Some(author) = &request.transaction_author {
                let author = NSString::from_str(author);
                let _: () = unsafe { msg_send![&*save, setTransactionAuthor: &*author] };
            }
            let mut saved = Vec::with_capacity(request.operations.len());
            for operation in &request.operations {
                match operation {
                    ContactSaveOperation::Add(contact) => {
                        let mutable: Retained<AnyObject> = unsafe { msg_send![mutable_class, new] };
                        write_contact(&mutable, contact, &ALL_WRITABLE_KEYS, false)?;
                        let container: Option<&NSString> = None;
                        let _: () = unsafe { msg_send![&*save, addContact: &*mutable, toContainerWithIdentifier: container] };
                        saved.push(mutable);
                    }
                    ContactSaveOperation::Update(contact) => {
                        let source = unified_contact_object(&contact.identifier, &contact.fetched_keys)?;
                        let mutable = mutable_copy(&source);
                        write_contact(&mutable, contact, &contact.fetched_keys, true)?;
                        let _: () = unsafe { msg_send![&*save, updateContact: &*mutable] };
                        saved.push(mutable);
                    }
                    ContactSaveOperation::Delete { identifier } => {
                        let source = unified_contact_object(identifier, &[ContactKey::Identifier])?;
                        let mutable = mutable_copy(&source);
                        let _: () = unsafe { msg_send![&*save, deleteContact: &*mutable] };
                        saved.push(mutable);
                    }
                }
            }
            let executed: Result<(), Retained<NSError>> = unsafe { msg_send![store, executeSaveRequest: &*save, error: _] };
            executed.map_err(|error| store_error(&error))?;
            Ok(saved.iter().map(|contact| contact_identifier(contact)).collect())
        })
    }

    fn watch_changes(&self, sink: StoreChangeSink) -> Result<ContactStoreChangeWatch, ContactsError> {
        store()?;
        let observer = StoreChangeObserver::new(sink);
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(storeDidChange:),
                Some(CNContactStoreDidChangeNotification),
                None,
            );
        }
        Ok(ContactStoreChangeWatch::new(StoreChangeRegistration { center, observer }))
    }
}

struct StoreChangeIvars {
    sink: StoreChangeSink,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseContactStoreChangeObserver"]
    #[ivars = StoreChangeIvars]
    struct StoreChangeObserver;

    impl StoreChangeObserver {
        #[unsafe(method(storeDidChange:))]
        fn store_did_change(&self, _notification: &NSNotification) {
            (self.ivars().sink)();
        }
    }
);

impl StoreChangeObserver {
    fn new(sink: StoreChangeSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(StoreChangeIvars { sink });
        unsafe { msg_send![super(this), init] }
    }
}

struct StoreChangeRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<StoreChangeObserver>,
}

unsafe impl Send for StoreChangeRegistration {}

impl Drop for StoreChangeRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}

#[repr(C)]
struct AnswerBlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
}

#[repr(C)]
struct AnswerBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const AnswerBlock, Bool, *mut NSError),
    descriptor: *const AnswerBlockDescriptor,
    request: u64,
}

unsafe impl RefEncode for AnswerBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static ANSWER_DESCRIPTOR: AnswerBlockDescriptor = AnswerBlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<AnswerBlock>() as c_ulong,
};

struct Waiting {
    answers: HashMap<u64, AccessAnswer>,
    next: u64,
}

static WAITING: Mutex<Option<Waiting>> = Mutex::new(None);

fn park(done: AccessAnswer) -> u64 {
    let mut guard = WAITING.lock();
    let waiting = guard.get_or_insert_with(|| Waiting { answers: HashMap::new(), next: 1 });
    let request = waiting.next;
    waiting.next += 1;
    waiting.answers.insert(request, done);
    request
}

fn unpark(request: u64) -> Option<AccessAnswer> {
    WAITING.lock().as_mut().and_then(|waiting| waiting.answers.remove(&request))
}

unsafe extern "C" fn access_answered(block: *const AnswerBlock, granted: Bool, error: *mut NSError) {
    let request = unsafe { (*block).request };
    let Some(done) = unpark(request) else {
        return;
    };
    let error = unsafe { error.as_ref() }.map(store_error);
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || done(granted.as_bool(), error))).is_err() {
        tracing::error!(target: "riseonly", "contacts access answer panicked");
    }
}
