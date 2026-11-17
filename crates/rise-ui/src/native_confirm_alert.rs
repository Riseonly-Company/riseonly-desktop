use gpui::{AnyWindowHandle, App, Global, PromptButton, PromptLevel};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::open_url::{OPEN_SETTINGS_URL_STRING, open_url};

use crate::guide_overlay_ui::guide_overlay_window::GuideOverlayHost;

/// a button work, run after the dialog is gone
pub type NativeConfirmAlertAction = Box<dyn FnOnce(&mut App)>;

/// window the open dialog belongs to, none while no dialog is up
#[derive(Default)]
struct NativeConfirmAlertWindow {
    window: Option<AnyWindowHandle>,
}

impl Global for NativeConfirmAlertWindow {}

/// what one of the dialog buttons means, the dialog answers with its index
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeConfirmAlertAnswer {
    Cancel,
    Confirm,
    Secondary,
    Choice(usize),
}

/// system two button destructive confirmation, callable from a store as well as a view
/// delete paths of one entity live in menus, hold menus and row routers at once
/// modal dialog of the active window, escape cancels and enter confirms
pub struct NativeConfirmAlert;

impl NativeConfirmAlert {
    /// confirm_title none means the default delete word of the menu
    #[allow(clippy::too_many_arguments)]
    pub fn present_delete(
        title: &str,
        message: &str,
        confirm_title: Option<String>,
        secondary_title: Option<String>,
        secondary_is_destructive: bool,
        on_secondary: Option<NativeConfirmAlertAction>,
        on_cancel: Option<NativeConfirmAlertAction>,
        on_confirm: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        Self::release_if_orphaned(cx);
        if Self::is_presenting(cx) {
            return;
        }
        let Some(host) = Self::make_host_root(cx) else {
            if let Some(on_cancel) = on_cancel {
                on_cancel(cx);
            }
            return;
        };

        let confirm_title = confirm_title.unwrap_or_else(|| tr("ctx_delete"));
        let mut answers = vec![(NativeConfirmAlertAnswer::Confirm, PromptButton::ok(confirm_title))];
        let secondary = match (secondary_title, on_secondary) {
            (Some(secondary_title), Some(on_secondary)) => {
                answers.push((
                    NativeConfirmAlertAnswer::Secondary,
                    PromptButton::new(secondary_title),
                ));
                Some(on_secondary)
            }
            _ => None,
        };
        answers.push((NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("common_cancel"))));

        Self::present(
            host,
            PromptLevel::Warning,
            title,
            message,
            answers,
            move |answer, cx| match answer {
                NativeConfirmAlertAnswer::Confirm => {
                    let _ = notification(NotificationKind::Warning);
                    on_confirm(cx);
                }
                NativeConfirmAlertAnswer::Secondary => {
                    if let Some(on_secondary) = secondary {
                        if secondary_is_destructive {
                            let _ = notification(NotificationKind::Warning);
                        }
                        on_secondary(cx);
                    }
                }
                NativeConfirmAlertAnswer::Cancel | NativeConfirmAlertAnswer::Choice(_) => {
                    if let Some(on_cancel) = on_cancel {
                        on_cancel(cx);
                    }
                }
            },
            cx,
        );
    }

    /// confirmation whose buttons are all destructive choices plus cancel
    /// eg block tiers next to block and report, the list keeps its order
    pub fn present_destructive_choices(
        title: &str,
        message: &str,
        choices: Vec<(String, NativeConfirmAlertAction)>,
        on_cancel: Option<NativeConfirmAlertAction>,
        cx: &mut App,
    ) {
        Self::release_if_orphaned(cx);
        if Self::is_presenting(cx) {
            return;
        }
        let Some(host) = Self::make_host_root(cx) else {
            if let Some(on_cancel) = on_cancel {
                on_cancel(cx);
            }
            return;
        };
        let mut answers = Vec::with_capacity(choices.len() + 1);
        let mut actions = Vec::with_capacity(choices.len());
        for (index, (title, action)) in choices.into_iter().enumerate() {
            let button = if index == 0 { PromptButton::ok(title) } else { PromptButton::new(title) };
            answers.push((NativeConfirmAlertAnswer::Choice(index), button));
            actions.push(Some(action));
        }
        answers.push((NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("common_cancel"))));
        Self::present(
            host,
            PromptLevel::Warning,
            title,
            message,
            answers,
            move |answer, cx| {
                let mut actions = actions;
                let action = match answer {
                    NativeConfirmAlertAnswer::Choice(index) => actions.get_mut(index).and_then(Option::take),
                    _ => None,
                };
                match action {
                    Some(action) => {
                        let _ = notification(NotificationKind::Warning);
                        action(cx);
                    }
                    None => {
                        if let Some(on_cancel) = on_cancel {
                            on_cancel(cx);
                        }
                    }
                }
            },
            cx,
        );
    }

    /// for a permission the person already refused, the os wont ask again and only settings can grant it
    /// during a guide it goes over the guide dim
    pub fn present_open_settings(title: &str, message: &str, cx: &mut App) {
        Self::release_if_orphaned(cx);
        if Self::is_presenting(cx) {
            return;
        }
        let answers = vec![
            (
                NativeConfirmAlertAnswer::Confirm,
                PromptButton::ok(tr("media_picker_open_settings")),
            ),
            (NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("common_cancel"))),
        ];
        let host = if GuideOverlayHost::shared(cx).is_presenting() {
            GuideOverlayHost::shared(cx).alert_host()
        } else {
            Self::make_host_root(cx)
        };
        let Some(host) = host else {
            return;
        };
        Self::present(
            host,
            PromptLevel::Info,
            title,
            message,
            answers,
            |answer, cx| {
                if answer != NativeConfirmAlertAnswer::Confirm {
                    return;
                }
                if let Some(url) = Url::from_string(OPEN_SETTINGS_URL_STRING) {
                    let _ = open_url(&url, cx);
                }
            },
            cx,
        );
    }

    /// plain question with one non destructive action, eg joining a voice channel named in a message
    pub fn present_confirm(
        title: &str,
        message: &str,
        confirm_title: &str,
        on_confirm: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        Self::release_if_orphaned(cx);
        if Self::is_presenting(cx) {
            return;
        }
        let Some(host) = Self::make_host_root(cx) else {
            return;
        };
        let answers = vec![
            (NativeConfirmAlertAnswer::Confirm, PromptButton::ok(confirm_title.to_owned())),
            (NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("common_cancel"))),
        ];
        Self::present(
            host,
            PromptLevel::Info,
            title,
            message,
            answers,
            move |answer, cx| {
                if answer == NativeConfirmAlertAnswer::Confirm {
                    on_confirm(cx);
                }
            },
            cx,
        );
    }

    /// a failure that finished after its screen was gone
    /// one acknowledgement with a retry beside it when work can resume
    pub fn present_notice(
        title: &str,
        message: &str,
        retry_title: Option<String>,
        on_retry: Option<NativeConfirmAlertAction>,
        on_dismiss: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        Self::release_if_orphaned(cx);
        if Self::is_presenting(cx) {
            return;
        }
        let Some(host) = Self::make_host_root(cx) else {
            return;
        };
        let (answers, retry) = match (retry_title, on_retry) {
            (Some(retry_title), Some(on_retry)) => (
                vec![
                    (NativeConfirmAlertAnswer::Confirm, PromptButton::ok(retry_title)),
                    (NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("common_cancel"))),
                ],
                Some(on_retry),
            ),
            _ => (
                vec![(NativeConfirmAlertAnswer::Cancel, PromptButton::cancel(tr("ok")))],
                None,
            ),
        };
        Self::present(
            host,
            PromptLevel::Info,
            title,
            message,
            answers,
            move |answer, cx| match (answer, retry) {
                (NativeConfirmAlertAnswer::Confirm, Some(on_retry)) => on_retry(cx),
                _ => on_dismiss(cx),
            },
            cx,
        );
    }

    /// true when a dialog of this type is up
    pub fn is_presenting(cx: &App) -> bool {
        cx.try_global::<NativeConfirmAlertWindow>()
            .is_some_and(|state| state.window.is_some())
    }

    fn present(
        host: AnyWindowHandle,
        level: PromptLevel,
        title: &str,
        message: &str,
        answers: Vec<(NativeConfirmAlertAnswer, PromptButton)>,
        on_answer: impl FnOnce(NativeConfirmAlertAnswer, &mut App) + 'static,
        cx: &mut App,
    ) {
        let buttons: Vec<PromptButton> = answers.iter().map(|(_, button)| button.clone()).collect();
        let meanings: Vec<NativeConfirmAlertAnswer> =
            answers.iter().map(|(meaning, _)| *meaning).collect();
        let title = title.to_string();
        let message = message.to_string();
        cx.default_global::<NativeConfirmAlertWindow>().window = Some(host);

        cx.defer(move |cx| {
            let detail = (!message.is_empty()).then_some(message.as_str());
            let receiver = host.update(cx, |_, window, cx| {
                window.prompt(level, &title, detail, &buttons, cx)
            });
            let Ok(receiver) = receiver else {
                Self::release(cx);
                on_answer(NativeConfirmAlertAnswer::Cancel, cx);
                return;
            };

            cx.spawn(async move |cx| {
                let answer = receiver.await;
                cx.update(|cx| {
                    Self::release(cx);
                    let meaning = answer
                        .ok()
                        .and_then(|index| meanings.get(index).copied())
                        .unwrap_or(NativeConfirmAlertAnswer::Cancel);
                    on_answer(meaning, cx);
                });
            })
            .detach();
        });
    }

    fn make_host_root(cx: &App) -> Option<AnyWindowHandle> {
        cx.active_window().or_else(|| cx.windows().first().copied())
    }

    /// a dialog taken down without one of its buttons (window closed) leaves state behind and
    /// every later confirmation would be dropped
    fn release_if_orphaned(cx: &mut App) {
        let Some(window) = cx
            .try_global::<NativeConfirmAlertWindow>()
            .and_then(|state| state.window)
        else {
            return;
        };
        let alive = cx
            .windows()
            .iter()
            .any(|open| open.window_id() == window.window_id());
        if !alive {
            Self::release(cx);
        }
    }

    fn release(cx: &mut App) {
        cx.default_global::<NativeConfirmAlertWindow>().window = None;
    }
}

#[cfg(test)]
#[path = "native_confirm_alert_tests.rs"]
mod tests;
