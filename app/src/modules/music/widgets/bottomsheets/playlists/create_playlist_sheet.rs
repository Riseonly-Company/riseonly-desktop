use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    App, AppContext as _, Axis, Context, Render, Background, Bounds, ElementId, Entity, EntityId, FillOptions, FontWeight, Hsla, Path, PathBuilder, PathStyle,
    Pixels, Point, SharedString, StyleRefinement, WeakEntity, Window, canvas, point, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::bot::components::bot_ui_components::BotSettingsFieldRow;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicPlaylist;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

const SELECTION_SPRING_RESPONSE: f64 = 0.2;
const SELECTION_SPRING_DAMPING: f64 = 0.8;

pub type CreatePlaylistSheetOnCreated = Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)>;

/// form state
/// desktop has no on-screen keyboard so the keyboard lift fields of the mobile form have no twin
struct CreatePlaylistSheetState {
    title: String,
    description: String,
    visibility: PlaylistVisibility,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    submit_error: Option<String>,
    sheet_fraction: f32,
    visibility_list: Option<Entity<CreatePlaylistVisibilityListView>>,
}

impl CreatePlaylistSheetState {
    fn new() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            visibility: PlaylistVisibility::Private,
            local_cover_file: None,
            cover_preview: None,
            submit_error: None,
            sheet_fraction: CreatePlaylistSheetPresentation::MINIMUM_FRACTION,
            visibility_list: None,
        }
    }

    fn normalized_title(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.title)
    }

    fn normalized_description(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.description)
    }

    /// validation text shows only once the field is non-empty, no complaints about an untouched form
    fn title_error(&self) -> Option<String> {
        if self.normalized_title().is_empty() {
            return None;
        }
        MusicFieldRules::playlist_title().error_text(&self.title)
    }

    fn description_error(&self) -> Option<String> {
        if self.normalized_description().is_empty() {
            return None;
        }
        MusicFieldRules::playlist_description().error_text(&self.description)
    }

    fn can_submit(&self, is_submitting: bool) -> bool {
        !self.normalized_title().is_empty()
            && MusicFieldRules::playlist_title().is_satisfied(&self.title)
            && !is_submitting
    }

    fn prepare_form(&mut self, locked_visibility: Option<PlaylistVisibility>) {
        self.title = String::new();
        self.description = String::new();
        self.visibility = locked_visibility.unwrap_or(PlaylistVisibility::Private);
        self.local_cover_file = None;
        self.cover_preview = None;
        self.submit_error = None;
    }
}

fn cover_upload_progress(cx: &mut App) -> f64 {
    MusicActionsStore::slices(cx)
        .create_playlist_sai
        .data()
        .and_then(|mutation| mutation.cover_upload_progress)
        .unwrap_or(0.)
}

fn is_cover_uploading(cx: &mut App) -> bool {
    MusicActionsStore::slices(cx).create_playlist_sai.data().and_then(|mutation| mutation.cover_uploading)
        == Some(true)
}

fn is_submitting(cx: &mut App) -> bool {
    MusicActionsStore::slices(cx).create_playlist_sai.is_pending() || is_cover_uploading(cx)
}

/// playlist creation sheet that owns its own presentation
/// height comes from measured content, form scrolls on short windows
/// button lives in the shell footer and content reserves its space
#[derive(IntoElement)]
pub struct CreatePlaylistSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    locked_visibility: Option<PlaylistVisibility>,
    on_created: Option<CreatePlaylistSheetOnCreated>,
}

impl CreatePlaylistSheet {
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>) -> Self {
        Self { id: id.into(), is_presented, locked_visibility: None, on_created: None }
    }

    pub fn locked_visibility(mut self, locked_visibility: Option<PlaylistVisibility>) -> Self {
        self.locked_visibility = locked_visibility;
        self
    }

    pub fn on_created(mut self, on_created: impl Fn(Option<RiseStoredMusicPlaylist>, &mut App) + 'static) -> Self {
        self.on_created = Some(Rc::new(on_created));
        self
    }
}

impl RenderOnce for CreatePlaylistSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("state")),
            cx,
            |_, _| CreatePlaylistSheetState::new(),
        );
        let locked_visibility = self.locked_visibility;
        let preparing = state.clone();
        on_change(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("presented")),
            self.is_presented.get(cx),
            true,
            window,
            cx,
            move |_, open, _, cx| {
                if *open {
                    preparing.update(cx, |form, cx| {
                        form.prepare_form(locked_visibility);
                        cx.notify();
                    });
                }
            },
        );

        let parts = CreatePlaylistSheetParts {
            id: self.id.clone(),
            state: state.clone(),
            is_presented: self.is_presented.clone(),
            locked_visibility,
            on_created: self.on_created.clone(),
        };
        let content_parts = parts.clone();
        let footer_parts = parts;
        let submitting = is_submitting(cx);
        let sheet_fraction = state.read(cx).sheet_fraction;

        BottomSheetUi::new(
            self.id.clone(),
            self.is_presented,
            move |window, cx| content_parts.sheet_content(window, cx),
            move |window, cx| footer_parts.submit_button(window, cx),
        )
        .snap_fractions(vec![sheet_fraction])
        .full_page_allows_swipe_dismiss(!submitting)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .keyboard_footer_spacing(CreatePlaylistSheetPresentation::KEYBOARD_FOOTER_SPACING)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(0.))
        .footer_top_padding(Some(0.))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }
}

#[derive(Clone)]
struct CreatePlaylistSheetParts {
    id: ElementId,
    state: Entity<CreatePlaylistSheetState>,
    is_presented: Binding<bool>,
    locked_visibility: Option<PlaylistVisibility>,
    on_created: Option<CreatePlaylistSheetOnCreated>,
}

impl CreatePlaylistSheetParts {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn sheet_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let body = self.sheet_body(window, cx);
        scroll_view(self.child_id("scroll")).shows_indicators(false).size_full().child(body).into_any_element()
    }

    fn sheet_body(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let cover_uploading = is_cover_uploading(cx);
        let has_cover = self.state.read(cx).local_cover_file.is_some();
        let closing = self.is_presented.clone();
        let local_cover_file = Binding::entity(
            self.state.clone(),
            |form: &CreatePlaylistSheetState| form.local_cover_file.clone(),
            |form, value, _| {
                form.local_cover_file = value;
            },
        );
        let cover_preview = Binding::entity(
            self.state.clone(),
            |form: &CreatePlaylistSheetState| form.cover_preview.clone(),
            |form, value, _| {
                form.cover_preview = value;
            },
        );
        let measuring = self.state.clone();

        let mut body = vstack(16.)
            .relative()
            .w_full()
            .pb(theme.button.height_300 + px(CreatePlaylistSheetPresentation::FOOTER_CHROME))
            .child(
                SheetHeaderBar::new(self.child_id("header"), tr("music_create_playlist"), move |window, cx| {
                    closing.set(false, window, cx);
                })
                .right_system_image(None),
            )
            .child(
                MusicCoverPickerUi::new(self.child_id("cover"), local_cover_file, cover_preview)
                    .placeholder_label(None::<String>)
                    .cover_upload_progress(cover_upload_progress(cx))
                    .show_upload_overlay(cover_uploading && has_cover),
            )
            .child(
                GroupedBtns::new(self.child_id("fields"), self.field_items(cx))
                    .corner_radius(CreatePlaylistSheetPresentation::CARD_CORNER_RADIUS),
            )
            .child(self.visibility_list(cx));

        if cover_uploading {
            body = body.child(SecondaryText::new(tr("music_upload_cover_uploading")).px(12.).tac(TextAlignT::Center));
        }

        let submitting = is_submitting(cx);
        let can_submit = self.state.read(cx).can_submit(submitting);
        body.child(measure(self.child_id("body-height"), move |bounds, window, cx| {
            let measured = BottomSheetMeasuredFraction::value(
                f32::from(bounds.size.height),
                CreatePlaylistSheetPresentation::MINIMUM_FRACTION,
                window,
            );
            measuring.update(cx, |form, cx| {
                if (measured - form.sheet_fraction).abs() > 0.005 {
                    form.sheet_fraction = measured;
                    cx.notify();
                }
            });
        }))
        .form_primary(can_submit, {
            let parts = self.clone();
            move |window, cx| parts.submit(window, cx)
        })
        .into_any_element()
    }

    /// fields are card rows like the profile settings page
    /// canon rule clamps input and prints the violation in the right slot
    fn field_items(&self, cx: &App) -> Vec<GroupedBtn> {
        let form = self.state.read(cx);
        let title_error = form.title_error();
        let description_error = form.description_error();
        let title = Binding::entity(
            self.state.clone(),
            |form: &CreatePlaylistSheetState| form.title.clone(),
            |form, value, _| {
                form.title = value;
            },
        );
        let description = Binding::entity(
            self.state.clone(),
            |form: &CreatePlaylistSheetState| form.description.clone(),
            |form, value, _| {
                form.description = value;
            },
        );
        vec![
            GroupedBtn::new("title", "").content_slot(
                BotSettingsFieldRow::new(
                    self.child_id("title-row"),
                    "music.note.list",
                    SettingsIconPalette::customization(),
                    tr("music_playlist_title_placeholder"),
                    MusicFieldRules::playlist_title().binding(title),
                )
                .rule(MusicFieldRules::playlist_title())
                .error_message(title_error),
            ),
            GroupedBtn::new("description", "").content_slot(
                BotSettingsFieldRow::new(
                    self.child_id("description-row"),
                    "text.alignleft",
                    SettingsIconPalette::privacy_description(),
                    tr("music_playlist_description_placeholder"),
                    MusicFieldRules::playlist_description().binding(description),
                )
                .axis(Axis::Vertical)
                .autocorrection_disabled(false)
                .rule(MusicFieldRules::playlist_description())
                .error_message(description_error),
            ),
        ]
    }

    /// the type card
    /// once its height is known it is a cached view since the sheet is a deferred draw
    /// a pick only splices in as a hole when the changed view is cached
    fn visibility_list(&self, cx: &mut App) -> AnyElement {
        let list = self.visibility_list_view(cx);
        let height = list.read(cx).height;
        match height {
            Some(height) => list.cached(StyleRefinement::default().w_full().h(height)).into_any_element(),
            None => list.into_any_element(),
        }
    }

    fn visibility_list_view(&self, cx: &mut App) -> Entity<CreatePlaylistVisibilityListView> {
        let id = self.id.clone();
        let form = self.state.downgrade();
        let locked_visibility = self.locked_visibility;
        if let Some(list) = self.state.read(cx).visibility_list.clone() {
            list.update(cx, |list, _| {
                list.id = id;
                list.form = form;
                list.locked_visibility = locked_visibility;
            });
            return list;
        }
        let list = cx.new(|_| CreatePlaylistVisibilityListView { id, form, locked_visibility, height: None });
        let stored = list.clone();
        self.state.update(cx, |form, _| form.visibility_list = Some(stored));
        list
    }

    fn submit_button(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let submitting = is_submitting(cx);
        let can_submit = self.state.read(cx).can_submit(submitting);
        let parts = self.clone();
        div()
            .w_full()
            .px(px(CreatePlaylistSheetPresentation::FOOTER_HORIZONTAL_PADDING))
            .pt(px(CreatePlaylistSheetPresentation::FOOTER_TOP_PADDING))
            .child(
                ButtonUi::new(self.child_id("submit"))
                    .disabled(!can_submit)
                    .on_press(move |window, cx| parts.submit(window, cx))
                    .child(
                        MainText::new(if submitting { tr("music_uploading") } else { tr("music_create_playlist") })
                            .px(16.)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    ),
            )
            .into_any_element()
    }

    fn submit(&self, window: &mut Window, cx: &mut App) {
        let submitting = is_submitting(cx);
        let form = self.state.read(cx);
        if !form.can_submit(submitting) {
            return;
        }
        let title = form.normalized_title().to_owned();
        let description = form.normalized_description().to_owned();
        let visibility = self.locked_visibility.unwrap_or(form.visibility).raw_value();
        let local_cover_file = form.local_cover_file.clone();

        let window_handle = window.window_handle();
        let is_presented = self.is_presented.clone();
        let on_created = self.on_created.clone();
        let on_success: CreatePlaylistSheetOnCreated = Rc::new(move |playlist: Option<RiseStoredMusicPlaylist>, cx: &mut App| {
            let closing = is_presented.clone();
            cx.defer(move |cx| {
                let _ = window_handle.update(cx, |_, window, cx| closing.set(false, window, cx));
            });
            if let Some(on_created) = &on_created {
                on_created(playlist, cx);
            }
        });
        let error_state = self.state.downgrade();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            let dismissed = error_state.clone();
            let _ = error_state.update(cx, |form, cx| {
                form.submit_error = Some(message.clone());
                cx.notify();
            });
            NativeConfirmAlert::present_notice(
                &tr("save_failed"),
                &message,
                None,
                None,
                move |cx| {
                    let _ = dismissed.update(cx, |form, cx| {
                        form.submit_error = None;
                        cx.notify();
                    });
                },
                cx,
            );
        });
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.create_playlist(
                &title,
                (!description.is_empty()).then_some(description.as_str()),
                visibility,
                local_cover_file,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }
}

fn visibility_items(
    id: &ElementId,
    form: &Entity<CreatePlaylistSheetState>,
    locked_visibility: Option<PlaylistVisibility>,
    theme: &AppTheme,
    cx: &App,
) -> Vec<GroupedBtn> {
    let visibility = form.read(cx).visibility;
    PlaylistVisibility::ALL
        .into_iter()
        .map(|option| {
            let is_selected = visibility == option;
            let is_locked_out = locked_visibility.is_some() && locked_visibility != Some(option);
            let state = form.downgrade();
            GroupedBtn::new(option.raw_value(), option.title())
                .icon(option.icon())
                .icon_background(option.icon_background())
                .icon_foreground(white())
                .subtitle(option.description())
                .subtitle_line_limit(2)
                .disabled(is_locked_out)
                .accessibility_selected(is_selected)
                .show_chevron(false)
                .trailing_slot(VisibilityCheckmark {
                    id: ElementId::NamedChild(
                        Arc::new(id.clone()),
                        SharedString::new_static(option.raw_value()),
                    ),
                    is_selected,
                    color: theme.primary._100,
                })
                .callback(move |_, cx| {
                    if let Some(state) = state.upgrade() {
                        select(&state, option, locked_visibility, cx);
                    }
                })
        })
        .collect()
}

fn select(
    state: &Entity<CreatePlaylistSheetState>,
    option: PlaylistVisibility,
    locked_visibility: Option<PlaylistVisibility>,
    cx: &mut App,
) {
    if state.read(cx).visibility == option || locked_visibility.is_some() {
        return;
    }
    let _ = selection_changed();
    state.update(cx, |form, _| form.visibility = option);
    if let Some(list) = state.read(cx).visibility_list.clone() {
        list.update(cx, |_, cx| cx.notify());
    } else {
        state.update(cx, |_, cx| cx.notify());
    }
}

/// playlist type card of the form as its own view
/// a pick redraws these rows and not the form or the screen under it
struct CreatePlaylistVisibilityListView {
    id: ElementId,
    form: WeakEntity<CreatePlaylistSheetState>,
    locked_visibility: Option<PlaylistVisibility>,
    height: Option<Pixels>,
}

impl Render for CreatePlaylistVisibilityListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(form) = self.form.upgrade() else {
            return div().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        let items = visibility_items(&self.id, &form, self.locked_visibility, &theme, cx);
        let types_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("types"));
        let height_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("types-height"));
        let measured = cx.entity().downgrade();
        let form = self.form.clone();
        div()
            .relative()
            .w_full()
            .child(
                GroupedBtns::new(types_id, items)
                    .title(tr("music_playlist_type"))
                    .corner_radius(CreatePlaylistSheetPresentation::CARD_CORNER_RADIUS),
            )
            .child(measure(height_id, move |bounds, _, cx| {
                let height = bounds.size.height;
                let changed = measured
                    .update(cx, |list, _| {
                        let changed = list.height != Some(height);
                        list.height = Some(height);
                        changed
                    })
                    .unwrap_or(false);
                if changed {
                    let _ = form.update(cx, |_, cx| cx.notify());
                }
            }))
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct VisibilityCheckmark {
    id: ElementId,
    is_selected: bool,
    color: Hsla,
}

const CHECKMARK_SIZE: f32 = 14.;

static NEXT_CHECKMARK_SWAP_KEY: AtomicU64 = AtomicU64::new(3 << 62);

/// fade of one checkmark and the swap key of its newest paint
/// element state so it never notifies the view holding the sheet
struct CheckmarkFade {
    opacity: Animated<f32>,
    swap_key: u64,
}

impl RenderOnce for VisibilityCheckmark {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = if self.is_selected { 1_f32 } else { 0_f32 };
        let reduce_motion = cx.reduce_motion();
        let fade = window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state::<Rc<RefCell<CheckmarkFade>>, _>(global_id, |fade, _| {
                let fade = fade.unwrap_or_else(|| {
                    Rc::new(RefCell::new(CheckmarkFade { opacity: Animated::new(target), swap_key: 0 }))
                });
                (fade.clone(), fade)
            })
        });
        fade.borrow_mut().opacity.set(
            target,
            spring(SELECTION_SPRING_RESPONSE, SELECTION_SPRING_DAMPING),
            RiseClock::monotonic(),
            reduce_motion,
        );
        let color = self.color;
        canvas(|_, _: &mut Window, _: &mut App| (), move |bounds, (), window, _| paint_checkmark(&fade, color, bounds, window))
            .frame_size(CHECKMARK_SIZE, CHECKMARK_SIZE)
    }
}

fn paint_checkmark(fade: &Rc<RefCell<CheckmarkFade>>, color: Hsla, bounds: Bounds<Pixels>, window: &mut Window) {
    let extent = f32::from(bounds.size.width.min(bounds.size.height));
    let Some(path) = checkmark_path(bounds.origin, extent) else {
        return;
    };
    let now = RiseClock::monotonic();
    let swap_key = NEXT_CHECKMARK_SWAP_KEY.fetch_add(1, Ordering::Relaxed);
    let (opacity, running) = {
        let mut fade = fade.borrow_mut();
        fade.swap_key = swap_key;
        (fade.opacity.value(now).clamp(0., 1.), fade.opacity.is_running(now))
    };
    window.paint_swappable_path(path, color.opacity(opacity), swap_key, bounds.origin);
    if running && is_painted_on_screen(bounds, window) {
        CheckmarkFadeSwaps { fade: fade.clone(), color, extent, swap_key, view: window.current_view() }.schedule(window);
    }
}

struct CheckmarkFadeSwaps {
    fade: Rc<RefCell<CheckmarkFade>>,
    color: Hsla,
    extent: f32,
    swap_key: u64,
    view: EntityId,
}

impl CheckmarkFadeSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(self, window: &mut Window, cx: &mut App) {
        if self.fade.borrow().swap_key != self.swap_key {
            return;
        }
        let now = RiseClock::monotonic();
        let (opacity, running) = {
            let fade = self.fade.borrow();
            (fade.opacity.value(now).clamp(0., 1.), fade.opacity.is_running(now))
        };
        let Some(path) = checkmark_path(point(px(0.), px(0.)), self.extent) else {
            return;
        };
        let color: Background = self.color.opacity(opacity).into();
        if window.swap_paths(vec![(self.swap_key, path, color)]) {
            if running {
                self.schedule(window);
            }
            return;
        }
        if running {
            cx.notify(self.view);
        }
    }
}

/// checkmark as two round capped strokes in one fill so a swap moves it whole
/// laid out in a square of side extent from origin
fn checkmark_path(origin: Point<Pixels>, extent: f32) -> Option<Path<Pixels>> {
    if extent <= 0. {
        return None;
    }
    let at = |x: f32, y: f32| point(origin.x + px(extent * x), origin.y + px(extent * y));
    let line_width = (extent * 0.14).max(1.);
    let start = at(0.1, 0.55);
    let corner = at(0.38, 0.84);
    let end = at(0.9, 0.16);
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero()));
    builder.add_polygon(&capsule(start, corner, line_width), true);
    builder.add_polygon(&capsule(corner, end, line_width), true);
    builder.build().ok()
}

/// outline of a round capped stroke from one point to another
/// wound the same for every direction so two of them fill as one shape
fn capsule(from: Point<Pixels>, to: Point<Pixels>, line_width: f32) -> Vec<Point<Pixels>> {
    let direction = f32::from(to.y - from.y).atan2(f32::from(to.x - from.x));
    let half = line_width * 0.5;
    const CAP_STEPS: usize = 8;
    let around = |center: Point<Pixels>, angle: f32| point(center.x + px(half * angle.cos()), center.y + px(half * angle.sin()));
    let mut points = Vec::with_capacity(2 * CAP_STEPS + 2);
    for step in 0..=CAP_STEPS {
        points.push(around(to, direction - PI * 0.5 + PI * step as f32 / CAP_STEPS as f32));
    }
    for step in 0..=CAP_STEPS {
        points.push(around(from, direction + PI * 0.5 + PI * step as f32 / CAP_STEPS as f32));
    }
    points
}

/// presentation of the create playlist sheet
/// no keyboard members since nothing covers the sheet on desktop
pub enum CreatePlaylistSheetPresentation {}

impl CreatePlaylistSheetPresentation {
    pub const MINIMUM_FRACTION: f32 = 0.42;

    pub const KEYBOARD_FOOTER_SPACING: f32 = 12.;

    pub const FOOTER_HORIZONTAL_PADDING: f32 = GroupedCardMetrics::HORIZONTAL_PADDING;
    pub const FOOTER_TOP_PADDING: f32 = 10.;

    pub const FOOTER_CHROME: f32 = 44.;

    pub const CARD_CORNER_RADIUS: f32 = 30.;
}
