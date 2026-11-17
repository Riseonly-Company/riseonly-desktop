use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, Subscription, Window, transparent_black, white,
};
use rise_canon::rise_request::map::UpdatePhotoGrid;
use rise_canon::{RiseGuideAnchor, RiseLimits};
use rise_i18n::tr;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::engine::core::rise_map_photo_upload::MapPhotoURL;
use crate::modules::map::engine::rise_map_engine_models::{RiseMapLoadState, RiseMapMutationOutcome};
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::stores::map_actions::map_photos_types::{MapPhoto, MapPhotoGrid};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;

type SheetCallback = Rc<dyn Fn(&mut Window, &mut App)>;

const DETAIL_MINIMUM_LINES: f32 = 3.0;
const DETAIL_MAXIMUM_LINES: usize = 6;
const DETAIL_LINE_HEIGHT: f32 = 22.0;

/// state of the grid editor sheet plus what its body tracks
struct MapGridEditorState {
    title: String,
    detail: String,
    cover_id: Option<String>,
    is_saving: bool,
    failed: bool,
    title_field: Entity<InputUiState>,
    detail_field: Entity<InputUiState>,
    _subscriptions: Vec<Subscription>,
}

impl MapGridEditorState {
    fn new(grid: &MapPhotoGrid, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = grid.display_title();
        let detail = grid.description.clone();
        let title_field = cx.new(|cx| {
            let mut field = InputUiState::single_line(cx);
            field.set_chromeless(true, cx);
            field.set_placeholder(tr("plan_title_label"), cx);
            field.reset(title.clone(), cx);
            field
        });
        let detail_field = cx.new(|cx| {
            let mut field = InputUiState::multi_line(cx);
            field.set_chromeless(true, cx);
            field.set_wraps(true, cx);
            field.set_max_visible_lines(DETAIL_MAXIMUM_LINES, cx);
            field.set_placeholder(tr("description_placeholder"), cx);
            field.reset(detail.clone(), cx);
            field
        });
        let presentation = RiseMapPresentation::shared(cx);
        let photos = presentation.read(cx).photos.clone();
        let sandbox = GuideSandbox::shared(cx);
        let subscriptions = vec![
            cx.subscribe_in(&title_field, window, |this, field, event: &InputUiEvent, window, cx| match event {
                InputUiEvent::Changed => {
                    this.title = field.read(cx).text().to_owned();
                    cx.notify();
                }
                InputUiEvent::Submitted | InputUiEvent::Cancelled => ScreenFocus::release_text_field(window, cx),
            }),
            cx.subscribe_in(&detail_field, window, |this, field, event: &InputUiEvent, window, cx| match event {
                InputUiEvent::Changed => {
                    this.detail = field.read(cx).text().to_owned();
                    cx.notify();
                }
                InputUiEvent::Submitted | InputUiEvent::Cancelled => ScreenFocus::release_text_field(window, cx),
            }),
            cx.observe(&photos, |_, _, cx| cx.notify()),
            cx.observe(&sandbox, |_, _, cx| cx.notify()),
        ];
        Self {
            title,
            detail,
            cover_id: grid.cover_photo_id.clone(),
            is_saving: false,
            failed: false,
            title_field,
            detail_field,
            _subscriptions: subscriptions,
        }
    }

    fn is_valid(&self) -> bool {
        i64::try_from(self.title.chars().count()).unwrap_or(i64::MAX)
            <= RiseLimits::service_value("map_max_place_name_chars")
            && i64::try_from(self.detail.chars().count()).unwrap_or(i64::MAX)
                <= RiseLimits::service_value("map_max_photo_caption_chars")
    }
}

/// sheet with name, description and cover of one photo grid
#[derive(IntoElement)]
pub struct MapGridEditor {
    grid: MapPhotoGrid,
    is_presented: Binding<bool>,
    on_did_dismiss: SheetCallback,
}

impl MapGridEditor {
    pub fn new(
        grid: MapPhotoGrid,
        is_presented: Binding<bool>,
        on_did_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { grid, is_presented, on_did_dismiss: Rc::new(on_did_dismiss) }
    }
}

#[derive(Clone)]
struct MapGridEditorParts {
    grid: MapPhotoGrid,
    is_presented: Binding<bool>,
    state: Entity<MapGridEditorState>,
}

impl MapGridEditorParts {
    fn id_for(&self, part: &str) -> ElementId {
        ElementId::Name(format!("map-grid-editor-{}-{part}", self.grid.id).into())
    }

    fn sheet_content(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let open = MapServicesStore::photos(cx).open_grid;
        let (title_field, detail_field, cover_id, failed) = {
            let state = self.state.read(cx);
            (state.title_field.clone(), state.detail_field.clone(), state.cover_id.clone(), state.failed)
        };
        title_field.update(cx, |field, cx| field.set_text_color(Some(theme.text.primary), cx));
        detail_field.update(cx, |field, cx| field.set_text_color(Some(theme.text.primary), cx));

        let mut covers = hstack(10.0);
        if let Some(open) = &open {
            let photos: &[MapPhoto] = if open.grid_id == self.grid.id { &open.photos } else { &[] };
            for photo in photos.iter().filter(|photo| !photo.is_pending) {
                let is_cover = cover_id.as_deref() == Some(photo.id.as_str())
                    || (cover_id.is_none() && photo.display_url() == self.grid.cover_url);
                let state = self.state.clone();
                let photo_id = photo.id.clone();
                let mark = is_cover.then(|| {
                    div()
                        .absolute()
                        .right(px(5.0))
                        .bottom(px(5.0))
                        .size(px(24.0))
                        .rounded_full()
                        .bg(theme.primary._100)
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("checkmark", px(13.0), theme.content_on(theme.primary._100), false))
                });
                covers = covers.child(
                    plain(self.id_for(&format!("cover-{}", photo.id)))
                        .on_press(move |_, cx| {
                            state.update(cx, |state, cx| {
                                state.cover_id = Some(photo_id.clone());
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .relative()
                                .child(MapGridThumbnail::new(photo.display_url().to_owned(), 88.0))
                                .children(mark),
                        ),
                );
            }
            if open.has_more {
                covers = covers.child(
                    div()
                        .frame_size(44.0, 88.0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(progress_view())
                        .on_appear(self.id_for("load-more"), |_, cx| {
                            MapInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.load_more_grid_photos(cx));
                        }),
                );
            }
        }

        let field_card = || div().w_full().p(px(14.0)).bg(theme.bg._200).rounded(px(16.0));
        let mut column = vstack(20.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(MainText::new(tr("edit_action")).px(22.0).font_weight(FontWeight::BOLD)),
            )
            .child(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(MainText::new(tr("plan_title_label")).px(13.0).color(theme.text.secondary))
                    .child(field_card().child(title_field)),
            )
            .child(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(MainText::new(tr("profile_field_description")).px(13.0).color(theme.text.secondary))
                    .child(
                        field_card()
                            .min_h(px(DETAIL_MINIMUM_LINES * DETAIL_LINE_HEIGHT + 28.0))
                            .child(detail_field),
                    ),
            )
            .child(MainText::new(tr("music_upload_cover")).px(15.0).font_weight(FontWeight::SEMIBOLD))
            .child(
                div()
                    .id(self.id_for("covers"))
                    .flex()
                    .w_full()
                    .overflow_x_scroll()
                    .child(covers.flex_none()),
            );
        if open.as_ref().is_some_and(|open| open.load_state == RiseMapLoadState::Failed) {
            column = column.child(
                plain(self.id_for("retry"))
                    .on_press(|_, cx| {
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.reopen_grid(cx));
                    })
                    .child(MainText::new(tr("common_retry")).px(17.0).color(theme.primary._100)),
            );
        }
        if failed {
            column = column.child(MainText::new(tr("map_photo_failed")).px(14.0).color(theme.text.secondary));
        }

        div()
            .w_full()
            .p(px(20.0))
            .pb(px(96.0))
            .child(column.guide_anchor(RiseGuideAnchor::MapGridEditorFields))
            .into_any_element()
    }

    fn footer(&self, cx: &mut App) -> AnyElement {
        let (is_valid, is_saving) = {
            let state = self.state.read(cx);
            (state.is_valid(), state.is_saving)
        };
        let parts = self.clone();
        let label = if is_saving {
            progress_view().tint(white()).into_any_element()
        } else {
            MainText::new(tr("save"))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white())
                .into_any_element()
        };
        ButtonUi::new(self.id_for("save"))
            .disabled(!is_valid || is_saving)
            .height(50.0)
            .on_press(move |window, cx| parts.save(window, cx))
            .child(label)
            .guide_anchor(RiseGuideAnchor::MapGridEditorSave)
            .into_any_element()
    }

    fn save(&self, window: &mut Window, cx: &mut App) {
        let (title, detail, cover_id) = {
            let state = self.state.read(cx);
            if !state.is_valid() || state.is_saving {
                return;
            }
            (state.title.clone(), state.detail.clone(), state.cover_id.clone())
        };
        self.state.update(cx, |state, cx| {
            state.is_saving = true;
            state.failed = false;
            cx.notify();
        });
        let body = UpdatePhotoGrid {
            cover_photo_id: cover_id,
            description: Some(detail),
            grid_id: self.grid.id.clone(),
            title: Some(if title == self.grid.display_title() { self.grid.title.clone() } else { title }),
        };
        let interactions = MapInteractionsStore::shared(cx);
        let update = interactions.update(cx, |interactions, cx| interactions.update_grid(body, cx));
        let parts = self.clone();
        window
            .spawn(cx, async move |cx| {
                let outcome = update.await;
                let _ = cx.update(|window, cx| {
                    if interactions.read(cx).grid_to_edit_id() != Some(parts.grid.id.as_str()) {
                        return;
                    }
                    let done = matches!(outcome, RiseMapMutationOutcome::Done);
                    parts.state.update(cx, |state, cx| {
                        state.is_saving = false;
                        state.failed = !done;
                        cx.notify();
                    });
                    if done {
                        parts.is_presented.set(false, window, cx);
                    }
                });
            })
            .detach();
    }
}

impl RenderOnce for MapGridEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let grid = self.grid;
        let seed = grid.clone();
        let state = window.use_keyed_state(
            ElementId::Name(format!("map-grid-editor-{}", grid.id).into()),
            cx,
            move |window, cx| MapGridEditorState::new(&seed, window, cx),
        );
        let parts = MapGridEditorParts { grid, is_presented: self.is_presented.clone(), state };
        let content_parts = parts.clone();
        let footer_parts = parts.clone();
        let did_dismiss = self.on_did_dismiss.clone();
        let refused_dismiss = self.on_did_dismiss;
        let refused_presented = self.is_presented.clone();
        BottomSheetUi::new(
            parts.id_for("sheet"),
            self.is_presented,
            move |_, cx| content_parts.sheet_content(cx),
            move |_, cx| footer_parts.footer(cx),
        )
        .snap_fractions(vec![0.7, 0.93])
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(16.0))
        .footer_top_padding(Some(8.0))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .on_did_dismiss(move |window, cx| did_dismiss(window, cx))
        .on_presentation_refused(move |window, cx| {
            refused_presented.set(false, window, cx);
            refused_dismiss(window, cx);
        })
    }
}

#[derive(IntoElement)]
pub struct MapGridThumbnail {
    source: String,
    size: f32,
}

impl MapGridThumbnail {
    pub fn new(source: impl Into<String>, size: f32) -> Self {
        Self { source: source.into(), size }
    }
}

impl RenderOnce for MapGridThumbnail {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._300;
        let image = MapPhotoURL::absolute(&self.source).map(|source| {
            CleverImageUi::new(source)
                .content_mode(ContentMode::Fill)
                .corner_radius(14.0)
                .persist_to_disk(false)
                .maximum_decode_pixel_dimension(320_i64)
                .cache_policy(CleverImageCachePolicy::MemoryOnly)
        });
        div()
            .flex_none()
            .size(px(self.size))
            .rounded(px(14.0))
            .overflow_hidden()
            .bg(background)
            .children(image)
    }
}
