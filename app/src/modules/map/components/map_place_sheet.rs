use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Context, ElementId, Entity, FontWeight, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, Styled, Subscription, Window, div, px, transparent_black,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::slider::slider;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_ui::switch_ui::SwitchUi;
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetFloatingHeader;
use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyph;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::core::ui::settings_field_row::SettingsFieldRow;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::map::stores::map_actions::map_place_types::{MapPlaceSheetSession, MapPlaceSheetSessionPage};
use crate::modules::map::stores::map_actions::map_tracks_types::{MapPlace, MapPlaceEmoji, MapPlacePolicy};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

type SheetCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// mounts the place sheet of the session the store holds
/// keeps the last one mounted during dismissal so it slides away
pub struct MapPlaceSheetHost {
    interactions: Entity<MapInteractionsStore>,
    retained: Option<MapPlaceSheetSession>,
    _interactions_observation: Subscription,
}

impl MapPlaceSheetHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let interactions = MapInteractionsStore::shared(cx);
        let retained = interactions.read(cx).place_sheet().cloned();
        let interactions_observation = cx.observe(&interactions, |this, interactions, cx| {
            if let Some(session) = interactions.read(cx).place_sheet() {
                this.retained = Some(session.clone());
            }
            cx.notify();
        });
        Self { interactions, retained, _interactions_observation: interactions_observation }
    }
}

impl Render for MapPlaceSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.interactions.read(cx).place_sheet().cloned().or_else(|| self.retained.clone());
        let Some(session) = shown.filter(|session| session.is_current()) else {
            return div().into_any_element();
        };
        let id = session.id;
        let reader = self.interactions.clone();
        let writer = self.interactions.clone();
        let is_presented = Binding::new(
            move |cx| reader.read(cx).place_sheet().is_some_and(|sheet| sheet.id == id),
            move |presented, _, cx| {
                if presented {
                    return;
                }
                writer.update(cx, |interactions, cx| {
                    if interactions.place_sheet().is_some_and(|sheet| sheet.id == id) {
                        interactions.dismiss_place(cx);
                    }
                });
            },
        );
        let host = cx.entity().downgrade();
        let on_did_dismiss: SheetCallback = Rc::new(move |_, cx| {
            let _ = host.update(cx, |this, cx| {
                let closed = this.interactions.read(cx).place_sheet().is_none();
                if closed && this.retained.as_ref().is_some_and(|retained| retained.id == id) {
                    this.retained = None;
                    cx.notify();
                }
            });
        });
        div().child(MapPlaceSheet::new(session, is_presented, on_did_dismiss)).into_any_element()
    }
}

#[derive(IntoElement)]
struct MapPlaceSheet {
    session: MapPlaceSheetSession,
    is_presented: Binding<bool>,
    on_did_dismiss: SheetCallback,
}

impl MapPlaceSheet {
    const PICKER_HEIGHT: f32 = 300.0;

    fn new(session: MapPlaceSheetSession, is_presented: Binding<bool>, on_did_dismiss: SheetCallback) -> Self {
        Self { session, is_presented, on_did_dismiss }
    }
}

/// what every slot of the sheet captures
/// frame session, the store owning the live one and the sheet's own state
#[derive(Clone)]
struct MapPlaceSheetParts {
    interactions: Entity<MapInteractionsStore>,
    session: MapPlaceSheetSession,
    key: SharedString,
    picker_open: Entity<bool>,
    snap: Entity<f32>,
    picker_presence: Entity<Presence<()>>,
    reduce_motion: bool,
    is_presented: Binding<bool>,
    on_did_dismiss: SheetCallback,
}

impl RenderOnce for MapPlaceSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let reduce_motion = reduce_motion(cx);
        let key = SharedString::from(format!("map-place-sheet-{}", self.session.id.uuid_string()));
        let picker_transition = if reduce_motion {
            Transition::opacity()
        } else {
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity())
        };
        let parts = MapPlaceSheetParts {
            interactions: MapInteractionsStore::shared(cx),
            picker_open: window.use_keyed_state(MapPlaceSheetParts::id(&key, "picker-open"), cx, |_, _| false),
            snap: window.use_keyed_state(MapPlaceSheetParts::id(&key, "snap"), cx, |_, _| 0.46_f32),
            picker_presence: window.use_keyed_state(MapPlaceSheetParts::id(&key, "picker-presence"), cx, |_, _| {
                Presence::new(None).with_transition(picker_transition)
            }),
            session: self.session,
            key,
            reduce_motion,
            is_presented: self.is_presented,
            on_did_dismiss: self.on_did_dismiss,
        };

        let page = parts.session.page;
        let page_parts = parts.clone();
        on_change(parts.id_for("page"), page, false, window, cx, move |_, page, _, cx| {
            page_parts.picker_open.update(cx, |open, cx| {
                *open = false;
                cx.notify();
            });
            page_parts.set_snap(if *page == MapPlaceSheetSessionPage::Edit { 0.76 } else { 0.46 }, cx);
        });
        on_change(parts.id_for("editor-open"), page == MapPlaceSheetSessionPage::Edit, true, window, cx, |_, editing, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapPlaceEditorOpen, *editing, cx);
        });
        let named = !trimming_whitespaces_and_newlines(&parts.session.draft.name).is_empty();
        on_change(parts.id_for("named"), named, true, window, cx, |_, named, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapPlaceNamed, *named, cx);
        });

        let is_saving = parts.session.is_saving;
        let content_parts = parts.clone();
        let footer_parts = parts.clone();
        let did_dismiss_parts = parts.clone();
        let refused_parts = parts.clone();
        BottomSheetUi::new(
            parts.id_for("sheet"),
            parts.is_presented.clone(),
            move |window, cx| content_parts.content(window, cx),
            move |window, cx| footer_parts.footer(window, cx),
        )
        .snap_fractions(vec![0.46, 0.76, 0.94])
        .fits_content(true)
        .full_page_allows_swipe_dismiss(!is_saving)
        .dismisses_on_backdrop_tap(!is_saving)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .full_page_hides_footer(page != MapPlaceSheetSessionPage::Edit)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(false))
        .footer_horizontal_padding(Some(16.0))
        .footer_top_padding(Some(10.0))
        .content_scrollable(true)
        .full_page_interaction_locked(is_saving)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .on_did_dismiss(move |window, cx| (did_dismiss_parts.on_did_dismiss)(window, cx))
        .on_presentation_refused(move |window, cx| {
            refused_parts.is_presented.set(false, window, cx);
            (refused_parts.on_did_dismiss)(window, cx);
        })
        .on_disappear(parts.id_for("lifecycle"), |_, cx| {
            GuideFacts::set(RiseGuideSignal::MapPlaceEditorOpen, false, cx);
            GuideFacts::set(RiseGuideSignal::MapPlaceNamed, false, cx);
        })
    }
}

impl MapPlaceSheetParts {
    fn id(key: &SharedString, suffix: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!("{key}-{suffix}")))
    }

    fn id_for(&self, suffix: &str) -> ElementId {
        Self::id(&self.key, suffix)
    }

    /// store session while its still this sheet's, else the frame copy
    /// the sheet is closing and edits land nowhere
    fn live(&self, cx: &App) -> MapPlaceSheetSession {
        let id = self.session.id;
        self.interactions
            .read(cx)
            .place_sheet()
            .filter(|session| session.id == id)
            .cloned()
            .unwrap_or_else(|| self.session.clone())
    }

    /// edits a copy of the store session and writes it back
    /// only while its still this sheet's
    fn edit(&self, cx: &mut App, change: impl FnOnce(&mut MapPlaceSheetSession)) {
        let id = self.session.id;
        self.interactions.update(cx, |interactions, cx| {
            let Some(mut session) = interactions.place_sheet().filter(|session| session.id == id).cloned() else {
                return;
            };
            change(&mut session);
            interactions.set_place_sheet(Some(session), cx);
        });
    }

    fn draft_binding<T: 'static>(
        &self,
        get: impl Fn(&MapPlace) -> T + 'static,
        set: impl Fn(&mut MapPlace, T) + 'static,
    ) -> Binding<T> {
        let reader = self.clone();
        let writer = self.clone();
        let set = Rc::new(set);
        Binding::new(
            move |cx| get(&reader.live(cx).draft),
            move |value, _, cx| {
                let set = set.clone();
                writer.edit(cx, move |session| set(&mut session.draft, value));
            },
        )
    }

    fn picker_open(&self, cx: &App) -> bool {
        *self.picker_open.read(cx)
    }

    fn set_snap(&self, value: f32, cx: &mut App) {
        self.snap.update(cx, |snap, cx| {
            *snap = value;
            cx.notify();
        });
    }

    fn set_picker_open(&self, open: bool, cx: &mut App) {
        self.picker_open.update(cx, |picker_open, cx| {
            *picker_open = open;
            cx.notify();
        });
    }

    fn navigate(&self, page: MapPlaceSheetSessionPage, window: &mut Window, cx: &mut App) {
        Self::hide_keyboard(window, cx);
        self.set_picker_open(false, cx);
        self.edit(cx, |session| {
            session.error_text = None;
            if page == MapPlaceSheetSessionPage::Details
                && let Some(saved) = session.saved.clone()
            {
                session.draft = saved;
            }
            session.page = page;
        });
        self.set_snap(if page == MapPlaceSheetSessionPage::Edit { 0.76 } else { 0.46 }, cx);
    }

    /// drops text field focus, no on screen keyboard to hide on desktop
    fn hide_keyboard(window: &mut Window, cx: &mut App) {
        ScreenFocus::release_text_field(window, cx);
    }

    fn close_picker(&self, cx: &mut App) {
        self.set_picker_open(false, cx);
        self.set_snap(0.76, cx);
    }

    fn pick_emoji(&self, value: String, custom: Option<MapPlaceEmoji>, cx: &mut App) {
        self.edit(cx, move |session| {
            session.draft.emoji = value;
            session.draft.custom_emoji = custom;
        });
    }

    fn content(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let session = &self.session;
        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(20.0))
            .pt(px(14.0))
            .pb(px(24.0))
            .child(self.header(&theme));
        column = match session.page {
            MapPlaceSheetSessionPage::Actions => {
                let parts = self.clone();
                column
                    .child(GroupedBtns::new(
                        self.id_for("actions"),
                        vec![
                            GroupedBtn::new("mark", tr("map_place_mark"))
                                .icon("mappin.and.ellipse")
                                .icon_background(SettingsIconPalette::profile())
                                .guide_anchor(RiseGuideAnchor::MapPlaceMark)
                                .callback(move |window, cx| parts.navigate(MapPlaceSheetSessionPage::Edit, window, cx)),
                        ],
                    ))
                    .child(Self::privacy_note(&theme))
            }
            MapPlaceSheetSessionPage::Details => {
                let parts = self.clone();
                column
                    .child(GroupedBtns::new(
                        self.id_for("details"),
                        vec![
                            GroupedBtn::new("settings", tr("map_place_settings"))
                                .icon("gearshape.fill")
                                .icon_background(SettingsIconPalette::profile())
                                .guide_anchor(RiseGuideAnchor::MapPlaceSettings)
                                .callback(move |window, cx| parts.navigate(MapPlaceSheetSessionPage::Edit, window, cx)),
                        ],
                    ))
                    .child(Self::privacy_note(&theme))
            }
            MapPlaceSheetSessionPage::Edit => column.child(self.editor(&theme)),
        };
        if let Some(error) = &session.error_text {
            column = column.child(
                div().w_full().px(px(20.0)).child(
                    MainText::new(error.clone()).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary),
                ),
            );
        }
        let page_key = match session.page {
            MapPlaceSheetSessionPage::Actions => "scroll-actions",
            MapPlaceSheetSessionPage::Details => "scroll-details",
            MapPlaceSheetSessionPage::Edit => "scroll-edit",
        };
        scroll_view(self.id_for(page_key)).shows_indicators(false).size_full().child(column).into_any_element()
    }

    fn header(&self, theme: &AppTheme) -> AnyElement {
        let session = &self.session;
        if session.page == MapPlaceSheetSessionPage::Edit {
            let parts = self.clone();
            let back_to = if session.is_creating { MapPlaceSheetSessionPage::Actions } else { MapPlaceSheetSessionPage::Details };
            let title = tr(if session.is_creating { "map_place_mark" } else { "map_place_settings" });
            return div()
                .relative()
                .w_full()
                .child(
                    SheetFloatingHeader::new(self.id_for("edit-header"), title, move |window, cx| {
                        parts.navigate(back_to, window, cx)
                    })
                    .shows_chrome(false)
                    .leading_system_image("chevron.left"),
                )
                .when(session.is_saving, |wrapper| wrapper.child(div().absolute().inset_0().occlude()))
                .into_any_element();
        }
        let mut stack = div().w_full().flex().flex_col().items_center().gap(px(12.0)).child(
            div()
                .h(px(78.0))
                .flex()
                .items_center()
                .justify_center()
                .child(MapPlaceGlyph::new(session.draft.clone(), 58.0).is_playing(!self.reduce_motion)),
        );
        if session.page == MapPlaceSheetSessionPage::Actions || !session.draft.name.is_empty() {
            let title = if session.page == MapPlaceSheetSessionPage::Actions {
                tr("map_place_location")
            } else {
                session.draft.name.clone()
            };
            stack = stack.child(
                div()
                    .w_full()
                    .px(px(24.0))
                    .child(MainText::new(title).px(22.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD)),
            );
        }
        if session.page == MapPlaceSheetSessionPage::Details && !session.draft.description.is_empty() {
            stack = stack.child(div().w_full().px(px(24.0)).child(
                MainText::new(session.draft.description.clone())
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary),
            ));
        }
        stack.into_any_element()
    }

    fn editor(&self, theme: &AppTheme) -> AnyElement {
        let session = &self.session;
        let mut quick = div().w_full().px(px(12.0)).flex().items_center();
        for (index, emoji) in MapPlacePolicy::QUICK_EMOJIS.iter().enumerate() {
            let parts = self.clone();
            let selected = session.draft.emoji == *emoji;
            quick = quick.child(
                div()
                    .id(self.id_for(&format!("quick-{index}")))
                    .flex_1()
                    .h(px(46.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(14.0))
                    .when(selected, |button| button.bg(theme.primary._100.opacity(0.14)))
                    .text_size(px(28.0))
                    .child(*emoji)
                    .on_click(move |_, _, cx| parts.pick_emoji((*emoji).to_owned(), None, cx)),
            );
        }
        let toggle_parts = self.clone();
        quick = quick.child(
            div()
                .id(self.id_for("picker-toggle"))
                .flex_1()
                .h(px(46.0))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("face.smiling", px(24.0), theme.text.primary, false))
                .on_click(move |_, window, cx| {
                    Self::hide_keyboard(window, cx);
                    let open = !toggle_parts.picker_open(cx);
                    toggle_parts.set_picker_open(open, cx);
                    toggle_parts.set_snap(if open { 0.94 } else { 0.76 }, cx);
                }),
        );

        let name_rule = MapPlacePolicy::name_rule();
        let description_rule = MapPlacePolicy::description_rule();
        let name = name_rule.binding(self.draft_binding(|draft| draft.name.clone(), |draft, value| draft.name = value));
        let description = description_rule
            .binding(self.draft_binding(|draft| draft.description.clone(), |draft, value| draft.description = value));
        let fields = GroupedBtns::new(
            self.id_for("fields"),
            vec![
                GroupedBtn::new("name", "").content_slot(
                    SettingsFieldRow::new(
                        self.id_for("name"),
                        "textformat",
                        ChatTileColor::blue(),
                        tr("map_place_name_optional"),
                        name,
                    )
                    .error_message(name_rule.error_text(&session.draft.name))
                    .rule(name_rule),
                ),
                GroupedBtn::new("description", "").content_slot(
                    SettingsFieldRow::new(
                        self.id_for("description"),
                        "text.alignleft",
                        ChatTileColor::teal(),
                        tr("map_place_description_optional"),
                        description,
                    )
                    .error_message(description_rule.error_text(&session.draft.description))
                    .rule(description_rule)
                    .axis(gpui::Axis::Vertical),
                ),
            ],
        );
        let closing_parts = self.clone();
        let fields = div()
            .w_full()
            .capture_any_mouse_down(move |_, _, cx| {
                if closing_parts.picker_open(cx) {
                    closing_parts.close_picker(cx);
                }
            })
            .child(fields.guide_anchor(RiseGuideAnchor::MapPlaceName));

        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(22.0))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(14.0))
                    .child(
                        div()
                            .h(px(76.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(MapPlaceGlyph::new(session.draft.clone(), 54.0).is_playing(!self.reduce_motion)),
                    )
                    .child(quick),
            )
            .child(fields);
        if !session.is_creating {
            let parts = self.clone();
            column = column.child(self.recording_settings(theme)).child(GroupedBtns::new(
                self.id_for("delete-group"),
                vec![
                    GroupedBtn::new("delete", tr("map_place_delete"))
                        .icon("trash")
                        .icon_background(SettingsIconPalette::logout())
                        .destructive(true)
                        .show_chevron(false)
                        .callback(move |_, cx| parts.confirm_delete(cx)),
                ],
            ));
        }
        column = column.child(Self::privacy_note(theme));
        Self::disabled(column, session.is_saving)
    }

    fn confirm_delete(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("map_place_delete_title"),
            &tr("map_place_delete_message"),
            Some(tr("map_place_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                let session = parts.live(cx);
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.delete_place(&session, cx));
            },
            cx,
        );
    }

    fn recording_settings(&self, theme: &AppTheme) -> AnyElement {
        let draft = &self.session.draft;
        let toggle_parts = self.clone();
        let record = self.draft_binding(|draft| draft.record_tracks, |draft, value| draft.record_tracks = value);
        let radius = self.draft_binding(
            |draft| draft.track_exclusion_radius_meters as f64,
            |draft, value: f64| draft.track_exclusion_radius_meters = MapPlacePolicy::radius(value.round() as i64),
        );
        let radius_text = tr_args("map_home_track_radius_value", &[FormatArg::Int(draft.track_exclusion_radius_meters)]);
        let radius_row = div()
            .w_full()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .when(draft.record_tracks, |row| row.opacity(0.45))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(MainText::new(tr("map_place_radius")).px(15.0))
                    .child(div().flex_1())
                    .child(
                        MainText::new(radius_text)
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
            .child(
                slider(
                    self.id_for("radius-slider"),
                    radius,
                    MapPlacePolicy::minimum_radius() as f64..=MapPlacePolicy::maximum_radius() as f64,
                )
                .step(1.0)
                .tint(theme.primary._100),
            );
        GroupedBtns::new(
            self.id_for("recording"),
            vec![
                GroupedBtn::new("record", tr("map_place_record_tracks"))
                    .icon("point.topleft.down.to.point.bottomright.curvepath")
                    .icon_background(SettingsIconPalette::privacy_goals())
                    .trailing_slot(SwitchUi::new(self.id_for("record-switch"), record))
                    .callback(move |_, cx| toggle_parts.edit(cx, |session| session.draft.record_tracks = !session.draft.record_tracks)),
                GroupedBtn::new("radius", "").content_slot(Self::disabled(radius_row, draft.record_tracks)),
            ],
        )
        .end_group_title(tr("map_place_recording_hint"))
        .guide_anchor(RiseGuideAnchor::MapPlaceRecording)
        .into_any_element()
    }

    fn privacy_note(theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .children(IconUi::sized("lock.fill", px(12.0), theme.text.secondary, true))
            .child(MainText::new(tr("map_place_private")).px(12.0).color(theme.text.secondary))
            .into_any_element()
    }

    /// content stays drawn as is but stops taking the pointer
    fn disabled(content: impl IntoElement, disabled: bool) -> AnyElement {
        div()
            .relative()
            .w_full()
            .child(content)
            .when(disabled, |wrapper| wrapper.child(div().absolute().inset_0().occlude()))
            .into_any_element()
    }

    fn footer(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.session.page != MapPlaceSheetSessionPage::Edit {
            return div().into_any_element();
        }
        let theme = rise_ui::theme(cx).clone();
        let session = &self.session;
        let save_parts = self.clone();
        let mut label = div().w_full().h(px(52.0)).flex().items_center().justify_center().gap(px(8.0));
        if session.is_saving {
            label = label.child(progress_view().tint(theme.content_on(theme.primary._100)));
        }
        label = label.child(MainText::new(tr("save")).px(17.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)));
        let save = LiquidGlassButton::new(self.id_for("save"), LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(0.0)
            .tint(Some(theme.primary._100))
            .disabled(!session.is_valid() || session.is_saving)
            .accessibility_label(Some(tr("save")))
            .on_press(move |_, cx| {
                let session = save_parts.live(cx);
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.save_place(&session, cx));
            })
            .child(label)
            .guide_anchor(RiseGuideAnchor::MapPlaceSave);
        div()
            .w_full()
            .pb(px(8.0))
            .flex()
            .flex_col()
            .child(self.emoji_picker(&theme, window, cx))
            .child(save)
            .into_any_element()
    }

    fn emoji_picker(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let open = self.picker_open(cx);
        let now = RiseClock::monotonic();
        let spec = (!self.reduce_motion).then(|| spring(BottomSheetMetrics::SNAP_RESPONSE, BottomSheetMetrics::SNAP_DAMPING));
        self.picker_presence.update(cx, |presence, cx| presence.set_in(open.then_some(()), spec, now, cx));

        let pick_parts = self.clone();
        let custom_parts = self.clone();
        let picker = div()
            .w_full()
            .h(px(MapPlaceSheet::PICKER_HEIGHT))
            .bg(theme.bg._100)
            .rounded(px(20.0))
            .overflow_hidden()
            .child(
                EmojiPickerUi::new(self.id_for("emoji-picker"), move |value: String, window: &mut Window, cx: &mut App| {
                    Self::hide_keyboard(window, cx);
                    pick_parts.pick_emoji(value, None, cx);
                    pick_parts.close_picker(cx);
                })
                .chrome(false)
                .shows_custom_emoji(true)
                .on_pick_custom(move |item: StickerItem, window: &mut Window, cx: &mut App| {
                    let Some(value) = EmojiPickerCatalog::custom_emoji_ref(&item.id) else {
                        return;
                    };
                    Self::hide_keyboard(window, cx);
                    custom_parts.pick_emoji(value, Some(MapPlaceEmoji::new(&item)), cx);
                    custom_parts.close_picker(cx);
                }),
            );
        let revealed = self.picker_presence.read(cx).render_transitioned(now, window, |_| picker);
        div()
            .w_full()
            .h(px(if open { MapPlaceSheet::PICKER_HEIGHT } else { 0.0 }))
            .flex()
            .flex_col()
            .justify_end()
            .overflow_hidden()
            .when(open, |container| container.mb(px(12.0)))
            .children(revealed)
            .when(!open || self.session.is_saving, |container| container.child(div().absolute().inset_0().occlude()))
            .relative()
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct MapPlaceGlyph {
    pub place: MapPlace,
    pub size: f32,
    pub is_playing: bool,
}

impl MapPlaceGlyph {
    pub fn new(place: MapPlace, size: f32) -> Self {
        Self { place, size, is_playing: false }
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }
}

impl RenderOnce for MapPlaceGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match &self.place.custom_emoji {
            Some(custom) => CustomEmojiGlyph::new(custom.item(), self.size)
                .maximum_aspect(1.0)
                .is_playing(self.is_playing)
                .into_any_element(),
            None => ReactionEmojiGlyph::new(self.place.emoji.clone(), self.size)
                .is_playing(self.is_playing)
                .into_any_element(),
        }
    }
}
