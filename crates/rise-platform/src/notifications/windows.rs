use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use futures::StreamExt as _;
use futures::channel::mpsc;
use ::windows::Data::Xml::Dom::XmlDocument;
use ::windows::Foundation::{IPropertyValue, TypedEventHandler};
use ::windows::UI::Notifications::{
    NotificationSetting, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};
use ::windows::Win32::Foundation::{
    APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, WIN32_ERROR,
};
use ::windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use ::windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use ::windows::core::{HSTRING, IInspectable, Interface as _, PCWSTR};

use crate::app_info::AppIdentity;
use crate::gpui_shim::PlatformSupport;
use crate::notifications::toast_document::*;
use crate::notifications::*;

const SILENT: bool = cfg!(debug_assertions);

static IDENTITY: OnceLock<AppIdentity> = OnceLock::new();

pub(super) fn host_precondition_met() -> bool {
    has_package_identity() || IDENTITY.get().is_some()
}

/// one activation, raised on a winrt thread pool thread and delivered on the main thread
struct Activation {
    tag: String,
    action_id: Option<String>,
    user_text: Option<String>,
}

struct Responses {
    sender: mpsc::UnboundedSender<Activation>,
    pending: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Activation>>>,
}

fn responses() -> &'static Responses {
    static RESPONSES: OnceLock<Responses> = OnceLock::new();
    RESPONSES.get_or_init(|| {
        let (sender, receiver) = mpsc::unbounded();
        Responses {
            sender,
            pending: parking_lot::Mutex::new(Some(receiver)),
        }
    })
}

#[derive(Default)]
struct Posted {
    notifier: Option<ToastNotifier>,
    live: HashMap<String, ToastNotification>,
}

thread_local! {
    static POSTED: RefCell<Posted> = RefCell::new(Posted::default());
    static RESPONSE_CALLBACK: RefCell<Option<NotificationActionResponseCallback>> =
        const { RefCell::new(None) };
}

pub(super) struct NativeNotifications;

impl NativeNotificationRoute for NativeNotifications {
    fn present(&self, _app: &gpui::App, notification: &LocalNotification) -> PlatformSupport {
        post(notification).unwrap_or_else(|error| {
            tracing::warn!(%error, "notifications: the toast was refused");
            PlatformSupport::Unsupported
        })
    }

    fn dismiss(&self, _app: &gpui::App, tag: &NotificationTag) -> PlatformSupport {
        retract(tag).unwrap_or_else(|error| {
            tracing::warn!(%error, "notifications: the toast could not be removed");
            PlatformSupport::Unsupported
        })
    }

    fn dismiss_all(&self, _app: &gpui::App) -> PlatformSupport {
        clear_all().unwrap_or_else(|error| {
            tracing::warn!(%error, "notifications: the toasts could not be cleared");
            PlatformSupport::Unsupported
        })
    }

    fn on_response(&self, app: &gpui::App, mut callback: NotificationResponseCallback) {
        self.on_action_response(
            app,
            Box::new(move |response, app| {
                let response = gpui::SystemNotificationResponse {
                    tag: response.tag,
                    action_id: response.action_id,
                };
                callback(response, app);
            }),
        );
    }

    fn on_action_response(&self, app: &gpui::App, callback: NotificationActionResponseCallback) {
        RESPONSE_CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));
        let Some(mut activations) = responses().pending.lock().take() else {
            return;
        };
        app.spawn(async move |cx| {
            while let Some(activation) = activations.next().await {
                deliver(cx, activation);
            }
        })
        .detach();
    }

    fn reply_input_support(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn take_service(&self, identity: &AppIdentity) -> Option<NotificationService> {
        IDENTITY.set(*identity).ok()?;
        if has_package_identity() {
            return None;
        }
        set_process_app_user_model_id(identity);
        let identity = *identity;
        Some(Box::pin(async move {
            if let Err(status) = register_app_user_model_id(&identity) {
                tracing::warn!(?status, "notifications: the AUMID registration failed");
            }
        }))
    }

    fn authorization_status(&self) -> NotificationAuthorizationQuery {
        Box::pin(async move {
            let setting = create_notifier()
                .and_then(|notifier| notifier.map(|notifier| notifier.Setting()).transpose());
            match setting {
                Ok(Some(setting)) => Some(authorization_from_setting(setting)),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(%error, "notifications: the toast setting could not be read");
                    None
                }
            }
        })
    }
}

fn authorization_from_setting(setting: NotificationSetting) -> NotificationAuthorizationStatus {
    if setting == NotificationSetting::Enabled {
        NotificationAuthorizationStatus::Authorized
    } else {
        NotificationAuthorizationStatus::Denied
    }
}

fn post(notification: &LocalNotification) -> ::windows::core::Result<PlatformSupport> {
    let Some(notifier) = notifier()? else {
        return Ok(PlatformSupport::Unsupported);
    };
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(notification, SILENT)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.SetTag(&HSTRING::from(toast_tag(&notification.tag)))?;
    toast.SetGroup(&HSTRING::from(TOAST_GROUP))?;

    let sender = responses().sender.clone();
    let tag = notification.tag.as_str().to_owned();
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_toast, arguments| {
            let arguments = arguments
                .as_ref()
                .and_then(|arguments| arguments.cast::<ToastActivatedEventArgs>().ok());
            let action_id = arguments
                .as_ref()
                .and_then(|arguments| arguments.Arguments().ok())
                .and_then(|arguments| action_from_arguments(&arguments.to_string()));
            let user_text = arguments
                .as_ref()
                .zip(action_id.as_deref())
                .and_then(|(arguments, action_id)| typed_text(arguments, action_id));
            sender
                .unbounded_send(Activation {
                    tag: tag.clone(),
                    action_id,
                    user_text,
                })
                .ok();
            Ok(())
        },
    ))?;

    notifier.Show(&toast)?;
    POSTED.with(|posted| {
        posted
            .borrow_mut()
            .live
            .insert(notification.tag.as_str().to_owned(), toast)
    });
    Ok(PlatformSupport::Performed)
}

fn typed_text(arguments: &ToastActivatedEventArgs, action_id: &str) -> Option<String> {
    let value = arguments.UserInput().ok()?.Lookup(&HSTRING::from(input_id(action_id))).ok()?;
    let text = value.cast::<IPropertyValue>().ok()?.GetString().ok()?;
    typed_reply(&text.to_string())
}

fn retract(tag: &NotificationTag) -> ::windows::core::Result<PlatformSupport> {
    let live = POSTED.with(|posted| {
        let mut posted = posted.borrow_mut();
        let toast = posted.live.remove(tag.as_str())?;
        Some((posted.notifier.clone()?, toast))
    });
    if let Some((notifier, toast)) = live {
        notifier.Hide(&toast)?;
    }
    let history = ToastNotificationManager::History()?;
    let windows_tag = HSTRING::from(toast_tag(tag));
    let group = HSTRING::from(TOAST_GROUP);
    if has_package_identity() {
        history.RemoveGroupedTag(&windows_tag, &group)?;
    } else if let Some(identity) = IDENTITY.get() {
        history.RemoveGroupedTagWithId(
            &windows_tag,
            &group,
            &HSTRING::from(identity.bundle_identifier),
        )?;
    } else {
        return Ok(PlatformSupport::Unsupported);
    }
    Ok(PlatformSupport::Performed)
}

fn clear_all() -> ::windows::core::Result<PlatformSupport> {
    let (notifier, live) = POSTED.with(|posted| {
        let mut posted = posted.borrow_mut();
        let live: Vec<ToastNotification> = posted.live.drain().map(|(_, toast)| toast).collect();
        (posted.notifier.clone(), live)
    });
    if let Some(notifier) = notifier {
        for toast in &live {
            if let Err(error) = notifier.Hide(toast) {
                tracing::debug!(%error, "notifications: a toast was already gone");
            }
        }
    }
    let history = ToastNotificationManager::History()?;
    if has_package_identity() {
        history.Clear()?;
    } else if let Some(identity) = IDENTITY.get() {
        history.ClearWithId(&HSTRING::from(identity.bundle_identifier))?;
    } else {
        return Ok(PlatformSupport::Unsupported);
    }
    Ok(PlatformSupport::Performed)
}

fn notifier() -> ::windows::core::Result<Option<ToastNotifier>> {
    if let Some(notifier) = POSTED.with(|posted| posted.borrow().notifier.clone()) {
        return Ok(Some(notifier));
    }
    let Some(notifier) = create_notifier()? else {
        tracing::warn!("notifications: no AUMID yet, take_notification_service was not called");
        return Ok(None);
    };
    POSTED.with(|posted| posted.borrow_mut().notifier = Some(notifier.clone()));
    Ok(Some(notifier))
}

/// notifier under the package identity, or under the process aumid once its known
fn create_notifier() -> ::windows::core::Result<Option<ToastNotifier>> {
    if has_package_identity() {
        return ToastNotificationManager::CreateToastNotifier().map(Some);
    }
    let Some(identity) = IDENTITY.get() else {
        return Ok(None);
    };
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identity.bundle_identifier))
        .map(Some)
}

fn deliver(cx: &mut gpui::AsyncApp, activation: Activation) {
    let response = NotificationActionResponse {
        tag: activation.tag.into(),
        action_id: activation.action_id.map(Into::into),
        user_text: activation.user_text,
    };
    let Some(mut callback) = RESPONSE_CALLBACK.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    cx.update(|app| callback(response, app));
    RESPONSE_CALLBACK.with(|slot| {
        slot.borrow_mut().get_or_insert(callback);
    });
}

fn has_package_identity() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut length = 0u32;
        let status = unsafe { GetCurrentPackageFullName(&raw mut length, None) };
        match status {
            ERROR_INSUFFICIENT_BUFFER => true,
            APPMODEL_ERROR_NO_PACKAGE => false,
            other => {
                tracing::warn!(?other, "notifications: package identity is unknown");
                false
            }
        }
    })
}

fn set_process_app_user_model_id(identity: &AppIdentity) {
    let app_id = HSTRING::from(identity.bundle_identifier);
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(&app_id) } {
        tracing::warn!(%error, "notifications: the process AUMID was refused");
    }
}

fn register_app_user_model_id(identity: &AppIdentity) -> Result<(), WIN32_ERROR> {
    let subkey = HSTRING::from(format!(
        r"Software\Classes\AppUserModelId\{}",
        identity.bundle_identifier
    ));
    let mut handle = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &raw mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(status);
    }
    let value: Vec<u8> = identity
        .display_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let status = unsafe {
        RegSetValueExW(handle, &HSTRING::from("DisplayName"), None, REG_SZ, Some(value.as_slice()))
    };
    let _ = unsafe { RegCloseKey(handle) };
    if status == ERROR_SUCCESS { Ok(()) } else { Err(status) }
}
