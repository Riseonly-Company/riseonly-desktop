use std::ops::Range;

use gpui::{App, AsyncApp, Bounds, Context, ElementId, Entity, FontWeight, Hsla, Pixels, SharedString, Subscription, Window};
use rise_catalog::AchievementRarity;
use rise_core::foundation::foundation_compare::{CompareOptions, range_of};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{tr, tr_plural};
use rise_theme::color;
use rise_ui::animated_media_playback_scope::AnimatedMediaViewportPlaybackPolicy;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::{AttributedString, RichFont};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::modules::user::components::achievements::achievement_artwork::{
    AchievementArtwork, AchievementArtworkPolicy, AchievementArtworkPolicyResolved,
};
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsPresentation;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;
use crate::modules::user::stores::user_achievements::user_achievements_services::user_achievements_services::UserAchievementsServicesStore;

pub enum PostAchievementMediaMetrics {}

impl PostAchievementMediaMetrics {
    pub const PLATE_HEIGHT: f32 = 268.0;
    pub const ARTWORK_SIDE: f32 = 184.0;
    pub const COMPOSER_PLATE_HEIGHT: f32 = 208.0;
    pub const COMPOSER_ARTWORK_SIDE: f32 = 148.0;
}

/// resolves a posts achievement_id against what the achievements domain holds
/// no fetch per row, an unresolved id asks the domain once
/// cache first and deduped per user
pub enum PostAchievementCatalog {}

impl PostAchievementCatalog {
    pub fn achievement(id: i64, cx: &mut App) -> Option<RiseAchievement> {
        if id <= 0 {
            return None;
        }

        let snapshot = UserAchievementsServicesStore::snapshot(cx);
        if let Some(own) = snapshot.achievements.iter().find(|achievement| achievement.id == id) {
            return Some(own.clone());
        }
        for showcase in &snapshot.showcases {
            if let Some(found) = showcase.achievements.iter().find(|achievement| achievement.id == id) {
                return Some(found.clone());
            }
        }
        None
    }
}

pub enum PostAchievementCopy {}

impl PostAchievementCopy {
    pub fn suggested_title(achievement: &RiseAchievement) -> String {
        Self::substitute(&tr("achievement_share_suggested_title"), &[("title", achievement.title.as_str())])
    }

    pub fn suggested_description(achievement: &RiseAchievement) -> String {
        let rarity = format!("{} {}.", tr("achievement_rarity_label"), Self::rarity_name(achievement.rarity_tier()));
        if achievement.granted_count <= 0 {
            return rarity;
        }

        let holders = tr_plural("achievements_holders_count", achievement.granted_count);
        format!("{holders}. {rarity}")
    }

    pub fn rarity_name(tier: AchievementRarity) -> String {
        tr(tier.i18n_key())
    }

    /// flat tiers carry a color and no gradient, gradient tiers the inverse
    /// a text run holds one color so a gradient tier uses its first stop
    pub fn rarity_color(tier: AchievementRarity) -> Option<Hsla> {
        if let Some(flat) = tier.color() {
            return Some(color(flat));
        }
        tier.gradient().first().map(|stop| color(stop))
    }

    pub fn highlighting_rarity(text: &str, tier: AchievementRarity) -> AttributedString {
        let mut attributed = AttributedString::new(text);
        let word = Self::rarity_name(tier);
        if word.is_empty() {
            return attributed;
        }
        let Some(tint) = Self::rarity_color(tier) else {
            return attributed;
        };
        let Some(range): Option<Range<usize>> = range_of(text, &word, CompareOptions::CASE_INSENSITIVE) else {
            return attributed;
        };
        attributed.update_byte_range(range, |attributes| {
            attributes.foreground_color = Some(tint);
            attributes.font = Some(RichFont::system_weight(0.0, FontWeight::BOLD));
        });
        attributed
    }

    fn substitute(template: &str, values: &[(&str, &str)]) -> String {
        let mut result = template.to_owned();
        for (name, value) in values {
            result = result.replace(&format!("{{{name}}}"), value);
        }
        result
    }
}

/// achievement animation gated to the viewport
/// remote lottie goes through the sticker view which already owns the viewport gate, playback
/// slot and residency release
#[derive(IntoElement)]
pub struct PostAchievementArtworkView {
    id: SharedString,
    achievement: RiseAchievement,
    side: f32,
    gates_playback_on_viewport: bool,
}

impl PostAchievementArtworkView {
    pub fn new(id: impl Into<SharedString>, achievement: RiseAchievement, side: f32) -> Self {
        Self { id: id.into(), achievement, side, gates_playback_on_viewport: true }
    }

    pub fn gates_playback_on_viewport(mut self, gates_playback_on_viewport: bool) -> Self {
        self.gates_playback_on_viewport = gates_playback_on_viewport;
        self
    }

    fn needs_viewport_gate(&self) -> bool {
        self.gates_playback_on_viewport
            && matches!(AchievementArtworkPolicy::resolve(&self.achievement), AchievementArtworkPolicyResolved::Builtin(_))
    }
}

impl RenderOnce for PostAchievementArtworkView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let needs_viewport_gate = self.needs_viewport_gate();
        let probe = window.use_keyed_state(ElementId::Name(format!("{}-probe", self.id).into()), cx, |_, _| {
            PostAchievementViewportProbe::new()
        });
        let is_in_viewport = probe.read(cx).is_in_viewport;
        let reader_id = probe.read(cx).reader_id.clone();

        let artwork = if needs_viewport_gate && !is_in_viewport {
            div().size_full().into_any_element()
        } else {
            AchievementArtwork::new(self.achievement, self.side).into_any_element()
        };

        div()
            .relative()
            .flex_none()
            .size(px(self.side))
            .when(needs_viewport_gate, |frame| {
                frame.child(
                    WindowScreenFrameReader::new(reader_id)
                        .on_change(move |_: Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                            PostAchievementViewportProbe::schedule_delivery(probe.clone(), cx);
                        }),
                )
            })
            .child(artwork)
    }
}

#[derive(IntoElement)]
pub struct PostAchievementMedia {
    id: SharedString,
    achievement_id: i64,
    author_id: Option<String>,
    plate_height: f32,
    artwork_side: f32,
    gates_playback_on_viewport: bool,
}

impl PostAchievementMedia {
    pub fn new(id: impl Into<SharedString>, achievement_id: i64) -> Self {
        Self {
            id: id.into(),
            achievement_id,
            author_id: None,
            plate_height: PostAchievementMediaMetrics::PLATE_HEIGHT,
            artwork_side: PostAchievementMediaMetrics::ARTWORK_SIDE,
            gates_playback_on_viewport: true,
        }
    }

    pub fn author_id(mut self, author_id: Option<String>) -> Self {
        self.author_id = author_id;
        self
    }

    pub fn plate_height(mut self, plate_height: f32) -> Self {
        self.plate_height = plate_height;
        self
    }

    pub fn artwork_side(mut self, artwork_side: f32) -> Self {
        self.artwork_side = artwork_side;
        self
    }

    pub fn gates_playback_on_viewport(mut self, gates_playback_on_viewport: bool) -> Self {
        self.gates_playback_on_viewport = gates_playback_on_viewport;
        self
    }

    /// fallback for an achievement not loaded yet
    /// a still trophy on the same plate, no spinner that could hang
    /// asks for the viewer and author lists once and artwork replaces it when either holds the id
    fn unresolved(&self, cx: &App) -> Div {
        let theme = rise_ui::theme(cx);
        div().flex().flex_none().items_center().justify_center().size(px(self.artwork_side)).children(IconUi::sized(
            "trophy.fill",
            px(self.artwork_side * 0.34),
            theme.text.secondary.opacity(0.55),
            true,
        ))
    }
}

/// observable on the achievements snapshot this views body reads
struct PostAchievementMediaObserver {
    _presentation: Subscription,
}

impl PostAchievementMediaObserver {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseUserAchievementsPresentation::shared(cx);
        Self { _presentation: cx.observe(&presentation, |_, _, cx| cx.notify()) }
    }
}

impl RenderOnce for PostAchievementMedia {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state(
            ElementId::Name(format!("{}-observer", self.id).into()),
            cx,
            |_, cx: &mut Context<PostAchievementMediaObserver>| PostAchievementMediaObserver::new(cx),
        );

        let content = match PostAchievementCatalog::achievement(self.achievement_id, cx) {
            Some(achievement) => PostAchievementArtworkView::new(
                format!("{}-artwork", self.id),
                achievement,
                self.artwork_side,
            )
            .gates_playback_on_viewport(self.gates_playback_on_viewport)
            .into_any_element(),
            None => {
                let author_id = self.author_id.clone();
                use_task(
                    SharedString::from(format!("{}-resolve", self.id)),
                    self.achievement_id,
                    window,
                    cx,
                    async move |cx| {
                        let cx: &mut AsyncApp = cx;
                        cx.update(|cx| {
                            UserAchievementsInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                interactions.resolve_shared_achievement(author_id.as_deref(), cx);
                            });
                        });
                    },
                );
                self.unresolved(cx).into_any_element()
            }
        };

        div()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(self.plate_height))
            .overflow_hidden()
            .child(content)
    }
}

struct PostAchievementViewportProbe {
    reader_id: String,
    is_in_viewport: bool,
    last_reported: Option<bool>,
    is_delivery_scheduled: bool,
}

impl PostAchievementViewportProbe {
    fn new() -> Self {
        Self {
            reader_id: format!("post-achievement-probe-{}", Uuid::new_v4().uuid_string()),
            is_in_viewport: false,
            last_reported: None,
            is_delivery_scheduled: false,
        }
    }

    fn schedule_delivery(probe: Entity<Self>, cx: &mut App) {
        let was_scheduled = probe.update(cx, |probe, _| std::mem::replace(&mut probe.is_delivery_scheduled, true));
        if was_scheduled {
            return;
        }
        cx.defer(move |cx| {
            probe.update(cx, |probe, cx| {
                probe.is_delivery_scheduled = false;
                let visible = MediaCellScreenFrameRegistry::attached_view(&probe.reader_id).is_some_and(|view| {
                    view.is_visible_in_active_clipped_viewport_with(
                        AnimatedMediaViewportPlaybackPolicy::MINIMUM_VISIBLE_HEIGHT_FRACTION,
                        true,
                    )
                });
                if probe.last_reported == Some(visible) {
                    return;
                }
                probe.last_reported = Some(visible);
                if probe.is_in_viewport != visible {
                    probe.is_in_viewport = visible;
                    cx.notify();
                }
            });
        });
    }
}
