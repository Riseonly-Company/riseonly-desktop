use futures::channel::oneshot;
use gpui::{App, Bounds, ClipboardItem, point, px, size};
use rise_platform::system_share::{ShareFinished, ShareItem, ShareOutcome, ShareRequest, share};

use crate::core::widgets::notifier::notifier_store::Notifier;

const SYSTEM_SHARE_SOURCE_BOTTOM_INSET: f32 = 40.0;

pub enum SystemShare {}

impl SystemShare {
    pub fn present(items: Vec<ShareItem>, completion: Option<Box<dyn FnOnce(&mut App)>>, cx: &mut App) -> bool {
        Self::present_reporting(items, None, completion, cx)
    }

    /// presents items to share, no system sheet on linux so text goes to the clipboard
    /// notifier says so, false when no window is open to show it
    pub fn present_reporting(
        items: Vec<ShareItem>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        completion: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut App,
    ) -> bool {
        let Some(window_handle) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
            .or_else(|| cx.windows().first().copied())
        else {
            return false;
        };
        cx.defer(move |cx| {
            let (sender, receiver) = oneshot::channel::<ShareFinished>();
            let outcome = window_handle.update(cx, |_, window, _| {
                let viewport = window.viewport_size();
                let anchor = Bounds::new(
                    point(viewport.width / 2.0, viewport.height - px(SYSTEM_SHARE_SOURCE_BOTTOM_INSET)),
                    size(px(0.0), px(0.0)),
                );
                match ShareRequest::new(items, anchor) {
                    Ok(request) => share(window, &request, move |finished| {
                        let _ = sender.send(finished);
                    }),
                    Err(reason) => ShareOutcome::Unavailable { reason, copy: None },
                }
            });
            match outcome {
                Ok(ShareOutcome::Presented) => {
                    cx.spawn(async move |cx| {
                        let succeeded = receiver.await.is_ok_and(|finished| finished.succeeded);
                        cx.update(|cx| {
                            if succeeded && let Some(on_success) = on_success {
                                on_success(cx);
                            }
                            if let Some(completion) = completion {
                                completion(cx);
                            }
                        });
                    })
                    .detach();
                }
                Ok(ShareOutcome::Unavailable { copy: Some(text), .. }) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    Notifier::copied(cx);
                    if let Some(completion) = completion {
                        completion(cx);
                    }
                }
                Ok(ShareOutcome::Unavailable { copy: None, .. }) | Err(_) => {
                    if let Some(completion) = completion {
                        completion(cx);
                    }
                }
            }
        });
        true
    }
}
