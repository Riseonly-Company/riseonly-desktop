use std::rc::Rc;

use gpui::{
    App, ElementId, Entity, IntoElement, Pixels, Point, RenderOnce, ScrollHandle, SharedString, Window, div,
    prelude::*, px,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification, selection_changed};
use rise_theme::AppTheme;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;

use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementPart;
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftModifiers, SharedElementSurfaceKey,
};
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::pager_back_swipe_yield::PagerBackSwipeYield;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::components::story_thumbnail_ui::StoryThumbnailUi;
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, HighlightScope, StoryDTOExt};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::profile_content::ProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::ProfileScrollMetrics;
use crate::modules::user::components::profile_top::profile_highlight_sheets_host::{
    ProfileHighlightSheetsMount, ProfileHighlightSheetsPresenter,
};
use crate::modules::user::components::profile_top::profile_highlights_shared_element_origin::ProfileHighlightsSharedElementOrigin;

pub enum ProfileHighlightsLayoutPolicy {}

impl ProfileHighlightsLayoutPolicy {
    pub const CIRCLE_SIZE: f32 = 64.0;
    pub const LABEL_WIDTH: f32 = Self::CIRCLE_SIZE + 20.0;
    pub const COLUMN_SPACING: f32 = 8.0;
    pub const ITEM_SPACING: f32 = 7.0;
    pub const LABEL_HEIGHT: f32 = 17.0;
    pub const VERTICAL_PADDING: f32 = 4.0;
    pub const ROW_HEIGHT: f32 = Self::CIRCLE_SIZE + Self::ITEM_SPACING + Self::LABEL_HEIGHT + Self::VERTICAL_PADDING;

    /// scroll opacity with the profiles own pinned tabs and outer spacing
    pub fn scroll_opacity_at_profile_metrics(frame_max_y: f32) -> f32 {
        Self::scroll_opacity(
            frame_max_y,
            ProfileScrollMetrics::PINNED_TABS_TOP,
            ProfileViewportLayoutPolicy::OUTER_SPACING,
        )
    }

    /// highlights row fades out while crossing the floating header region, zero just before the tabs pin
    pub fn scroll_opacity(frame_max_y: f32, pinned_tabs_top: f32, outer_spacing: f32) -> f32 {
        let fully_hidden_max_y = pinned_tabs_top - outer_spacing;
        let progress = (frame_max_y - fully_hidden_max_y) / Self::ROW_HEIGHT.max(1.0);
        progress.clamp(0.0, 1.0)
    }
}

struct ProfileHighlightsState {
    show_create: bool,
    show_edit: bool,
    editing_highlight: Option<HighlightDTO>,
    shared_element_origin: Rc<ProfileHighlightsSharedElementOrigin>,
    scroll_handle: ScrollHandle,
}

#[derive(IntoElement)]
pub struct ProfileHighlights {
    id: ElementId,
    is_owner: bool,
    user_id: String,
}

impl ProfileHighlights {
    pub fn new(id: impl Into<ElementId>, user_id: impl Into<String>) -> Self {
        Self { id: id.into(), is_owner: true, user_id: user_id.into() }
    }

    pub fn is_owner(mut self, is_owner: bool) -> Self {
        self.is_owner = is_owner;
        self
    }

    fn manage_actions() -> Vec<HoldContextMenuAction> {
        vec![
            HoldContextMenuAction::new("edit", tr("highlight_menu_edit"))
                .system_image("pencil")
                .guide_anchor(RiseGuideAnchor::HighlightMenuEdit),
            HoldContextMenuAction::new("share", tr("highlight_menu_share_link"))
                .system_image("arrowshape.turn.up.right"),
            HoldContextMenuAction::new("archive", tr("highlight_menu_archive")).system_image("archivebox"),
            HoldContextMenuAction::new("delete", tr("highlight_menu_delete"))
                .system_image("trash")
                .destructive(true)
                .section_break(true),
        ]
    }

    fn handle_manage_action(
        id: &str,
        item: &HighlightDTO,
        state: &Entity<ProfileHighlightsState>,
        sheets: &ProfileHighlightSheetsMount,
        cx: &mut App,
    ) {
        match id {
            "edit" => {
                ProfileHighlightSheetsPresenter::take(sheets.clone(), cx);
                let editing = item.clone();
                state.update(cx, |state, cx| {
                    state.editing_highlight = Some(editing);
                    state.show_edit = true;
                    cx.notify();
                });
            }
            "share" => {
                tracing::warn!(target: "riseonly::X1182", id = item.id, "highlight share chosen");
                let sharing = item.clone();
                HoldContextMenuPresenter::after_dismiss(
                    move |cx: &mut App| {
                        tracing::warn!(target: "riseonly::X1182", id = sharing.id, "highlight share present");
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.share_highlight(&sharing, cx));
                    },
                    cx,
                );
            }
            "archive" => Self::confirm_archive(item.clone(), cx),
            "delete" => Self::confirm_delete(item.clone(), cx),
            _ => {}
        }
    }

    fn confirm_archive(item: HighlightDTO, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("highlight_archive_confirm_title"),
            &tr("highlight_archive_confirm_message"),
            Some(tr("highlight_menu_archive")),
            None,
            false,
            None,
            None,
            move |cx| {
                let _ = selection_changed();
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_highlight_archived(&item, true, Some(item.owner_id.as_str()), cx);
                });
            },
            cx,
        );
    }

    fn confirm_delete(item: HighlightDTO, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("highlight_delete_title"),
            &tr("highlight_delete_message"),
            Some(tr("highlight_menu_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                let _ = notification(NotificationKind::Warning);
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_highlight(item.id, Some(item.owner_id.as_str()), HighlightScope::Profile, cx);
                });
            },
            cx,
        );
    }

    fn open(
        item: &HighlightDTO,
        origin: &Rc<ProfileHighlightsSharedElementOrigin>,
        shared_elements_active: bool,
        cx: &mut App,
    ) {
        let _ = selection_changed();
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.open_highlight(item.id, &item.owner_id, shared_elements_active.then_some(origin), Some(item), cx);
        });
    }

    fn load_highlights(user_id: &str, cx: &mut App) {
        if user_id.is_empty() {
            return;
        }
        let mine = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let target = (mine.as_deref() != Some(user_id)).then_some(user_id);
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_user_highlights(target, HighlightScope::Profile, cx);
        });
    }

    fn sheets_mount(
        id: &ElementId,
        user_id: &str,
        state: &Entity<ProfileHighlightsState>,
    ) -> ProfileHighlightSheetsMount {
        ProfileHighlightSheetsMount {
            row_id: id.clone(),
            user_id: user_id.to_owned(),
            is_creating: Binding::entity(
                state.clone(),
                |state: &ProfileHighlightsState| state.show_create,
                |state: &mut ProfileHighlightsState, value, cx| {
                    state.show_create = value;
                    cx.notify();
                },
            ),
            is_editing: Binding::entity(
                state.clone(),
                |state: &ProfileHighlightsState| state.show_edit,
                |state: &mut ProfileHighlightsState, value, cx| {
                    state.show_edit = value;
                    cx.notify();
                },
            ),
            editing_highlight: Binding::entity(
                state.clone(),
                |state: &ProfileHighlightsState| state.editing_highlight.clone(),
                |state: &mut ProfileHighlightsState, value, cx| {
                    state.editing_highlight = value;
                    cx.notify();
                },
            ),
        }
    }

    fn add_button(
        &self,
        state: &Entity<ProfileHighlightsState>,
        sheets: &ProfileHighlightSheetsMount,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let circle_size = ProfileHighlightsLayoutPolicy::CIRCLE_SIZE;
        let state = state.clone();
        let sheets = sheets.clone();
        plain((self.id.clone(), "add"))
            .on_press(move |_, cx| {
                ProfileHighlightSheetsPresenter::take(sheets.clone(), cx);
                state.update(cx, |state, cx| {
                    state.show_create = true;
                    cx.notify();
                });
            })
            .child(
                vstack(ProfileHighlightsLayoutPolicy::ITEM_SPACING)
                    .child(
                        zstack(Alignment::Center)
                            .relative()
                            .size(px(circle_size))
                            .child(div().absolute().inset_0().rounded_full().bg(theme.bg._200))
                            .child(centred_stroke(theme.border._200.opacity(0.7), 2.0, Shape::Circle))
                            .children(IconUi::sized("plus", px(24.0), theme.text.primary, false)),
                    )
                    .child(
                        MainText::new(tr("highlights_add"))
                            .px(12.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary)
                            .number_of_lines(1)
                            .minimum_scale_factor(0.6)
                            .w(px(ProfileHighlightsLayoutPolicy::LABEL_WIDTH)),
                    ),
            )
            .guide_anchor(RiseGuideAnchor::ProfileHighlightAdd)
    }

    fn highlight_circle_content(item: &HighlightDTO, theme: &AppTheme) -> impl IntoElement + use<> {
        let circle_size = ProfileHighlightsLayoutPolicy::CIRCLE_SIZE;
        let cover = item.cover_story.as_ref().filter(|cover| !cover.media_source().is_empty());
        let mut circle = zstack(Alignment::Center)
            .relative()
            .size(px(circle_size))
            .child(div().absolute().inset_0().rounded_full().bg(theme.bg._200));
        circle = match cover {
            Some(cover) => circle.child(
                div().size(px(circle_size)).rounded_full().overflow_hidden().child(
                    StoryThumbnailUi::from_story(
                        cover,
                        circle_size,
                        circle_size,
                        (circle_size * 3.0).ceil() as i64,
                    )
                    .corner_radius(circle_size / 2.0)
                    .shows_progress_indicator(true),
                ),
            ),
            None => circle.children(IconUi::sized("photo.on.rectangle", px(22.0), theme.text.secondary, false)),
        };
        vstack(ProfileHighlightsLayoutPolicy::ITEM_SPACING)
            .child(
                circle
                    .child(centred_stroke(theme.border._200.opacity(0.7), 2.0, Shape::Circle))
                    .shared_element_anchor(
                        ProfileHighlightsSharedElementOrigin::item_id(item.id),
                        SharedElementPart::Media,
                    ),
            )
            .child(
                MainText::new(item.title.clone())
                    .px(12.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.primary)
                    .number_of_lines(1)
                    .w(px(ProfileHighlightsLayoutPolicy::LABEL_WIDTH)),
            )
    }

    fn managed_circle(
        item: &HighlightDTO,
        is_first: bool,
        state: &Entity<ProfileHighlightsState>,
        sheets: &ProfileHighlightSheetsMount,
        origin: &Rc<ProfileHighlightsSharedElementOrigin>,
        shared_elements_active: bool,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let actions = Self::manage_actions();
        let content_item = item.clone();
        let content_theme = theme.clone();
        let action_item = item.clone();
        let action_state = state.clone();
        let action_sheets = sheets.clone();
        let tap_item = item.clone();
        let tap_origin = origin.clone();
        HoldContextMenuContainer::new(
            ElementId::NamedInteger(SharedString::new_static("profile-highlight-hold"), item.id as u64),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                Self::highlight_circle_content(&content_item, &content_theme)
                    .guide_anchor(is_first.then_some(RiseGuideAnchor::ProfileFirstHighlight))
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_manage_action(&id, &action_item, &action_state, &action_sheets, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            Self::open(&tap_item, &tap_origin, shared_elements_active, cx);
        }))
    }

    fn plain_circle(
        item: &HighlightDTO,
        origin: &Rc<ProfileHighlightsSharedElementOrigin>,
        shared_elements_active: bool,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let tap_item = item.clone();
        let tap_origin = origin.clone();
        plain(ElementId::NamedInteger(SharedString::new_static("profile-highlight"), item.id as u64))
            .on_press(move |_, cx| Self::open(&tap_item, &tap_origin, shared_elements_active, cx))
            .child(Self::highlight_circle_content(item, theme))
    }
}

impl RenderOnce for ProfileHighlights {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| ProfileHighlightsState {
            show_create: false,
            show_edit: false,
            editing_highlight: None,
            shared_element_origin: ProfileHighlightsSharedElementOrigin::new(),
            scroll_handle: ScrollHandle::new(),
        });
        let (origin, scroll_handle) = {
            let state = state.read(cx);
            (state.shared_element_origin.clone(), state.scroll_handle.clone())
        };
        let sheets = Self::sheets_mount(&self.id, &self.user_id, &state);
        let highlights = Rc::new(StoryServicesStore::shared(cx).update(cx, |service, cx| {
            service.highlights(&self.user_id, HighlightScope::Profile, cx)
        }));
        origin.record_latest_highlights(&highlights);

        let can_manage = self.is_owner || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author();
        let shared_elements_active = SharedElementNavigationPolicy::stories_enabled() && !self.user_id.is_empty();

        let change_origin = origin.clone();
        on_change(
            (self.id.clone(), "user-id"),
            self.user_id.clone(),
            false,
            window,
            cx,
            move |_, user_id, _, cx| {
                change_origin.configure(user_id);
                Self::load_highlights(user_id, cx);
            },
        );

        let row = (self.is_owner || !highlights.is_empty()).then(|| {
            let first_id = highlights.first().map(|item| item.id);
            let surface = shared_elements_active.then(|| ProfileHighlightsSharedElementOrigin::surface(&self.user_id));
            let probe = shared_elements_active.then(|| {
                let container: Rc<dyn SharedElementOriginContainer> = origin.clone();
                let layout_origin = origin.clone();
                SharedElementHostProbe::origin(container).on_layout(move |_, window, _| {
                    layout_origin.record_layout(window);
                })
            });
            let circles = highlights.iter().map(|item| {
                if can_manage {
                    Self::managed_circle(
                        item,
                        Some(item.id) == first_id,
                        &state,
                        &sheets,
                        &origin,
                        shared_elements_active,
                        &theme,
                    )
                    .into_any_element()
                } else {
                    Self::plain_circle(item, &origin, shared_elements_active, &theme).into_any_element()
                }
            });
            let add = self.is_owner.then(|| self.add_button(&state, &sheets, &theme));
            let items = hstack(ProfileHighlightsLayoutPolicy::COLUMN_SPACING)
                .relative()
                .flex_none()
                .items_start()
                .px(px(16.0))
                .py(px(2.0))
                .child(
                    div().absolute().inset_0().child(
                        PagerBackSwipeYield::new((self.id.clone(), "back-swipe-yield"), None)
                            .scroll_handle(scroll_handle.clone()),
                    ),
                )
                .children(probe)
                .children(add)
                .children(circles);
            env_scope::<SharedElementSurfaceKey>(surface).child(
                div()
                    .id((self.id.clone(), "scroll"))
                    .flex()
                    .w_full()
                    .h(px(ProfileHighlightsLayoutPolicy::ROW_HEIGHT))
                    .overflow_x_scroll()
                    .track_scroll(&scroll_handle)
                    .child(items),
            )
        });

        let appear_user_id = self.user_id.clone();
        let appear_origin = origin;
        let appear_sheets = sheets;
        div()
            .relative()
            .w_full()
            .children(row)
            .on_appear((self.id, "appear"), move |_, cx| {
                ProfileHighlightSheetsPresenter::offer(appear_sheets.clone(), cx);
                appear_origin.configure(&appear_user_id);
                Self::load_highlights(&appear_user_id, cx);
            })
    }
}
