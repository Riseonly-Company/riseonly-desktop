use gpui::{TestAppContext, VisualContext};

use super::*;

fn is_active(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| StickerHoldPreviewPresenter::shared(cx).read(cx).is_active())
}

fn show_preview(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update(|cx| {
        StickerHoldPreviewPresenter::shared(cx).update(cx, |presenter, _| {
            presenter.asset = Some(StickerPreviewAsset::UnicodeEmoji("x".to_owned()));
            presenter.presenting_window = Some(window);
        });
    });
}

#[gpui::test]
fn escape_in_another_window_leaves_the_preview_open(cx: &mut TestAppContext) {
    let preview_window = cx.add_empty_window().window_handle();
    let call_window = cx.add_empty_window().window_handle();
    show_preview(preview_window, cx);

    cx.simulate_keystrokes(call_window, "escape");

    assert!(is_active(cx));
}

#[gpui::test]
fn escape_in_the_presenting_window_closes_the_preview(cx: &mut TestAppContext) {
    let preview_window = cx.add_empty_window().window_handle();
    show_preview(preview_window, cx);

    cx.simulate_keystrokes(preview_window, "escape");

    assert!(!is_active(cx));
}
