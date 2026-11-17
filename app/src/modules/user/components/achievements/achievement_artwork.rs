use gpui::{App, ElementId, Hsla, ObjectFit, SharedString, StyledImage, Window, img, size};
use rise_canon::rise_maintenance::BUILTIN_MEDIA;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_platform::power::system_low_power_mode;
use rise_theme::color;
use rise_ui::icon_ui::IconUi;
use rise_ui::lottie_view::LottieView;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::{RiseAchievement, RiseAchievementMediaKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AchievementArtworkPolicyResolved {
    Builtin(String),
    RemoteLottie(String),
    Image(String),
    Fallback,
}

/// resolves achievement media
/// newer kinds decode as builtin, bad url counts as missing
/// both fall back to the trophy
pub enum AchievementArtworkPolicy {}

impl AchievementArtworkPolicy {
    pub const BUILTIN_NAMES: [&'static str; BUILTIN_MEDIA.len()] = BUILTIN_MEDIA;

    pub fn resolve(achievement: &RiseAchievement) -> AchievementArtworkPolicyResolved {
        let reference = trimming_whitespaces_and_newlines(&achievement.media_ref);
        if reference.is_empty() {
            return AchievementArtworkPolicyResolved::Fallback;
        }

        match achievement.media_kind {
            RiseAchievementMediaKind::Builtin => {
                if Self::BUILTIN_NAMES.contains(&reference) {
                    AchievementArtworkPolicyResolved::Builtin(reference.to_owned())
                } else {
                    AchievementArtworkPolicyResolved::Fallback
                }
            }
            RiseAchievementMediaKind::Lottie => {
                if Self::is_remote(reference) {
                    AchievementArtworkPolicyResolved::RemoteLottie(reference.to_owned())
                } else {
                    AchievementArtworkPolicyResolved::Fallback
                }
            }
            RiseAchievementMediaKind::Image => {
                if Self::is_remote(reference) {
                    AchievementArtworkPolicyResolved::Image(reference.to_owned())
                } else {
                    AchievementArtworkPolicyResolved::Fallback
                }
            }
        }
    }

    pub fn accent_color(achievement: &RiseAchievement) -> Option<Hsla> {
        let raw = trimming_whitespaces_and_newlines(&achievement.accent_color);
        if character_count(raw) != 7 || !raw.starts_with('#') {
            return None;
        }

        Some(color(raw))
    }

    fn is_remote(value: &str) -> bool {
        value.to_lowercase().starts_with("http")
    }
}

#[derive(IntoElement)]
pub struct AchievementArtwork {
    achievement: RiseAchievement,
    side: f32,
    loops: bool,
}

impl AchievementArtwork {
    pub fn new(achievement: RiseAchievement, side: f32) -> Self {
        Self { achievement, side, loops: true }
    }

    pub fn loops(mut self, loops: bool) -> Self {
        self.loops = loops;
        self
    }

    /// grid mounts a dozen of these and builtin plays directly with no hub to throttle
    /// so this gate adds low power mode and power saving
    fn loops_now(&self, cx: &mut App) -> bool {
        if !self.loops
            || reduce_motion(cx)
            || system_low_power_mode()
            || RiseEnergySavingStore::shared(cx)
                .read(cx)
                .reduces(RiseEnergySavingFeature::InterfaceEffects)
        {
            return false;
        }
        true
    }

    fn child_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("achievement-artwork-{}-{suffix}", self.achievement.id)))
    }

    fn remote_lottie(&self, url: String, loops_now: bool, window: &mut Window, cx: &mut App) -> AnyElement {
        let remote_first_frame_loaded = window.use_keyed_state(self.child_id("first-frame"), cx, |_, _| false);
        let is_loaded = *remote_first_frame_loaded.read(cx);

        div()
            .relative()
            .flex_none()
            .size(px(self.side))
            .overflow_hidden()
            .when(!is_loaded, |frame| frame.children(self.poster(&url)))
            .child(
                div().absolute().inset_0().child(
                    LottieStickerView::new(self.child_id("sticker"), url)
                        .size(self.side)
                        .r#loop(loops_now)
                        .playback_id(Some(format!("achievement-{}", self.achievement.id)))
                        .on_first_frame(move |cx| {
                            remote_first_frame_loaded.update(cx, |loaded, cx| {
                                *loaded = true;
                                cx.notify();
                            });
                        }),
                ),
            )
            .into_any_element()
    }

    fn poster(&self, url: &str) -> Option<AnyElement> {
        let image = RiseLottieFirstFrameCache::memory_image(
            url,
            size(px(self.side), px(self.side)),
            SaiAccountCacheScope::active_user_id().as_deref(),
        )?;
        Some(
            div()
                .absolute()
                .inset_0()
                .child(img(image).size_full().object_fit(ObjectFit::Contain))
                .into_any_element(),
        )
    }

    fn fallback(&self, cx: &App) -> Option<AnyElement> {
        let theme = rise_ui::theme(cx);
        let accent = AchievementArtworkPolicy::accent_color(&self.achievement).unwrap_or(theme.primary._100);
        let icon = IconUi::sized("trophy.fill", px(self.side * 0.38), accent, true)?;
        Some(icon.into_any_element())
    }
}

impl RenderOnce for AchievementArtwork {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = match AchievementArtworkPolicy::resolve(&self.achievement) {
            AchievementArtworkPolicyResolved::Builtin(name) => {
                let loops_now = self.loops_now(cx);
                Some(LottieView::new(name).size(self.side).looping(loops_now).into_any_element())
            }
            AchievementArtworkPolicyResolved::RemoteLottie(url) => {
                let loops_now = self.loops_now(cx);
                Some(self.remote_lottie(url, loops_now, window, cx))
            }
            AchievementArtworkPolicyResolved::Image(url) => Some(
                CleverImageUi::new(url)
                    .id(self.child_id("image"))
                    .content_mode(ContentMode::Fit)
                    .corner_radius(0.0)
                    .persist_to_disk(true)
                    .shows_progress_indicator(false)
                    .maximum_decode_pixel_dimension((self.side * 3.0) as i64)
                    .into_any_element(),
            ),
            AchievementArtworkPolicyResolved::Fallback => self.fallback(cx),
        };

        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(self.side))
            .children(content)
    }
}
