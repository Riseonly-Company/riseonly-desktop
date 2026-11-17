use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, ScrollHandle, Stateful, Subscription, Window, black, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification, selection_changed};
use rise_theme::AppTheme;
use rise_ui::error_text_ui::system_red;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::modules::story::components::story_thumbnail_ui::StoryThumbnailUi;
use crate::modules::story::stores::story::story_actions::story_actions::StoryActionsStore;
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    HighlightDTO, HighlightScope, StoryDTO, StoryDTOExt,
};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_top::highlight_create_sheet::{
    HighlightGridMetrics, HighlightSheetAction, HighlightSheetActionPill, HighlightStorySelectionView, child_id,
    highlight_cover, highlight_title_field, with_window,
};

/// edits a highlight,
/// title cover and story set. page two is the story picker,
/// opened by the owner or a viewer acting as author
#[derive(IntoElement)]
pub struct HighlightEditSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    highlight: Option<HighlightDTO>,
    owner_user_id: String,
}

struct HighlightEditSheetState {
    title: String,
    cover_story_id: Option<i64>,
    selected_to_add: Vec<i64>,
}

impl HighlightEditSheet {
    const EDIT_PAGE_ID: &'static str = "edit";
    const ADD_PAGE_ID: &'static str = "add";

    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        highlight: Option<HighlightDTO>,
        owner_user_id: impl Into<String>,
    ) -> Self {
        Self { id: id.into(), is_presented, highlight, owner_user_id: owner_user_id.into() }
    }

    fn pages(&self, state: &Entity<HighlightEditSheetState>) -> Vec<BottomSheetScreenItem> {
        let read = state.clone();
        let write = state.clone();
        let title = Binding::new(
            move |cx: &App| read.read(cx).title.clone(),
            move |value: String, _: &mut Window, cx: &mut App| {
                write.update(cx, |state, cx| {
                    state.title = value;
                    cx.notify();
                });
            },
        );
        let read = state.clone();
        let write = state.clone();
        let cover_story_id = Binding::new(
            move |cx: &App| read.read(cx).cover_story_id,
            move |value: Option<i64>, _: &mut Window, cx: &mut App| {
                write.update(cx, |state, cx| {
                    state.cover_story_id = value;
                    cx.notify();
                });
            },
        );
        let read = state.clone();
        let write = state.clone();
        let selected_to_add = Binding::new(
            move |cx: &App| read.read(cx).selected_to_add.clone(),
            move |value: Vec<i64>, _: &mut Window, cx: &mut App| {
                write.update(cx, |state, cx| {
                    state.selected_to_add = value;
                    cx.notify();
                });
            },
        );

        let edit_id = child_id(&self.id, "edit-page");
        let edit_highlight = self.highlight.clone();
        let edit_owner_user_id = self.owner_user_id.clone();
        let is_presented = self.is_presented.clone();
        let add_id = child_id(&self.id, "add-page");
        let add_highlight_id = self.highlight.as_ref().map(|highlight| highlight.id);
        let add_owner_user_id = self.owner_user_id.clone();
        let add_state = state.clone();
        vec![
            BottomSheetScreenItem::new(
                Self::EDIT_PAGE_ID,
                tr("highlights_edit_title"),
                HighlightGridMetrics::SHEET_FRACTION,
                move |_: &mut Window, _: &mut App| match edit_highlight.clone() {
                    Some(highlight) => {
                        let close = is_presented.clone();
                        HighlightEditView {
                            id: edit_id.clone(),
                            highlight,
                            owner_user_id: edit_owner_user_id.clone(),
                            title: title.clone(),
                            cover_story_id: cover_story_id.clone(),
                            on_close: Rc::new(move |window: &mut Window, cx: &mut App| close.set(false, window, cx)),
                        }
                        .into_any_element()
                    }
                    None => div().into_any_element(),
                },
            ),
            BottomSheetScreenItem::new(
                Self::ADD_PAGE_ID,
                tr("highlight_add_stories"),
                HighlightGridMetrics::SHEET_FRACTION,
                move |_: &mut Window, _: &mut App| {
                    let state = add_state.clone();
                    let owner_user_id = add_owner_user_id.clone();
                    HighlightStorySelectionView::new(
                        add_id.clone(),
                        add_owner_user_id.clone(),
                        selected_to_add.clone(),
                        move |navigation, window, cx| {
                            Self::add_selected_stories(add_highlight_id, &owner_user_id, &state, cx);
                            if let Some(navigation) = navigation {
                                BottomSheetNavigationState::go_back(&navigation, window, cx);
                            }
                        },
                    )
                },
            ),
        ]
    }

    fn seed(highlight: Option<&HighlightDTO>, state: &Entity<HighlightEditSheetState>, cx: &mut App) {
        let title = highlight.map_or_else(String::new, |highlight| highlight.title.clone());
        let cover_story_id = highlight.and_then(|highlight| highlight.cover_story.as_ref()).map(|story| story.id);
        state.update(cx, |state, cx| {
            state.title = title;
            state.cover_story_id = cover_story_id;
            state.selected_to_add = Vec::new();
            cx.notify();
        });
    }

    fn add_selected_stories(
        highlight_id: Option<i64>,
        owner_user_id: &str,
        state: &Entity<HighlightEditSheetState>,
        cx: &mut App,
    ) {
        let Some(highlight_id) = highlight_id else {
            return;
        };
        let ids = state.update(cx, |state, cx| {
            cx.notify();
            std::mem::take(&mut state.selected_to_add)
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.add_stories_to_highlight(highlight_id, &ids, Some(owner_user_id), cx);
        });
    }
}

impl RenderOnce for HighlightEditSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| HighlightEditSheetState {
            title: String::new(),
            cover_story_id: None,
            selected_to_add: Vec::new(),
        });
        let pages = self.pages(&state);
        let seed_highlight = self.highlight.clone();
        let refused = self.is_presented.clone();
        BottomSheetPagesUi::new(child_id(&self.id, "pages"), self.is_presented.clone(), pages, Self::EDIT_PAGE_ID)
            .sheet_background(Some(background))
            .on_present(move |_, cx| Self::seed(seed_highlight.as_ref(), &state, cx))
            .on_presentation_refused(move |window, cx| refused.set(false, window, cx))
    }
}

enum HighlightEditMetrics {}

impl HighlightEditMetrics {
    const COVER_SIZE: f32 = 132.0;
    const TILE_WIDTH: f32 = 72.0;
    const TILE_HEIGHT: f32 = 128.0;
    const TILE_SPACING: f32 = 8.0;
    const TILE_CORNER_RADIUS: f32 = 10.0;
    const TILE_OVERDRAW: usize = 4;
    const FIRST_FRAME_STRIP_WIDTH: f32 = 720.0;
    const STRIP_HORIZONTAL_PADDING: f32 = 16.0;
}

#[derive(IntoElement)]
struct HighlightEditView {
    id: ElementId,
    highlight: HighlightDTO,
    owner_user_id: String,
    title: Binding<String>,
    cover_story_id: Binding<Option<i64>>,
    on_close: HighlightSheetAction,
}

struct HighlightEditViewState {
    story_pending_removal: Option<StoryDTO>,
    error_text: Option<String>,
    strip_scroll: ScrollHandle,
    _service_observation: Subscription,
}

#[derive(Clone)]
struct HighlightEditViewContext {
    highlight: Rc<HighlightDTO>,
    owner_user_id: Rc<str>,
    title: Binding<String>,
    cover_story_id: Binding<Option<i64>>,
    on_close: HighlightSheetAction,
    state: Entity<HighlightEditViewState>,
}

impl HighlightEditViewContext {
    fn title_changed_value(&self, cx: &App) -> (String, bool, bool) {
        let title = self.title.get(cx);
        let trimmed = trimming_whitespaces_and_newlines(&title).to_owned();
        let title_changed = trimmed != self.highlight.title;
        let cover = self.cover_story_id.get(cx);
        let cover_changed =
            cover.is_some() && cover != self.highlight.cover_story.as_ref().map(|story| story.id);
        (trimmed, title_changed, cover_changed)
    }

    fn save(&self, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.error_text = None;
            cx.notify();
        });
        let (trimmed, title_changed, cover_changed) = self.title_changed_value(cx);
        if !title_changed && !cover_changed {
            (self.on_close)(window, cx);
            return;
        }
        let failed = self.state.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            let _ = notification(NotificationKind::Error);
            failed.update(cx, |state, cx| {
                state.error_text = Some(message);
                cx.notify();
            });
        });
        let cover_story_id = self.cover_story_id.get(cx);
        let highlight_id = self.highlight.id;
        let owner_user_id = self.owner_user_id.clone();
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_highlight(
                highlight_id,
                title_changed.then_some(trimmed.as_str()),
                if cover_changed { cover_story_id } else { None },
                Some(&*owner_user_id),
                Some(on_error),
                cx,
            );
        });
        if self.state.read(cx).error_text.is_none() {
            let _ = notification(NotificationKind::Success);
            (self.on_close)(window, cx);
        }
    }

    fn remove(&self, story: &StoryDTO, is_last: bool, stories: &[StoryDTO], window: &mut Window, cx: &mut App) {
        let _ = notification(NotificationKind::Warning);
        let highlight_id = self.highlight.id;
        let owner_user_id = self.owner_user_id.clone();
        if is_last {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.delete_highlight(highlight_id, Some(&*owner_user_id), HighlightScope::Profile, cx);
            });
            (self.on_close)(window, cx);
            return;
        }
        if self.cover_story_id.get(cx) == Some(story.id) {
            let next = stories.iter().find(|candidate| candidate.id != story.id).map(|candidate| candidate.id);
            self.cover_story_id.set(next, window, cx);
        }
        let story_id = story.id;
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.remove_story_from_highlight(highlight_id, story_id, Some(&*owner_user_id), cx);
        });
    }

    fn present_removal_alert(&self, stories: Rc<Vec<StoryDTO>>, window: &mut Window, cx: &mut App) {
        let Some(story) = self.state.read(cx).story_pending_removal.clone() else {
            return;
        };
        let is_last = stories.len() <= 1;
        let message =
            if is_last { tr("highlight_remove_last_story_message") } else { tr("highlight_remove_story_message") };
        let cancelled = self.state.clone();
        let confirmed = self.clone();
        let handle = window.window_handle();
        NativeConfirmAlert::present_delete(
            &tr("highlight_remove_story_title"),
            &message,
            Some(tr("highlight_remove_story_action")),
            None,
            false,
            None,
            Some(Box::new(move |cx: &mut App| {
                cancelled.update(cx, |state, cx| {
                    state.story_pending_removal = None;
                    cx.notify();
                });
            })),
            move |cx: &mut App| {
                confirmed.state.update(cx, |state, cx| {
                    state.story_pending_removal = None;
                    cx.notify();
                });
                with_window(handle, move |window, cx| confirmed.remove(&story, is_last, &stories, window, cx), cx);
            },
            cx,
        );
    }

    fn cover(&self, stories: &[StoryDTO], theme: &AppTheme, cx: &App) -> Div {
        let cover_story_id = self.cover_story_id.get(cx);
        let cover_story = cover_story_id
            .and_then(|cover_story_id| stories.iter().find(|story| story.id == cover_story_id))
            .filter(|story| !story.media_source().is_empty())
            .or(self.highlight.cover_story.as_ref());
        highlight_cover(
            cover_story,
            HighlightEditMetrics::COVER_SIZE,
            (HighlightEditMetrics::COVER_SIZE * 3.0) as i64,
            theme,
        )
    }

    fn stories_strip(
        &self,
        id: &ElementId,
        stories: &Rc<Vec<StoryDTO>>,
        navigation: Option<Entity<BottomSheetNavigationState>>,
        theme: &AppTheme,
        cx: &App,
    ) -> Stateful<Div> {
        let scroll = self.state.read(cx).strip_scroll.clone();
        let step = HighlightEditMetrics::TILE_WIDTH + HighlightEditMetrics::TILE_SPACING;
        let scrolled = (-f32::from(scroll.offset().x)).max(0.0);
        let viewport = match f32::from(scroll.bounds().size.width) {
            width if width > 0.0 => width,
            _ => HighlightEditMetrics::FIRST_FRAME_STRIP_WIDTH,
        };
        let first_visible = (scrolled / step).floor() as usize;
        let first = first_visible.saturating_sub(HighlightEditMetrics::TILE_OVERDRAW).min(stories.len());
        let last = (first_visible + (viewport / step).ceil() as usize + HighlightEditMetrics::TILE_OVERDRAW)
            .min(stories.len());
        let last_id = stories.last().map(|story| story.id);
        let tiles = stories[first..last.max(first)].iter().map(|story| {
            let tile = self.story_tile(story, stories, theme, cx);
            if Some(story.id) != last_id {
                return tile.into_any_element();
            }
            let highlight_id = self.highlight.id;
            tile.on_appear(("highlight-edit-last-story", story.id as u64), move |_, cx| {
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_highlight_stories(highlight_id, cx));
            })
            .into_any_element()
        });
        div()
            .id(child_id(id, "strip"))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&scroll)
            .gap(px(HighlightEditMetrics::TILE_SPACING))
            .px(px(HighlightEditMetrics::STRIP_HORIZONTAL_PADDING))
            .py(px(4.0))
            .when(first > 0, |strip| {
                strip.child(
                    div()
                        .flex_shrink_0()
                        .w(px(first as f32 * step - HighlightEditMetrics::TILE_SPACING)),
                )
            })
            .children(tiles.collect::<Vec<_>>())
            .when(last < stories.len(), |strip| {
                strip.child(
                    div()
                        .flex_shrink_0()
                        .w(px((stories.len() - last) as f32 * step - HighlightEditMetrics::TILE_SPACING)),
                )
            })
            .child(self.add_tile(id, navigation, theme))
    }

    fn story_tile(&self, story: &StoryDTO, stories: &Rc<Vec<StoryDTO>>, theme: &AppTheme, cx: &App) -> impl IntoElement + use<> {
        let is_cover = self.cover_story_id.get(cx) == Some(story.id);
        let story_id = story.id;
        let select = self.cover_story_id.clone();
        let removal = self.clone();
        let removal_story = story.clone();
        let removal_stories = stories.clone();
        let source_is_empty = story.media_source().is_empty();
        div()
            .id(("highlight-edit-tile", story.id as u64))
            .relative()
            .flex_shrink_0()
            .w(px(HighlightEditMetrics::TILE_WIDTH))
            .h(px(HighlightEditMetrics::TILE_HEIGHT))
            .bg(theme.bg._200)
            .continuous_corners(HighlightEditMetrics::TILE_CORNER_RADIUS)
            .overflow_hidden()
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let _ = selection_changed();
                select.set(Some(story_id), window, cx);
            })
            .when(!source_is_empty, |tile| {
                tile.child(
                    StoryThumbnailUi::from_story(
                        story,
                        HighlightEditMetrics::TILE_WIDTH,
                        HighlightEditMetrics::TILE_HEIGHT,
                        (HighlightEditMetrics::TILE_HEIGHT * 3.0) as i64,
                    )
                    .corner_radius(HighlightEditMetrics::TILE_CORNER_RADIUS),
                )
            })
            .when(is_cover, |tile| {
                tile.child(
                    div()
                        .absolute()
                        .inset_0()
                        .continuous_corners(HighlightEditMetrics::TILE_CORNER_RADIUS)
                        .stroke_border(theme.primary._100, 2.5),
                )
            })
            .child(
                div()
                    .id(("highlight-edit-tile-remove", story.id as u64))
                    .absolute()
                    .top_0()
                    .right_0()
                    .occlude()
                    .aria_label(tr("highlight_remove_story_action"))
                    .tooltip_label(tr("highlight_remove_story_action"))
                    .child(
                        plain(("highlight-edit-tile-remove-button", story.id as u64))
                            .p(px(4.0))
                            .on_press(move |window, cx| {
                                let story = removal_story.clone();
                                removal.state.update(cx, |state, cx| {
                                    state.story_pending_removal = Some(story);
                                    cx.notify();
                                });
                                removal.present_removal_alert(removal_stories.clone(), window, cx);
                            })
                            .child(
                                div()
                                    .w(px(20.0))
                                    .h(px(20.0))
                                    .rounded_full()
                                    .bg(black().opacity(0.6))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(IconUi::sized("minus", px(12.0), white(), false)),
                            ),
                    ),
            )
    }

    fn add_tile(
        &self,
        id: &ElementId,
        navigation: Option<Entity<BottomSheetNavigationState>>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        div()
            .id(child_id(id, "add-tile"))
            .relative()
            .flex_shrink_0()
            .w(px(HighlightEditMetrics::TILE_WIDTH))
            .h(px(HighlightEditMetrics::TILE_HEIGHT))
            .bg(theme.bg._200)
            .continuous_corners(HighlightEditMetrics::TILE_CORNER_RADIUS)
            .stroke_border(theme.border._200.opacity(0.7), 1.0)
            .child(
                plain(child_id(id, "add-tile-button"))
                    .size_full()
                    .on_press(move |window, cx| {
                        if let Some(navigation) = navigation.as_ref() {
                            BottomSheetNavigationState::navigate(
                                navigation,
                                HighlightEditSheet::ADD_PAGE_ID,
                                window,
                                cx,
                            );
                        }
                    })
                    .child(
                        vstack(6.0)
                            .size_full()
                            .justify_center()
                            .children(IconUi::sized("plus", px(22.0), theme.text.primary, false))
                            .child(
                                div().px(px(4.0)).child(
                                    MainText::new(tr("highlight_add_stories"))
                                        .px(11.0)
                                        .tac(TextAlignT::Center)
                                        .color(theme.text.secondary)
                                        .number_of_lines(2),
                                ),
                            ),
                    ),
            )
    }
}

impl HighlightEditView {
    fn title_rule() -> RiseFieldRule {
        RiseFieldRule::new("story_update_highlight", "/title")
    }
}

impl RenderOnce for HighlightEditView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx: &mut Context<HighlightEditViewState>| {
            let service = StoryServicesStore::shared(cx);
            HighlightEditViewState {
                story_pending_removal: None,
                error_text: None,
                strip_scroll: ScrollHandle::new(),
                _service_observation: cx.observe(&service, |_, _, cx| cx.notify()),
            }
        });
        let highlight_id = self.highlight.id;
        let highlight_cover_id = self.highlight.cover_story.as_ref().map(|story| story.id);
        let stories = Rc::new(
            StoryServicesStore::shared(cx).update(cx, |service, cx| service.highlight_stories(highlight_id, cx)),
        );
        let context = HighlightEditViewContext {
            highlight: Rc::new(self.highlight),
            owner_user_id: Rc::from(self.owner_user_id.as_str()),
            title: self.title.clone(),
            cover_story_id: self.cover_story_id.clone(),
            on_close: self.on_close.clone(),
            state: state.clone(),
        };

        on_change(
            child_id(&self.id, "cover-changed"),
            self.cover_story_id.get(cx),
            true,
            window,
            cx,
            move |_, cover, _, cx| {
                GuideFacts::set(
                    RiseGuideSignal::HighlightCoverChanged,
                    cover.is_some() && *cover != highlight_cover_id,
                    cx,
                );
            },
        );

        let error_text = state.read(cx).error_text.clone();
        let is_pending = StoryActionsStore::shared(cx).read(cx).update_highlight_sai.is_pending();
        let (trimmed, title_changed, cover_changed) = context.title_changed_value(cx);
        let has_changes = title_changed || cover_changed;
        let is_title_empty = trimmed.is_empty();
        let navigation = bottom_sheet_navigation(cx);
        let on_submit = context.clone();
        let on_cancel = self.on_close.clone();
        let on_done = context.clone();
        let cover = context.cover(&stories, &theme, cx);
        let strip = context.stories_strip(&self.id, &stories, navigation, &theme, cx);

        scroll_view(child_id(&self.id, "scroll"))
            .shows_indicators(false)
            .size_full()
            .bg(theme.bg._100)
            .child(
                vstack(14.0)
                    .w_full()
                    .pb(px(24.0))
                    .child(div().pt(px(20.0)).child(cover))
                    .child(
                        hstack(0.0).w_full().pt(px(6.0)).px(px(24.0)).child(
                            highlight_title_field(
                                child_id(&self.id, "title"),
                                Self::title_rule(),
                                self.title.clone(),
                                &theme,
                            )
                            .on_submit(move |window, cx| on_submit.save(window, cx))
                            .on_cancel(move |window, cx| on_cancel(window, cx)),
                        ),
                    )
                    .children(match error_text {
                        Some(error_text) => Some(div().px(px(24.0)).child(
                            MainText::new(error_text)
                                .px(13.0)
                                .tac(TextAlignT::Center)
                                .color(system_red(cx)),
                        )),
                        None if is_title_empty => Some(div().px(px(24.0)).child(
                            MainText::new(tr("story_highlight_name_required"))
                                .px(13.0)
                                .tac(TextAlignT::Center)
                                .color(theme.semantic.error_100),
                        )),
                        None => None,
                    })
                    .child(
                        div().pt(px(8.0)).px(px(24.0)).child(
                            MainText::new(tr("highlight_cover_hint"))
                                .px(13.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary),
                        ),
                    )
                    .child(div().w_full().child(strip).guide_anchor(RiseGuideAnchor::HighlightEditStories))
                    .child(
                        div().pt(px(8.0)).child(
                            HighlightSheetActionPill::new(child_id(&self.id, "done"), tr("done"), move |window, cx| {
                                on_done.save(window, cx);
                            })
                            .disabled(!has_changes || is_title_empty)
                            .pending(is_pending)
                            .guide_anchor(RiseGuideAnchor::HighlightEditDone),
                        ),
                    ),
            )
            .on_appear(child_id(&self.id, "appear"), move |_, cx| {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_highlight_stories(highlight_id, cx));
                GuideFacts::set(RiseGuideSignal::HighlightEditOpen, true, cx);
            })
            .on_disappear(child_id(&self.id, "disappear"), |_, cx| {
                GuideFacts::set(RiseGuideSignal::HighlightEditOpen, false, cx);
                GuideFacts::set(RiseGuideSignal::HighlightCoverChanged, false, cx);
            })
    }
}
