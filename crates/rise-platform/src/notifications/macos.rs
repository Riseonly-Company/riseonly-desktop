use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_int, c_ulong, c_void};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::task::{Context, Poll, Waker};

use objc2::encode::{Encoding, RefEncode};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2_foundation::{NSArray, NSData, NSError, NSSet, NSString, NSURL};
use parking_lot::Mutex;

use crate::gpui_shim::PlatformSupport;
use crate::notifications::*;

const SILENT: bool = cfg!(debug_assertions);

const AUTHORIZATION_OPTIONS: usize = (1 << 1) | (1 << 2);
const OUTGOING_MESSAGE_TEXT: isize = 1;
const PERSON_HANDLE_UNKNOWN: isize = 0;
const SUGGESTION_NONE: isize = 0;
const ME_IDENTIFIER: &str = "riseonly.me";
const FALLBACK_TITLE: &str = "Riseonly";

#[link(name = "Intents", kind = "framework")]
unsafe extern "C" {
    static INIntentErrorDomain: &'static NSString;
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
}

#[repr(C)]
struct AuthorizationBlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
}

#[repr(C)]
struct AuthorizationBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const AuthorizationBlock, Bool, *mut NSError),
    descriptor: *const AuthorizationBlockDescriptor,
}

unsafe impl RefEncode for AuthorizationBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

#[repr(C)]
struct NilBlock {
    _private: [u8; 0],
}

unsafe impl RefEncode for NilBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static AUTHORIZATION_DESCRIPTOR: AuthorizationBlockDescriptor = AuthorizationBlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<AuthorizationBlock>() as c_ulong,
};

#[repr(C)]
struct SettingsBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const SettingsBlock, *mut AnyObject),
    descriptor: *const AuthorizationBlockDescriptor,
    request: u64,
}

unsafe impl RefEncode for SettingsBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static SETTINGS_DESCRIPTOR: AuthorizationBlockDescriptor = AuthorizationBlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<SettingsBlock>() as c_ulong,
};

fn authorization_status_from_raw(raw: isize) -> Option<NotificationAuthorizationStatus> {
    match raw {
        0 => Some(NotificationAuthorizationStatus::NotDetermined),
        1 => Some(NotificationAuthorizationStatus::Denied),
        2 => Some(NotificationAuthorizationStatus::Authorized),
        3 => Some(NotificationAuthorizationStatus::Provisional),
        4 => Some(NotificationAuthorizationStatus::Ephemeral),
        _ => None,
    }
}

#[derive(Default)]
struct StatusSlot {
    answer: Option<Option<NotificationAuthorizationStatus>>,
    waker: Option<Waker>,
}

struct StatusQuery {
    slot: Arc<Mutex<StatusSlot>>,
}

impl Future for StatusQuery {
    type Output = Option<NotificationAuthorizationStatus>;

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

struct StatusWaiting {
    slots: HashMap<u64, Arc<Mutex<StatusSlot>>>,
    next: u64,
}

static STATUS_WAITING: Mutex<Option<StatusWaiting>> = Mutex::new(None);

fn park_status(slot: Arc<Mutex<StatusSlot>>) -> u64 {
    let mut guard = STATUS_WAITING.lock();
    let waiting = guard.get_or_insert_with(|| StatusWaiting { slots: HashMap::new(), next: 1 });
    let request = waiting.next;
    waiting.next += 1;
    waiting.slots.insert(request, slot);
    request
}

/// false when the query was already settled
fn settle_status(request: u64, status: Option<NotificationAuthorizationStatus>) -> bool {
    let slot = STATUS_WAITING
        .lock()
        .as_mut()
        .and_then(|waiting| waiting.slots.remove(&request));
    let Some(slot) = slot else {
        return false;
    };
    let waker = {
        let mut slot = slot.lock();
        slot.answer = Some(status);
        slot.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
    true
}

const STATUS_ANSWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

unsafe extern "C" fn settings_answered(block: *const SettingsBlock, settings: *mut AnyObject) {
    let request = unsafe { (*block).request };
    let read = std::panic::AssertUnwindSafe(|| {
        let raw = unsafe { settings.as_ref() }
            .map(|settings| -> isize { unsafe { msg_send![settings, authorizationStatus] } });
        let status = raw.and_then(authorization_status_from_raw);
        if status.is_none() {
            tracing::warn!(
                target: "riseonly",
                raw = ?raw,
                "notification settings carry no known authorization status"
            );
        }
        let _ = settle_status(request, status);
    });
    if std::panic::catch_unwind(read).is_err() {
        tracing::error!(target: "riseonly", "notification settings answer panicked");
    }
}

fn query_authorization_status() -> NotificationAuthorizationQuery {
    if !host_precondition_met() {
        return Box::pin(std::future::ready(None));
    }
    let Some(center_class) = AnyClass::get(c"UNUserNotificationCenter") else {
        tracing::warn!(target: "riseonly", "class UNUserNotificationCenter missing");
        return Box::pin(std::future::ready(None));
    };
    let slot = Arc::new(Mutex::new(StatusSlot::default()));
    let request = park_status(Arc::clone(&slot));
    std::thread::spawn(move || {
        std::thread::sleep(STATUS_ANSWER_DEADLINE);
        if settle_status(request, None) {
            tracing::warn!(target: "riseonly::X753", request, "notification settings unanswered past the deadline");
        }
    });
    autoreleasepool(|_| {
        let center: Option<Retained<AnyObject>> =
            unsafe { msg_send![center_class, currentNotificationCenter] };
        let Some(center) = center else {
            tracing::warn!(target: "riseonly", "currentNotificationCenter returned nil");
            let _ = settle_status(request, None);
            return;
        };
        let block = SettingsBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: settings_answered,
            descriptor: &raw const SETTINGS_DESCRIPTOR,
            request,
        };
        unsafe {
            let _: () = msg_send![
                &*center,
                getNotificationSettingsWithCompletionHandler: &raw const block
            ];
        }
    });
    Box::pin(StatusQuery { slot })
}

unsafe extern "C" fn authorization_answered(
    _block: *const AuthorizationBlock,
    granted: Bool,
    error: *mut NSError,
) {
    match unsafe { error.as_ref() } {
        Some(error) => tracing::warn!(
            target: "riseonly",
            error = %error.localizedDescription(),
            "notification authorization failed"
        ),
        None if !granted.as_bool() => {
            tracing::info!(target: "riseonly", "notification authorization denied")
        }
        None => {}
    }
}

/// one step for the poster thread, in the order the main thread asked
enum Job {
    Post(LocalNotification),
    Remove(NotificationTag),
    RemoveAll,
    RequestAuthorization,
}

fn poster() -> Option<&'static mpsc::Sender<Job>> {
    static POSTER: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    POSTER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new()
                .name("rise-notifications".to_owned())
                .spawn(move || {
                    let mut center = PosterCenter::default();
                    while let Ok(job) = receiver.recv() {
                        let run = std::panic::AssertUnwindSafe(|| {
                            autoreleasepool(|_| center.run(job))
                        });
                        if std::panic::catch_unwind(run).is_err() {
                            tracing::error!(target: "riseonly", "notification job panicked");
                        }
                    }
                });
            match spawned {
                Ok(_) => Some(sender),
                Err(error) => {
                    tracing::error!(
                        target: "riseonly",
                        %error,
                        "notification thread did not start"
                    );
                    None
                }
            }
        })
        .as_ref()
}

thread_local! {
    static GPUI_CENTER_READY: Cell<bool> = const { Cell::new(false) };
    static RESPONSE_TAP: RefCell<Option<Retained<ResponseTap>>> = const { RefCell::new(None) };
}

/// typed replies waiting for the gpui response task to hand over the same click
struct PendingReply {
    tag: String,
    action_id: String,
    text: String,
}

const PENDING_REPLY_CAP: usize = 16;

static PENDING_REPLIES: Mutex<Vec<PendingReply>> = Mutex::new(Vec::new());

fn remember_reply(reply: PendingReply) {
    let mut pending = PENDING_REPLIES.lock();
    pending.retain(|held| held.tag != reply.tag);
    if pending.len() == PENDING_REPLY_CAP {
        pending.remove(0);
    }
    pending.push(reply);
}

fn take_reply(tag: &str, action_id: &str) -> Option<String> {
    let mut pending = PENDING_REPLIES.lock();
    let index = pending
        .iter()
        .position(|held| held.tag == tag && held.action_id == action_id)?;
    Some(pending.remove(index).text)
}

fn note_reply_text(response: &AnyObject) {
    let Some(text_response) = AnyClass::get(c"UNTextInputNotificationResponse") else {
        return;
    };
    let is_text: bool = unsafe { msg_send![response, isKindOfClass: text_response] };
    if !is_text {
        return;
    }
    let reply = unsafe {
        let text: Retained<NSString> = msg_send![response, userText];
        let action_id: Retained<NSString> = msg_send![response, actionIdentifier];
        let notification: Retained<AnyObject> = msg_send![response, notification];
        let request: Retained<AnyObject> = msg_send![&*notification, request];
        let tag: Retained<NSString> = msg_send![&*request, identifier];
        PendingReply {
            tag: tag.to_string(),
            action_id: action_id.to_string(),
            text: text.to_string(),
        }
    };
    remember_reply(reply);
}

struct ResponseTapIvars {
    gpui_delegate: Retained<AnyObject>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseNotificationResponseTap"]
    #[ivars = ResponseTapIvars]
    struct ResponseTap;

    impl ResponseTap {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(&self, center: &AnyObject, response: &AnyObject, completion: &AnyObject) {
            note_reply_text(response);
            unsafe {
                let _: () = msg_send![
                    &*self.ivars().gpui_delegate,
                    userNotificationCenter: center,
                    didReceiveNotificationResponse: response,
                    withCompletionHandler: completion
                ];
            }
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, center: &AnyObject, notification: &AnyObject, completion: &AnyObject) {
            unsafe {
                let _: () = msg_send![
                    &*self.ivars().gpui_delegate,
                    userNotificationCenter: center,
                    willPresentNotification: notification,
                    withCompletionHandler: completion
                ];
            }
        }
    }
);

/// gpui response drops userText and its center owns the delegate slot
/// so this delegate sits in front, keeps the text and passes every call on
/// call after the gpui center is up and on the main thread
fn install_response_tap() {
    if RESPONSE_TAP.with(|tap| tap.borrow().is_some()) || !host_precondition_met() {
        return;
    }
    let Some(center_class) = AnyClass::get(c"UNUserNotificationCenter") else {
        return;
    };
    let (center, gpui_delegate): (Option<Retained<AnyObject>>, Option<Retained<AnyObject>>) = unsafe {
        let center: Option<Retained<AnyObject>> = msg_send![center_class, currentNotificationCenter];
        let delegate = center.as_ref().and_then(|center| msg_send![&**center, delegate]);
        (center, delegate)
    };
    let (Some(center), Some(gpui_delegate)) = (center, gpui_delegate) else {
        tracing::warn!(target: "riseonly", "no gpui notification delegate to stand in front of");
        return;
    };
    let tap = ResponseTap::alloc().set_ivars(ResponseTapIvars { gpui_delegate });
    let tap: Retained<ResponseTap> = unsafe { msg_send![super(tap), init] };
    let tap_object: &AnyObject = &tap;
    unsafe {
        let _: () = msg_send![&*center, setDelegate: tap_object];
    }
    RESPONSE_TAP.with(|slot| *slot.borrow_mut() = Some(tap));
}

pub(super) struct NativeNotifications;

impl NativeNotificationRoute for NativeNotifications {
    fn attachment_fallback_support(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn present(&self, app: &gpui::App, notification: &LocalNotification) -> PlatformSupport {
        if !GPUI_CENTER_READY.replace(true) {
            app.dismiss_system_notification(notification.tag.as_str());
        }
        install_response_tap();
        enqueue(Job::Post(notification.clone()))
    }

    fn dismiss(&self, _app: &gpui::App, tag: &NotificationTag) -> PlatformSupport {
        enqueue(Job::Remove(tag.clone()))
    }

    fn dismiss_all(&self, _app: &gpui::App) -> PlatformSupport {
        enqueue(Job::RemoveAll)
    }

    fn authorization_status(&self) -> NotificationAuthorizationQuery {
        query_authorization_status()
    }

    fn on_action_response(&self, app: &gpui::App, mut callback: NotificationActionResponseCallback) {
        self.on_response(
            app,
            Box::new(move |response, app| {
                let user_text = response
                    .action_id
                    .as_ref()
                    .and_then(|action_id| take_reply(&response.tag, action_id));
                let mut response = NotificationActionResponse::from(response);
                response.user_text = user_text;
                callback(response, app);
            }),
        );
        GPUI_CENTER_READY.set(true);
        install_response_tap();
    }

    fn reply_input_support(&self) -> PlatformSupport {
        if host_precondition_met() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }

    fn request_authorization(&self) -> PlatformSupport {
        if host_precondition_met() {
            enqueue(Job::RequestAuthorization)
        } else {
            PlatformSupport::Unsupported
        }
    }
}

fn enqueue(job: Job) -> PlatformSupport {
    match poster().map(|sender| sender.send(job)) {
        Some(Ok(())) => PlatformSupport::Performed,
        Some(Err(_)) | None => PlatformSupport::Unsupported,
    }
}

pub(super) fn host_precondition_met() -> bool {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .is_some()
}

/// why a post couldnt be built, logged and the post is dropped
enum PostFailure {
    ClassMissing(&'static str),
    NilFrom(&'static str),
}

impl std::fmt::Display for PostFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClassMissing(name) => write!(f, "class {name} missing"),
            Self::NilFrom(call) => write!(f, "{call} returned nil"),
        }
    }
}

fn class(name: &'static std::ffi::CStr, label: &'static str) -> Result<&'static AnyClass, PostFailure> {
    AnyClass::get(name).ok_or(PostFailure::ClassMissing(label))
}

#[derive(Default)]
struct PosterCenter {
    center: Option<Retained<AnyObject>>,
    authorization_requested: bool,
    categories: HashMap<Vec<NotificationAction>, (String, Retained<AnyObject>)>,
    communication_refused: bool,
}

impl PosterCenter {
    fn run(&mut self, job: Job) {
        let result = match job {
            Job::Post(notification) => self.post(&notification),
            Job::Remove(tag) => self.remove(&tag),
            Job::RemoveAll => self.remove_all(),
            Job::RequestAuthorization => self.center().map(|center| self.request_authorization(&center)),
        };
        if let Err(failure) = result {
            tracing::warn!(target: "riseonly", %failure, "notification not handed to the system");
        }
    }

    fn center(&mut self) -> Result<Retained<AnyObject>, PostFailure> {
        if let Some(center) = &self.center {
            return Ok(center.clone());
        }
        let center_class = class(c"UNUserNotificationCenter", "UNUserNotificationCenter")?;
        let center: Option<Retained<AnyObject>> =
            unsafe { msg_send![center_class, currentNotificationCenter] };
        let center = center.ok_or(PostFailure::NilFrom("currentNotificationCenter"))?;
        self.center = Some(center.clone());
        Ok(center)
    }

    fn request_authorization(&mut self, center: &AnyObject) {
        if std::mem::replace(&mut self.authorization_requested, true) {
            return;
        }
        let block = AuthorizationBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: authorization_answered,
            descriptor: &raw const AUTHORIZATION_DESCRIPTOR,
        };
        unsafe {
            let _: () = msg_send![
                center,
                requestAuthorizationWithOptions: AUTHORIZATION_OPTIONS,
                completionHandler: &raw const block
            ];
        }
    }

    fn post(&mut self, notification: &LocalNotification) -> Result<(), PostFailure> {
        let center = self.center()?;
        self.request_authorization(&center);

        let category = if notification.actions.is_empty() {
            None
        } else {
            Some(self.register_category(&center, &notification.actions)?)
        };
        let content = base_content(notification, category.as_deref())?;
        let content = self.conversation_content(&content, notification)?;

        let request_class = class(c"UNNotificationRequest", "UNNotificationRequest")?;
        let identifier = NSString::from_str(notification.tag.as_str());
        let request: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                request_class,
                requestWithIdentifier: &*identifier,
                content: &*content,
                trigger: Option::<&AnyObject>::None
            ]
        };
        let request = request.ok_or(PostFailure::NilFrom("requestWithIdentifier"))?;
        unsafe {
            let _: () = msg_send![
                &*center,
                addNotificationRequest: &*request,
                withCompletionHandler: std::ptr::null::<NilBlock>()
            ];
        }
        Ok(())
    }

    fn remove(&mut self, tag: &NotificationTag) -> Result<(), PostFailure> {
        let center = self.center()?;
        let identifiers = NSArray::from_retained_slice(&[NSString::from_str(tag.as_str())]);
        unsafe {
            let _: () = msg_send![&*center, removePendingNotificationRequestsWithIdentifiers: &*identifiers];
            let _: () = msg_send![&*center, removeDeliveredNotificationsWithIdentifiers: &*identifiers];
        }
        Ok(())
    }

    fn remove_all(&mut self) -> Result<(), PostFailure> {
        let center = self.center()?;
        unsafe {
            let _: () = msg_send![&*center, removeAllPendingNotificationRequests];
            let _: () = msg_send![&*center, removeAllDeliveredNotifications];
        }
        Ok(())
    }

    fn register_category(
        &mut self,
        center: &AnyObject,
        actions: &[NotificationAction],
    ) -> Result<String, PostFailure> {
        if let Some((identifier, _)) = self.categories.get(actions) {
            return Ok(identifier.clone());
        }

        let action_class = class(c"UNNotificationAction", "UNNotificationAction")?;
        let text_input_class = class(c"UNTextInputNotificationAction", "UNTextInputNotificationAction")?;
        let category_class = class(c"UNNotificationCategory", "UNNotificationCategory")?;
        let mut platform_actions = Vec::with_capacity(actions.len());
        for action in actions {
            let identifier = NSString::from_str(&action.id);
            let title = NSString::from_str(&action.label);
            let platform: Option<Retained<AnyObject>> = match &action.text_input {
                Some(input) => unsafe {
                    msg_send![
                        text_input_class,
                        actionWithIdentifier: &*identifier,
                        title: &*title,
                        options: 0usize,
                        textInputButtonTitle: &*NSString::from_str(&input.button_title),
                        textInputPlaceholder: &*NSString::from_str(&input.placeholder)
                    ]
                },
                None => unsafe {
                    msg_send![
                        action_class,
                        actionWithIdentifier: &*identifier,
                        title: &*title,
                        options: 0usize
                    ]
                },
            };
            platform_actions.push(platform.ok_or(PostFailure::NilFrom("actionWithIdentifier"))?);
        }

        let identifier = format!("riseonly-notification-{}", self.categories.len());
        let platform_actions = NSArray::from_retained_slice(&platform_actions);
        let intent_identifiers = NSArray::<NSString>::new();
        let category: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                category_class,
                categoryWithIdentifier: &*NSString::from_str(&identifier),
                actions: &*platform_actions,
                intentIdentifiers: &*intent_identifiers,
                options: 0usize
            ]
        };
        let category = category.ok_or(PostFailure::NilFrom("categoryWithIdentifier"))?;
        self.categories
            .insert(actions.to_vec(), (identifier.clone(), category));

        let all: Vec<Retained<AnyObject>> = self
            .categories
            .values()
            .map(|(_, category)| category.clone())
            .collect();
        let all = NSSet::from_retained_slice(&all);
        unsafe {
            let _: () = msg_send![center, setNotificationCategories: &*all];
        }
        Ok(identifier)
    }

    fn conversation_content(
        &mut self,
        content: &Retained<AnyObject>,
        notification: &LocalNotification,
    ) -> Result<Retained<AnyObject>, PostFailure> {
        let avatar = notification.avatar.as_deref().and_then(read_avatar);
        if notification.sender.is_none() && avatar.is_none() {
            return Ok(content.clone());
        }

        if !self.communication_refused {
            let group_avatar = match notification.group_author_id() {
                Some(_) => notification.group_avatar.as_deref().and_then(read_avatar),
                None => None,
            };
            let pictures = Pictures { sender: avatar.as_deref(), group: group_avatar.as_deref() };
            match communication_content(content, notification, pictures) {
                Ok(updated) => return Ok(updated),
                Err(refusal) => {
                    self.communication_refused = true;
                    tracing::warn!(
                        target: "riseonly",
                        %refusal,
                        "communication notification refused, avatar goes on as an attachment"
                    );
                }
            }
        }

        if let Some(path) = notification.attachment.as_deref().or(notification.avatar.as_deref()) {
            attach_avatar(content, path);
        }
        Ok(content.clone())
    }
}

fn base_content(
    notification: &LocalNotification,
    category: Option<&str>,
) -> Result<Retained<AnyObject>, PostFailure> {
    let content_class = class(c"UNMutableNotificationContent", "UNMutableNotificationContent")?;
    let content: Option<Retained<AnyObject>> = unsafe { msg_send![content_class, new] };
    let content = content.ok_or(PostFailure::NilFrom("UNMutableNotificationContent new"))?;
    apply_text(&content, notification, category);

    if !SILENT {
        let sound_class = class(c"UNNotificationSound", "UNNotificationSound")?;
        let sound: Option<Retained<AnyObject>> = unsafe { msg_send![sound_class, defaultSound] };
        if let Some(sound) = sound {
            unsafe {
                let _: () = msg_send![&*content, setSound: &*sound];
            }
        }
    }
    Ok(content)
}

/// writes back the fields the content update may have replaced
fn apply_text(content: &AnyObject, notification: &LocalNotification, category: Option<&str>) {
    unsafe {
        let _: () = msg_send![content, setTitle: &*NSString::from_str(&notification.title)];
        let _: () = msg_send![content, setBody: &*NSString::from_str(&notification.body)];
        if let Some(thread) = notification.thread.as_deref() {
            let _: () = msg_send![content, setThreadIdentifier: &*NSString::from_str(thread)];
        }
        if let Some(category) = category {
            let _: () = msg_send![content, setCategoryIdentifier: &*NSString::from_str(category)];
        }
    }
}

fn read_avatar(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        Ok(_) => None,
        Err(error) => {
            tracing::debug!(target: "riseonly", %error, "notification avatar unreadable");
            None
        }
    }
}

fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

/// why the communication route gave no content
enum CommunicationRefusal {
    Unavailable(PostFailure),
    Refused { domain: String, code: isize, intents_domain: bool },
}

impl std::fmt::Display for CommunicationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(failure) => write!(f, "{failure}"),
            Self::Refused { domain, code, intents_domain } => {
                write!(f, "{domain} {code} (Intents domain: {intents_domain})")
            }
        }
    }
}

impl From<PostFailure> for CommunicationRefusal {
    fn from(failure: PostFailure) -> Self {
        Self::Unavailable(failure)
    }
}

fn person_handle(value: &str) -> Result<Retained<AnyObject>, PostFailure> {
    let handle_class = class(c"INPersonHandle", "INPersonHandle")?;
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![handle_class, alloc];
        let handle: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithValue: &*NSString::from_str(value),
            type: PERSON_HANDLE_UNKNOWN
        ];
        handle.ok_or(PostFailure::NilFrom("INPersonHandle"))
    }
}

/// image bytes a communication notification carries, the sender (member in a group) and the group
/// read from the files of the notification
#[derive(Clone, Copy)]
struct Pictures<'a> {
    sender: Option<&'a [u8]>,
    group: Option<&'a [u8]>,
}

/// makes an intent image from the bytes, no bytes or an image the system rejects is none
fn intent_image(bytes: Option<&[u8]>) -> Result<Option<Retained<AnyObject>>, PostFailure> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let image_class = class(c"INImage", "INImage")?;
    let data = NSData::with_bytes(bytes);
    let image: Option<Retained<AnyObject>> =
        unsafe { msg_send![image_class, imageWithImageData: &*data] };
    Ok(image)
}

fn communication_content(
    content: &Retained<AnyObject>,
    notification: &LocalNotification,
    pictures: Pictures<'_>,
) -> Result<Retained<AnyObject>, CommunicationRefusal> {
    let person_class = class(c"INPerson", "INPerson")?;
    let intent_class = class(c"INSendMessageIntent", "INSendMessageIntent")?;
    let speakable_class = class(c"INSpeakableString", "INSpeakableString")?;

    let conversation = notification
        .thread
        .clone()
        .unwrap_or_else(|| notification.tag.as_str().to_owned());
    let sender_name = notification.sender.as_deref().unwrap_or_default();
    let target_title = if !is_blank(&notification.title) {
        notification.title.as_str()
    } else if !sender_name.is_empty() {
        sender_name
    } else {
        FALLBACK_TITLE
    };
    let display_name = if sender_name.is_empty() {
        target_title
    } else {
        sender_name
    };

    let image = intent_image(pictures.sender)?;
    let group_author = notification.group_author_id();
    let group_image = match group_author {
        Some(_) => intent_image(pictures.group)?,
        None => None,
    };

    let conversation_id = NSString::from_str(&conversation);
    let sender_key = group_author.unwrap_or(conversation.as_str());
    let sender_id = NSString::from_str(sender_key);
    let sender_handle = person_handle(sender_key)?;
    let sender: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![person_class, alloc];
        msg_send![
            allocated,
            initWithPersonHandle: &*sender_handle,
            nameComponents: Option::<&AnyObject>::None,
            displayName: &*NSString::from_str(display_name),
            image: image.as_deref(),
            contactIdentifier: Option::<&NSString>::None,
            customIdentifier: &*sender_id
        ]
    };
    let sender = sender.ok_or(PostFailure::NilFrom("INPerson sender"))?;

    let me_handle = person_handle(ME_IDENTIFIER)?;
    let me: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![person_class, alloc];
        msg_send![
            allocated,
            initWithPersonHandle: &*me_handle,
            nameComponents: Option::<&AnyObject>::None,
            displayName: Option::<&NSString>::None,
            image: Option::<&AnyObject>::None,
            contactIdentifier: Option::<&NSString>::None,
            customIdentifier: &*NSString::from_str(ME_IDENTIFIER),
            isMe: Bool::YES,
            suggestionType: SUGGESTION_NONE
        ]
    };
    let me = me.ok_or(PostFailure::NilFrom("INPerson me"))?;

    let group_name: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![speakable_class, alloc];
        msg_send![allocated, initWithSpokenPhrase: &*NSString::from_str(target_title)]
    };
    let group_name = group_name.ok_or(PostFailure::NilFrom("INSpeakableString"))?;

    let mut recipients = vec![me];
    if group_author.is_some() {
        let group_handle = person_handle(&conversation)?;
        let group: Option<Retained<AnyObject>> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![person_class, alloc];
            msg_send![
                allocated,
                initWithPersonHandle: &*group_handle,
                nameComponents: Option::<&AnyObject>::None,
                displayName: &*NSString::from_str(target_title),
                image: group_image.as_deref(),
                contactIdentifier: Option::<&NSString>::None,
                customIdentifier: &*conversation_id
            ]
        };
        recipients.push(group.ok_or(PostFailure::NilFrom("INPerson group"))?);
    }
    let recipients = NSArray::from_retained_slice(&recipients);
    let body = NSString::from_str(&notification.body);
    let intent: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![intent_class, alloc];
        msg_send![
            allocated,
            initWithRecipients: &*recipients,
            outgoingMessageType: OUTGOING_MESSAGE_TEXT,
            content: &*body,
            speakableGroupName: &*group_name,
            conversationIdentifier: &*conversation_id,
            serviceName: Option::<&NSString>::None,
            sender: &*sender,
            attachments: Option::<&AnyObject>::None
        ]
    };
    let intent = intent.ok_or(PostFailure::NilFrom("INSendMessageIntent"))?;

    let parameter_images = [("sender", image.as_deref()), ("speakableGroupName", group_image.as_deref())];
    for (parameter, picture) in parameter_images {
        let Some(picture) = picture else {
            continue;
        };
        unsafe {
            let _: () = msg_send![&*intent, setImage: picture, forParameterNamed: &*NSString::from_str(parameter)];
        }
    }

    let updated: Result<Retained<AnyObject>, Retained<NSError>> =
        unsafe { msg_send![&**content, contentByUpdatingWithProvider: &*intent, error: _] };
    let updated = match updated {
        Ok(updated) => updated,
        Err(error) => {
            let domain = error.domain();
            let intents_domain = *domain == *unsafe { INIntentErrorDomain };
            return Err(CommunicationRefusal::Refused {
                domain: domain.to_string(),
                code: error.code(),
                intents_domain,
            });
        }
    };

    let mutable: Option<Retained<AnyObject>> = unsafe { msg_send![&*updated, mutableCopy] };
    let mutable = mutable.ok_or(PostFailure::NilFrom("mutableCopy"))?;
    let category: Retained<NSString> = unsafe { msg_send![&**content, categoryIdentifier] };
    let category = category.to_string();
    apply_text(
        &mutable,
        notification,
        (!category.is_empty()).then_some(category.as_str()),
    );
    unsafe {
        let sound: Option<Retained<AnyObject>> = msg_send![&**content, sound];
        let _: () = msg_send![&*mutable, setSound: sound.as_deref()];
    }
    Ok(mutable)
}

fn staged_attachment(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");
    std::env::temp_dir()
        .join("riseonly-notification-attachments")
        .join(format!("{}-{n}.{extension}", std::process::id()))
}

fn attach_avatar(content: &AnyObject, path: &Path) {
    let staged = staged_attachment(path);
    let staged_ok = staged
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| std::fs::hard_link(path, &staged).or_else(|_| std::fs::copy(path, &staged).map(|_| ())));
    if let Err(error) = staged_ok {
        tracing::debug!(target: "riseonly", %error, "notification avatar not staged");
        return;
    }

    let Some(attachment_class) = AnyClass::get(c"UNNotificationAttachment") else {
        return;
    };
    let url = NSURL::from_file_path(&staged);
    let Some(url) = url else {
        return;
    };
    let attachment: Result<Retained<AnyObject>, Retained<NSError>> = unsafe {
        msg_send![
            attachment_class,
            attachmentWithIdentifier: &*NSString::from_str("avatar"),
            URL: &*url,
            options: Option::<&AnyObject>::None,
            error: _
        ]
    };
    match attachment {
        Ok(attachment) => {
            let attachments = NSArray::from_retained_slice(&[attachment]);
            unsafe {
                let _: () = msg_send![content, setAttachments: &*attachments];
            }
        }
        Err(error) => {
            if let Err(removal) = std::fs::remove_file(&staged) {
                tracing::debug!(target: "riseonly", %removal, "staged notification avatar left behind");
            }
            tracing::debug!(
                target: "riseonly",
                error = %error.localizedDescription(),
                "notification avatar attachment refused"
            );
        }
    }
}
