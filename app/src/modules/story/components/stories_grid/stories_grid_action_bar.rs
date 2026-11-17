use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div, px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::stacks::hstack;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Confirmation {
    Archive,
    Unarchive,
    Delete,
}

/// action bar of selection mode,
/// mount it on the screen root not the grid or the clipped pager cuts it off
#[derive(IntoElement)]
pub struct StoriesGridActionBar {
    pub owner_user_id: String,
    pub scope: StoryProfileScope,
}

impl StoriesGridActionBar {
    const TRANSITION_DURATION: f64 = 0.2;

    pub fn new(owner_user_id: impl Into<String>, scope: StoryProfileScope) -> Self {
        Self { owner_user_id: owner_user_id.into(), scope }
    }

    fn bar(&self, cx: &mut App) -> AnyElement {
        let is_archive = self.scope == StoryProfileScope::Archive;
        let error_tint = ThemeStore::shared(cx).read(cx).current_theme().semantic.error_100;
        let confirm_owner = self.owner_user_id.clone();
        let delete_owner = self.owner_user_id.clone();
        let primary = Self::pill(
            "stories-grid-bar-archive",
            if is_archive { tr("story_action_unarchive") } else { tr("story_action_archive") },
            if is_archive { "arrow.uturn.backward" } else { "archivebox" },
            None,
            move |cx| {
                let confirmation = if is_archive { Confirmation::Unarchive } else { Confirmation::Archive };
                Self::alert(confirmation, &confirm_owner, cx);
            },
            cx,
        );
        let delete = Self::pill(
            "stories-grid-bar-delete",
            tr("ctx_delete"),
            "trash",
            Some(error_tint),
            move |cx| Self::alert(Confirmation::Delete, &delete_owner, cx),
            cx,
        );
        let archive_anchor = (self.scope == StoryProfileScope::Profile).then_some(RiseGuideAnchor::StoriesBarArchive);
        hstack(10.0)
            .w_full()
            .justify_center()
            .px(px(16.0))
            .child(primary.guide_anchor(archive_anchor))
            .child(delete)
            .into_any_element()
    }

    fn pill(
        id: &'static str,
        title: String,
        system_image: &'static str,
        tint: Option<Hsla>,
        action: impl Fn(&mut App) + 'static,
        cx: &mut App,
    ) -> LiquidGlassButton {
        let enabled = !StoryGridSelection::shared(cx).read(cx).is_empty();
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let id = ElementId::Name(SharedString::new_static(id));
        LiquidGlassButton::new(id, LiquidGlassButtonShape::Capsule)
            .horizontal_padding(18.0)
            .vertical_padding(12.0)
            .tint(tint)
            .haptic_on_press(enabled)
            .disabled(!enabled)
            .accessibility_label(Some(title.clone()))
            .on_press(move |_, cx| {
                if enabled {
                    action(cx);
                }
            })
            .child(
                hstack(6.0)
                    .opacity(if enabled { 1.0 } else { 0.45 })
                    .children(IconUi::sized(system_image, px(14.0), text_primary, false))
                    .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(text_primary)),
            )
    }

    /// the selection as of the press, as the alert reads it when its built
    pub(super) fn alert(confirmation: Confirmation, owner_user_id: &str, cx: &mut App) {
        let selection = StoryGridSelection::shared(cx);
        let ids: Vec<i64> = selection.read(cx).selected_ids().to_vec();
        let many = ids.len() > 1;
        let owner = owner_user_id.to_owned();
        let finish = {
            let selection = selection.clone();
            move |cx: &mut App| selection.update(cx, |selection, cx| selection.end(cx))
        };
        match confirmation {
            Confirmation::Archive => NativeConfirmAlert::present_delete(
                &tr(if many { "story_archive_many_confirm_title" } else { "story_archive_confirm_title" }),
                &tr(if many { "story_archive_many_confirm_message" } else { "story_archive_confirm_message" }),
                Some(tr("story_action_archive")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.archive_stories(&ids, true, Some(&owner), None, cx);
                    });
                    finish(cx);
                },
                cx,
            ),
            Confirmation::Unarchive => NativeConfirmAlert::present_delete(
                &tr("story_unarchive_confirm_title"),
                &tr(if many { "story_unarchive_many_confirm_message" } else { "story_unarchive_confirm_message" }),
                Some(tr("story_action_unarchive")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.archive_stories(&ids, false, Some(&owner), None, cx);
                    });
                    finish(cx);
                },
                cx,
            ),
            Confirmation::Delete => NativeConfirmAlert::present_delete(
                &tr(if many { "story_delete_many_confirm_title" } else { "story_delete_confirm_title" }),
                &tr("story_delete_confirm_message"),
                Some(tr("ctx_delete")),
                None,
                false,
                None,
                None,
                move |cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.delete_stories(&ids, Some(&owner), None, cx);
                    });
                    finish(cx);
                },
                cx,
            ),
        }
    }
}

impl RenderOnce for StoriesGridActionBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_active = StoryGridSelection::shared(cx).read(cx).is_active_for(&self.owner_user_id, self.scope);
        let id = ElementId::NamedChild(
            Arc::new(ElementId::Name(SharedString::from(format!(
                "stories-grid-action-bar-{}-{}",
                self.owner_user_id,
                self.scope.raw_value()
            )))),
            SharedString::new_static("presence"),
        );
        let shown = is_active.then_some(());
        let presence = window.use_keyed_state(id, cx, |_, _| {
            Presence::new(shown).with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity()))
        });
        let now = RiseClock::monotonic();
        presence.update(cx, |presence, cx| presence.set_in(shown, ease_out(Self::TRANSITION_DURATION), now, cx));
        if !is_active && !presence.read(cx).is_running(now) {
            return div().into_any_element();
        }
        let bar = self.bar(cx);
        div()
            .w_full()
            .children(presence.read(cx).render_transitioned(now, window, move |_| div().w_full().child(bar)))
            .into_any_element()
    }
}

/// replaces the top right header item while a grid is selecting
#[derive(IntoElement)]
pub struct StoriesGridDoneButton;

impl RenderOnce for StoriesGridDoneButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selection = StoryGridSelection::shared(cx);
        let text_primary = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let title = tr("common_done");
        LiquidGlassButton::new(
            ElementId::Name(SharedString::new_static("stories-grid-done")),
            LiquidGlassButtonShape::Capsule,
        )
        .horizontal_padding(14.0)
        .vertical_padding(10.0)
        .accessibility_label(Some(title.clone()))
        .on_press(move |_, cx| selection.update(cx, |selection, cx| selection.end(cx)))
        .child(MainText::new(title).px(14.0).font_weight(FontWeight::SEMIBOLD).color(text_primary))
    }
}
