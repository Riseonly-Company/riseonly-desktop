use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::layer_keys::LayerKeysExt;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::text_area_ui::TextAreaUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::components::stories_grid::stories_grid_cell::{StoriesGridCell, StoriesGridMetrics};
use crate::modules::story::shared::story_media_reedit::{StoryMediaReedit, StoryMediaReeditResult};
use crate::modules::story::shared::story_publish_staging::StoryPublishStaging;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionStoryHandler, StoryActionTextHandler,
};
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_types::{StoryAudience, StoryAudienceExt};
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

const CAPTION_FIELD: &str = "caption";
const CAPTION_RULE_FIELD: &str = "story_caption";
const NOW_TICK: Duration = Duration::from_secs(5);
const VIDEO_MEDIA_TYPE: &str = "video";

static WINDOW_CLOSED_TEXT: LazyLock<String> = LazyLock::new(|| {
    CanonErrors::text(Some("STORY_EDIT_WINDOW_CLOSED"), &HashMap::new())
        .unwrap_or_else(|| tr("error_story_edit_window_closed"))
});

const SELECTABLE_AUDIENCES: [StoryAudience; 3] =
    [StoryAudience::Everyone, StoryAudience::Contacts, StoryAudience::CloseFriends];

struct StoryEditPageState {
    caption: String,
    allow_screenshots: bool,
    post_to_profile: bool,
    audience: StoryAudience,
    original_audience: StoryAudience,
    did_load_fields: bool,
    is_saving: bool,
    is_reediting: bool,
    pending_media: Option<StoryMediaReeditResult>,
    now: Date,
    layered_document: Option<(i64, bool)>,
    page_width: Option<f32>,
    _observations: [Subscription; 2],
}

impl StoryEditPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let service = StoryServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            caption: String::new(),
            allow_screenshots: true,
            post_to_profile: true,
            audience: StoryAudience::Everyone,
            original_audience: StoryAudience::Everyone,
            did_load_fields: false,
            is_saving: false,
            is_reediting: false,
            pending_media: None,
            now: Date::now(),
            layered_document: None,
            page_width: None,
            _observations: [
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn audience_is_list_based(&self) -> bool {
        self.original_audience == StoryAudience::Selected
    }
}

#[derive(Clone)]
struct StoryEditPageParts {
    story_id: i64,
    state: Entity<StoryEditPageState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl StoryEditPageParts {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    fn story(&self, cx: &mut App) -> Option<StoryDTO> {
        let owner_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()?;
        let service = StoryServicesStore::shared(cx);
        for scope in [StoryProfileScope::Profile, StoryProfileScope::Archive] {
            let hit = service
                .update(cx, |service, cx| service.profile_stories(&owner_id, scope, cx))
                .into_iter()
                .find(|story| story.id == self.story_id);
            if hit.is_some() {
                return hit;
            }
        }
        service
            .update(cx, |service, cx| service.user_stories(&owner_id, cx))
            .into_iter()
            .find(|story| story.id == self.story_id)
    }

    fn appearance(cx: &mut App) -> Appearance {
        match ThemeStore::shared(cx).read(cx).resolved_appearance() {
            RiseThemeAppearance::Light => Appearance::Light,
            RiseThemeAppearance::Dark => Appearance::Dark,
        }
    }

    fn warn_window_closed(cx: &mut App) {
        Notifier::send(
            WINDOW_CLOSED_TEXT.clone(),
            NotifierSendOptions { r#type: NotifierType::Warning, ..NotifierSendOptions::default() },
            cx,
        );
    }

    fn present_error(message: String, cx: &mut App) {
        NativeConfirmAlert::present_notice(&tr("story_edit_error"), &message, None, None, |_| {}, cx);
    }

    fn preview(&self, story: Option<&StoryDTO>, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let page_width = self.state.read(cx).page_width;
        let side = page_width.map_or(220.0, |width| (width - 120.0).clamp(0.0, 220.0));
        let height = side / StoriesGridMetrics::CELL_ASPECT;
        div().w(px(side)).h(px(height)).rounded(px(14.0)).overflow_hidden().map(|preview| match story {
            Some(story) => preview.child(StoriesGridCell::new(story.clone(), side, height, true, None, false)),
            None => preview.bg(theme.bg._300),
        })
    }

    fn has_layered_document(&self, story: &StoryDTO, cx: &mut App) -> bool {
        let known = self.state.read(cx).layered_document.filter(|(story_id, _)| *story_id == story.id);
        if let Some((_, layered)) = known {
            return layered;
        }
        let layered = StoryMediaReedit::has_layered_document(story, cx);
        let story_id = story.id;
        self.state.update(cx, |state, _| state.layered_document = Some((story_id, layered)));
        layered
    }

    fn media_section(&self, story: Option<&StoryDTO>, theme: &AppTheme, cx: &mut App) -> Option<impl IntoElement> {
        let state = self.state.read(cx);
        let now = state.now;
        let shows = state.pending_media.is_some()
            || story.is_some_and(|story| StoryMediaReedit::is_in_edit_window_at(story, now));
        shows.then(|| self.media_controls(story, theme, cx))
    }

    fn media_controls(&self, story: Option<&StoryDTO>, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let layered = story.is_some_and(|story| self.has_layered_document(story, cx));
        let state = self.state.read(cx);
        let parts = self.clone();
        vstack(8.0)
            .w_full()
            .child(
                LiquidGlassButton::new("story-edit-media", LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(20.0)
                    .vertical_padding(11.0)
                    .disabled(state.is_reediting || story.is_none())
                    .on_press(move |_, cx| parts.reedit_media(cx))
                    .child(
                        hstack(8.0)
                            .children(IconUi::sized("wand.and.stars", px(14.0), theme.text.primary, false))
                            .child(
                                MainText::new(tr("story_edit_media"))
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary),
                            ),
                    ),
            )
            .child(
                div().w_full().px(px(28.0)).child(
                    MainText::new(tr(if layered { "story_edit_media_layers_hint" } else { "story_edit_media_flat_hint" }))
                        .px(11.5)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(3),
                ),
            )
            .when(state.pending_media.is_some(), |section| {
                section.child(
                    MainText::new(tr("story_edit_media_ready"))
                        .px(12.0)
                        .tac(TextAlignT::Center)
                        .color(theme.primary._100),
                )
            })
    }

    fn separator(theme: &AppTheme) -> impl IntoElement {
        div().w_full().pl(px(56.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.4)))
    }

    fn tinted_icon(tint: Hsla, icon: &str, theme: &AppTheme) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(32.0))
            .rounded_full()
            .bg(tint)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(icon, px(14.0), theme.content_on(tint), false))
    }

    fn audience_section(&self, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let (list_based, audience, original_audience) = {
            let state = self.state.read(cx);
            (state.audience_is_list_based(), state.audience, state.original_audience)
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        if list_based {
            rows.push(self.audience_row(original_audience, theme, false, cx).into_any_element());
            rows.push(Self::separator(theme).into_any_element());
        }
        for (index, option) in SELECTABLE_AUDIENCES.iter().enumerate() {
            if index > 0 || list_based {
                rows.push(Self::separator(theme).into_any_element());
            }
            rows.push(self.audience_row(*option, theme, true, cx).into_any_element());
        }
        div().w_full().px(px(16.0)).child(
            vstack(8.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .child(
                    MainText::new(tr("story_audience_title"))
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .color(theme.text.secondary)
                        .m_l(4.0),
                )
                .child(div().w_full().bg(theme.bg._200).rounded(px(14.0)).overflow_hidden().children(rows))
                .when(list_based && audience != original_audience, |section| {
                    section.child(
                        MainText::new(tr("story_audience_list_dropped_hint"))
                            .px(11.5)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .m_l(4.0)
                            .number_of_lines(3),
                    )
                }),
        )
    }

    fn audience_row(&self, option: StoryAudience, theme: &AppTheme, selectable: bool, cx: &mut App) -> impl IntoElement {
        let is_selected = self.state.read(cx).audience == option;
        let tint = Self::audience_color(option, cx);
        let state = self.state.clone();
        plain(SharedString::from(format!("story-edit-audience-{}", option.wire())))
            .disabled(!selectable)
            .on_press(move |_, cx| {
                if !selectable {
                    return;
                }
                let _ = SelectionFeedbackGenerator::new().selection_changed();
                state.update(cx, |state, cx| {
                    state.audience = option;
                    cx.notify();
                });
            })
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(12.0))
            .py(px(11.0))
            .opacity(if selectable { 1.0 } else { 0.55 })
            .child(Self::tinted_icon(tint, option.icon(), theme))
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(
                        MainText::new(tr(option.i18n_key()))
                            .px(15.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        MainText::new(tr(option.hint_i18n_key()))
                            .px(11.5)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .number_of_lines(2),
                    ),
            )
            .children(is_selected.then(|| IconUi::sized("checkmark", px(14.0), theme.primary._100, false)).flatten())
    }

    fn audience_color(option: StoryAudience, cx: &mut App) -> Hsla {
        let appearance = Self::appearance(cx);
        match option {
            StoryAudience::Everyone => UIColor::system_blue(),
            StoryAudience::Contacts => UIColor::system_green(),
            StoryAudience::CloseFriends => UIColor::system_orange(),
            StoryAudience::Selected => UIColor::system_purple(),
        }
        .resolved_color(appearance)
    }

    /// caption ceiling from canon validation,
    /// same rule gateway and service check,
    /// for the authors tier
    fn caption_rule(cx: &mut App) -> RiseFieldRule {
        let premium =
            AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        RiseFieldRule::client_rule(CAPTION_RULE_FIELD).for_tier(premium)
    }

    fn caption_section(&self, cx: &mut App) -> impl IntoElement {
        let caption = self.state.read(cx).caption.clone();
        let state = self.state.clone();
        div().w_full().px(px(16.0)).child(
            TextAreaUi::new("story-edit-caption")
                .values(HashMap::from([(CAPTION_FIELD.to_owned(), caption)]))
                .name(CAPTION_FIELD)
                .placeholder(tr("story_caption_placeholder"))
                .min_lines(2)
                .max_lines(6)
                .rule(Self::caption_rule(cx))
                .set_value(move |_, value, _, cx| {
                    let value = value.to_owned();
                    state.update(cx, |state, cx| {
                        state.caption = value;
                        cx.notify();
                    });
                }),
        )
    }

    fn toggles_section(&self, theme: &AppTheme, cx: &mut App) -> impl IntoElement {
        let appearance = Self::appearance(cx);
        let allow_screenshots = Binding::entity(
            self.state.clone(),
            |state: &StoryEditPageState| state.allow_screenshots,
            |state, value, _| state.allow_screenshots = value,
        );
        let post_to_profile = Binding::entity(
            self.state.clone(),
            |state: &StoryEditPageState| state.post_to_profile,
            |state, value, _| state.post_to_profile = value,
        );
        div().w_full().px(px(16.0)).child(
            div()
                .w_full()
                .bg(theme.bg._200)
                .rounded(px(14.0))
                .overflow_hidden()
                .child(Self::toggle_row(
                    "story-edit-allow-screenshots",
                    "camera.viewfinder",
                    UIColor::system_indigo().resolved_color(appearance),
                    tr("story_allow_screenshots"),
                    tr("story_allow_screenshots_hint"),
                    allow_screenshots,
                    theme,
                ))
                .child(Self::separator(theme))
                .child(Self::toggle_row(
                    "story-edit-post-to-profile",
                    "person.crop.square.fill",
                    UIColor::system_teal().resolved_color(appearance),
                    tr("story_post_to_profile"),
                    tr("story_post_to_profile_hint"),
                    post_to_profile,
                    theme,
                )),
        )
    }

    fn toggle_row(
        id: &'static str,
        icon: &str,
        tint: Hsla,
        title: String,
        hint: String,
        value: Binding<bool>,
        theme: &AppTheme,
    ) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .px(px(12.0))
            .py(px(11.0))
            .child(Self::tinted_icon(tint, icon, theme))
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(title).px(15.0).tac(TextAlignT::Left).font_weight(FontWeight::MEDIUM))
                    .child(
                        MainText::new(hint)
                            .px(11.5)
                            .tac(TextAlignT::Left)
                            .color(theme.text.secondary)
                            .number_of_lines(2),
                    ),
            )
            .child(SwitchUi::new(id, value))
    }

    fn save_button(&self, has_story: bool, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let parts = self.clone();
        LiquidGlassButton::new("story-edit-save", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(28.0)
            .vertical_padding(12.0)
            .disabled(self.state.read(cx).is_saving || !has_story)
            .on_press(move |_, cx| parts.save(cx))
            .child(
                MainText::new(tr("story_edit_save"))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
    }

    /// seeded once from whichever cache holds the story,
    /// reseeding on every revision would discard what the user types
    fn load_fields_if_needed(&self, cx: &mut App) {
        if self.state.read(cx).did_load_fields {
            return;
        }
        let Some(story) = self.story(cx) else {
            return;
        };
        self.state.update(cx, |state, cx| {
            state.did_load_fields = true;
            state.caption = story.caption.clone().unwrap_or_default();
            state.allow_screenshots = story.allow_screenshots;
            state.post_to_profile = story.post_to_profile;
            let resolved = StoryAudience::from_wire(&story.audience).unwrap_or(StoryAudience::Everyone);
            state.original_audience = resolved;
            state.audience = resolved;
            cx.notify();
        });
    }

    fn reedit_media(&self, cx: &mut App) {
        if self.state.read(cx).is_reediting {
            return;
        }
        let Some(story) = self.story(cx) else {
            return;
        };
        if !StoryMediaReedit::is_in_edit_window(&story) {
            Self::warn_window_closed(cx);
            self.pop(cx);
            return;
        }
        self.state.update(cx, |state, cx| {
            state.is_reediting = true;
            cx.notify();
        });
        let theme_store = ThemeStore::shared(cx);
        let state = self.state.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let outcome = StoryMediaReedit::run(story, theme_store, None, cx).await;
            cx.update(|cx| {
                let failed = outcome.is_err();
                state.update(cx, |state, cx| {
                    state.is_reediting = false;
                    if let Ok(Some(result)) = outcome {
                        StoryPublishStaging::discard(
                            state.pending_media.as_ref().and_then(|media| media.local_file_url.as_deref()),
                        );
                        state.pending_media = Some(result);
                    }
                    cx.notify();
                });
                if failed {
                    Self::present_error(tr("story_edit_error"), cx);
                }
            });
        })
        .detach();
    }

    fn save(&self, cx: &mut App) {
        if self.state.read(cx).is_saving {
            return;
        }
        let Some(story) = self.story(cx) else {
            return;
        };
        let state = self.state.read(cx);
        let audience_changed = state.audience != state.original_audience;
        let caption = state.caption.clone();
        let audience = state.audience;
        let allow_screenshots = state.allow_screenshots;
        let post_to_profile = state.post_to_profile;
        let media_file_id = state.pending_media.as_ref().map(|media| media.file_id.clone());
        let media_type = state.pending_media.as_ref().map(|media| media.media_type.clone());
        let canvas = state.pending_media.as_ref().and_then(|media| media.canvas.clone());
        let link_areas = state.pending_media.as_ref().and_then(|media| media.link_areas.clone());
        let track_plate = state.pending_media.as_ref().and_then(|media| media.track_plate.clone());
        self.state.update(cx, |state, cx| {
            state.is_saving = true;
            cx.notify();
        });
        let parts = self.clone();
        let on_success: StoryActionStoryHandler = Rc::new(move |saved: StoryDTO, cx: &mut App| {
            let adopted = parts.state.update(cx, |state, cx| {
                state.is_saving = false;
                cx.notify();
                state.pending_media.as_mut().and_then(|media| {
                    let local = media.local_file_url.take()?;
                    Some((local, media.media_type == VIDEO_MEDIA_TYPE, media.local_aspect))
                })
            });
            if let Some((local, is_video, aspect)) = adopted {
                StoryServicesStore::shared(cx)
                    .update(cx, |service, cx| service.adopt_reedited_media(local, is_video, aspect, &saved, cx));
            }
            parts.pop(cx);
        });
        let error_parts = self.clone();
        let on_error: StoryActionTextHandler = Rc::new(move |message: String, cx: &mut App| {
            error_parts.state.update(cx, |state, cx| {
                state.is_saving = false;
                cx.notify();
            });
            if message == *WINDOW_CLOSED_TEXT {
                Self::warn_window_closed(cx);
                error_parts.pop(cx);
                return;
            }
            Self::present_error(message, cx);
        });
        StoryInteractionsStore::actions(cx).update(cx, |actions, cx| {
            actions.update_story_action(
                story.id,
                Some(caption),
                audience_changed.then_some(audience),
                Some(allow_screenshots),
                Some(post_to_profile),
                audience_changed.then(Vec::new),
                audience_changed.then(Vec::new),
                media_file_id,
                media_type,
                canvas,
                link_areas,
                track_plate,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
struct StoryEditPageBody {
    parts: StoryEditPageParts,
}

impl RenderOnce for StoryEditPageBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parts = self.parts;
        let theme = rise_ui::theme(cx).clone();
        let story = parts.story(cx);
        let story = story.as_ref();
        let width_state = parts.state.clone();
        let can_save = !parts.state.read(cx).is_saving && story.is_some();
        let submit = parts.clone();
        scroll_view("story-edit-scroll")
            .list_shell_scroll_under_chrome(cx)
            .size_full()
            .bg(theme.bg._100)
            .child(
                vstack(20.0)
                    .w_full()
                    .pt(px(12.0))
                    .form_primary(can_save, move |_, cx| submit.save(cx))
                    .child(parts.preview(story, &theme, cx))
                    .children(parts.media_section(story, &theme, cx))
                    .child(parts.caption_section(cx))
                    .child(parts.audience_section(&theme, cx))
                    .child(parts.toggles_section(&theme, cx))
                    .child(parts.save_button(story.is_some(), &theme, cx))
                    .child(div().w_full().h(px(60.0)))
                    .child(measure("story-edit-page-width", move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        width_state.update(cx, |state, cx| {
                            state.page_width = Some(width);
                            cx.notify();
                        });
                    })),
            )
    }
}

/// edit page of a published story, row keeps id and counters
/// only a media re render stamps edited_at_ms, a caption change doesnt
/// audience sent only if changed
#[derive(IntoElement)]
pub struct StoryEditPage {
    pub story_id: i64,
}

impl StoryEditPage {
    pub fn new(story_id: i64) -> Self {
        Self { story_id }
    }
}

impl RenderOnce for StoryEditPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            SharedString::from(format!("story-edit-state-{}", self.story_id)),
            cx,
            |_, cx| StoryEditPageState::new(cx),
        );
        let parts =
            StoryEditPageParts { story_id: self.story_id, state, nav_store: env::<NavigationStoreKey>(cx) };

        let task_parts = parts.clone();
        use_task("story-edit-load", self.story_id, window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_parts.load_fields_if_needed(cx));
        });
        let tick_state = parts.state.clone();
        use_task("story-edit-now", self.story_id, window, cx, async move |cx| loop {
            cx.background_executor().timer(NOW_TICK).await;
            let ticked = cx.update(|_, cx| {
                tick_state.update(cx, |state, cx| {
                    state.now = Date::now();
                    cx.notify();
                });
            });
            if ticked.is_err() {
                return;
            }
        });
        let resolved_id = parts.story(cx).map(|story| story.id);
        let change_parts = parts.clone();
        on_change("story-edit-story-id", resolved_id, false, window, cx, move |_, _, _, cx| {
            change_parts.load_fields_if_needed(cx);
        });

        let back_parts = parts.clone();
        SettingsWrapperUi::new(tr("story_edit_title"), move |_, cx| back_parts.pop(cx))
            .content(StoryEditPageBody { parts })
    }
}
