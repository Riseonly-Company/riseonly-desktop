use gpui::{
    App, Context, InteractiveElement, IntoElement, ParentElement, RenderOnce, Styled, Subscription, Window, black,
    div, px, white,
};
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::alpha;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

#[derive(IntoElement, Default)]
pub struct FeedSoundButton;

impl FeedSoundButton {
    pub fn new() -> Self {
        Self
    }
}

/// redraws the view around the button when the store changes
struct FeedSoundButtonStoreObserver {
    _shorts_store: Subscription,
}

impl RenderOnce for FeedSoundButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_state(cx, |_, cx: &mut Context<FeedSoundButtonStoreObserver>| {
            let shorts_store = ShortsServicesStore::shared(cx);
            FeedSoundButtonStoreObserver {
                _shorts_store: cx.observe(&shorts_store, |_, _, cx| cx.notify()),
            }
        });
        let is_muted = ShortsServicesStore::shared(cx).read(cx).feed_audio_muted;
        let glyph = if is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" };
        let label = tr(if is_muted { "feed_sound_unmute" } else { "feed_sound_mute" });
        div()
            .id("feed-sound-button-target")
            .occlude()
            .tooltip_label(label)
            .child(
                plain("feed-sound-button")
                    .on_press(|_, cx| {
                        let _ = selection_changed();
                        ShortsServicesStore::shared(cx).update(cx, |store, cx| {
                            store.feed_audio_muted = !store.feed_audio_muted;
                            cx.notify();
                        });
                    })
                    .size(px(48.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(34.0))
                            .circle()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(alpha(black(), 0.45))
                            .children(IconUi::sized(glyph, px(14.0), white(), true)),
                    ),
            )
    }
}
