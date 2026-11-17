use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, RenderOnce, SharedString, Subscription, TextAlign,
    Window,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{Locale, tr};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderMetrics};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;
use crate::modules::media_picker::shared::media_picker_recent_files::{
    MediaPickerRecentFile, MediaPickerRecentFiles, MediaPickerRecentKind,
};
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;
use crate::modules::media_picker::ui::media_picker_session::MediaPickerAudioDraft;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

/// no matched geometry here so the search field grows out of the magnifier slot
/// the close button slides across the header, one spring over the measured header width
enum MediaPickerSourceSearchHero {}

impl MediaPickerSourceSearchHero {
    const FIELD_ID: &'static str = "media-picker-source-search-field";
    const CLOSE_ID: &'static str = "media-picker-source-search-close";
    const SPACING: f32 = 10.0;

    fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

/// state the view keeps between frames
struct MediaPickerDocumentSourcePageState {
    search_motion: Animated<f32>,
    header_width: f32,
    _recents_observer: Subscription,
}

impl MediaPickerDocumentSourcePageState {
    fn new(recents: &Entity<MediaPickerRecentFiles>, is_searching: bool, cx: &mut Context<Self>) -> Self {
        Self {
            search_motion: Animated::new(if is_searching { 1.0 } else { 0.0 }),
            header_width: 0.0,
            _recents_observer: cx.observe(recents, |_, _, cx| cx.notify()),
        }
    }
}

#[derive(IntoElement)]
pub struct MediaPickerDocumentSourcePage {
    id: ElementId,
    pub kind: MediaPickerRecentKind,
    pub on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_choose: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_recent: Rc<dyn Fn(&MediaPickerRecentFile, &mut Window, &mut App)>,
    pub allowed_file_extensions: Option<HashSet<String>>,
    pub query: Binding<String>,
    pub is_searching: Binding<bool>,
}

impl MediaPickerDocumentSourcePage {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        kind: MediaPickerRecentKind,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_choose: impl Fn(&mut Window, &mut App) + 'static,
        on_recent: impl Fn(&MediaPickerRecentFile, &mut Window, &mut App) + 'static,
        allowed_file_extensions: Option<HashSet<String>>,
        query: Binding<String>,
        is_searching: Binding<bool>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            on_cancel: Rc::new(on_cancel),
            on_choose: Rc::new(on_choose),
            on_recent: Rc::new(on_recent),
            allowed_file_extensions,
            query,
            is_searching,
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn is_allowed_recent(&self, entry: &MediaPickerRecentFile) -> bool {
        let Some(allowed_file_extensions) = &self.allowed_file_extensions else {
            return true;
        };
        if self.kind != MediaPickerRecentKind::File {
            return true;
        }
        let ext = entry
            .local_url()
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        allowed_file_extensions.contains(&ext)
    }

    fn header(
        &self,
        theme: &AppTheme,
        state: &Entity<MediaPickerDocumentSourcePageState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let is_searching = self.is_searching.get(cx);
        let progress = state.read(cx).search_motion.frame_value(RiseClock::monotonic(), window);
        let width = state.read(cx).header_width;
        let slot = PageHeaderMetrics::TOOLBAR_BUTTON_SIZE;
        let travel = (width - slot).max(0.0);
        let field_left = travel * (1.0 - progress);
        let field_width = slot + (travel - MediaPickerSourceSearchHero::SPACING - slot) * progress;
        let close_left = travel * progress;

        let title = if self.kind == MediaPickerRecentKind::Audio {
            tr("media_picker_audio_title")
        } else {
            tr("media_picker_files_title")
        };
        let title_layer = (progress < 1.0).then(|| {
            div().absolute().inset_0().flex().items_center().justify_center().opacity(1.0 - progress).child(
                MainText::new(title)
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
        });

        let field_layer = (is_searching || progress > 0.0).then(|| {
            div()
                .absolute()
                .top_0()
                .left(px(field_left))
                .w(px(field_width.max(slot)))
                .h(px(slot))
                .flex()
                .items_center()
                .opacity(progress)
                .child(
                    div().w_full().min_w_0().child(
                        SearchFieldUi::new(
                            (self.id.clone(), MediaPickerSourceSearchHero::FIELD_ID),
                            self.query.clone(),
                        )
                        .placeholder(Some(SharedString::from(tr("media_picker_search_files"))))
                        .uses_inline_text_field(true)
                        .auto_focus(true),
                    ),
                )
        });

        let magnifier_layer = (!is_searching).then(|| {
            let open = self.is_searching.clone();
            div().absolute().top_0().left(px(field_left)).opacity(1.0 - progress).child(
                MediaPickerHeaderIconButton::new(self.child_id("search-open"), "magnifyingglass", move |window, cx| {
                    open.set(true, window, cx);
                })
                .font_size(16.0),
            )
        });

        let (close_query, close_searching, on_cancel) =
            (self.query.clone(), self.is_searching.clone(), self.on_cancel.clone());
        let close_layer = div().absolute().top_0().left(px(close_left)).child(
            MediaPickerHeaderIconButton::new(
                (self.id.clone(), MediaPickerSourceSearchHero::CLOSE_ID),
                "xmark",
                move |window, cx| {
                    if close_searching.get(cx) {
                        close_query.set(String::new(), window, cx);
                        close_searching.set(false, window, cx);
                    } else {
                        on_cancel(window, cx);
                    }
                },
            )
            .font_size(15.0),
        );

        let width_state = state.clone();
        let geometry = measure(self.child_id("header-geometry"), move |bounds, _, cx| {
            width_state.update(cx, |state, cx| {
                state.header_width = f32::from(bounds.size.width);
                cx.notify();
            });
        })
        .absolute()
        .size_full();

        div().w_full().px(px(14.0)).py(px(10.0)).bg(theme.bg._100).child(
            div()
                .relative()
                .w_full()
                .h(px(slot))
                .child(geometry)
                .children(title_layer)
                .children(field_layer)
                .children(magnifier_layer)
                .child(close_layer),
        )
    }

    fn choose_button(&self, theme: &AppTheme) -> GroupedBtn {
        let on_choose = self.on_choose.clone();
        let is_audio = self.kind == MediaPickerRecentKind::Audio;
        let text = if is_audio { tr("media_picker_choose_audio") } else { tr("media_picker_choose_file") };
        let button = GroupedBtn::new("choose", text)
            .icon(if is_audio { "waveform" } else { "folder.fill" })
            .icon_background(theme.primary._100)
            .callback(move |window, cx| on_choose(window, cx));
        if is_audio { button.subtitle(tr("media_picker_mp3_only")) } else { button }
    }

    fn recent_button(&self, entry: &MediaPickerRecentFile, theme: &AppTheme, green: Hsla) -> GroupedBtn {
        let is_audio = self.kind == MediaPickerRecentKind::Audio;
        let subtitle = entry.artist.clone().unwrap_or_else(|| {
            RiseFormat::date_time_label(
                &Date::from_time_interval_since_1970(entry.last_used_at),
                &Date::now(),
                &Calendar::current(),
                Locale::app_locale().identifier(),
            )
        });
        let (on_recent, picked) = (self.on_recent.clone(), entry.clone());
        GroupedBtn::new(entry.id.clone(), entry.title.clone().unwrap_or_else(|| entry.display_name.clone()))
            .icon(if is_audio { "music.note" } else { "doc.fill" })
            .icon_background(if is_audio { green.opacity(0.85) } else { theme.primary._100 })
            .subtitle(subtitle)
            .text_line_limit(2)
            .subtitle_line_limit(1)
            .callback(move |window, cx| on_recent(&picked, window, cx))
    }

    fn empty_state(&self, theme: &AppTheme, query_is_empty: bool) -> impl IntoElement {
        let symbol = if self.kind == MediaPickerRecentKind::Audio { "music.note.list" } else { "clock.arrow.circlepath" };
        let message = if query_is_empty { tr("media_picker_no_recent_files") } else { tr("media_picker_search_empty") };
        vstack(10.0)
            .w_full()
            .py(px(34.0))
            .children(IconUi::sized(symbol, px(34.0), theme.text.secondary, false))
            .child(MainText::new(message).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary))
    }
}

impl RenderOnce for MediaPickerDocumentSourcePage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let appearance = theme_store.read(cx).resolved_appearance();
        let recents = MediaPickerRecentFiles::shared(cx);
        let is_searching = self.is_searching.get(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| {
            MediaPickerDocumentSourcePageState::new(&recents, is_searching, cx)
        });

        let motion_state = state.clone();
        on_change(self.child_id("searching"), is_searching, false, window, cx, move |_, is_searching, _, cx| {
            let target = if *is_searching { 1.0 } else { 0.0 };
            motion_state.update(cx, |state, cx| {
                state.search_motion.set_in(
                    target,
                    MediaPickerSourceSearchHero::animation(),
                    RiseClock::monotonic(),
                    cx,
                );
                cx.notify();
            });
        });

        let query = self.query.get(cx);
        let kind = self.kind;
        let recent: Vec<MediaPickerRecentFile> = recents
            .update(cx, |recents, cx| recents.files(&query, kind, cx))
            .into_iter()
            .filter(|entry| self.is_allowed_recent(entry))
            .collect();

        let green = UIColor::system_green().resolved_color(appearance);
        let recent_section = if recent.is_empty() {
            self.empty_state(&theme, query.is_empty()).into_any_element()
        } else {
            let rows = recent.iter().map(|entry| self.recent_button(entry, &theme, green)).collect();
            GroupedBtns::new(self.child_id("recent"), rows).title(tr("media_picker_recently_sent")).into_any_element()
        };

        let header = self.header(&theme, &state, window, cx);
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(header)
            .child(
                div().flex_1().min_h_0().w_full().child(
                    scroll_view(self.child_id("scroll")).child(
                        vstack(18.0)
                            .w_full()
                            .pt(px(10.0))
                            .pb(px(108.0))
                            .child(GroupedBtns::new(self.child_id("choose"), vec![self.choose_button(&theme)]))
                            .child(recent_section),
                    ),
                ),
            )
    }
}

/// state pair of the form
struct MediaPickerAudioMetadataEditorState {
    draft: MediaPickerAudioDraft,
    show_cover_picker: bool,
}

#[derive(IntoElement)]
pub struct MediaPickerAudioMetadataEditor {
    id: ElementId,
    draft: MediaPickerAudioDraft,
    pub theme_store: Entity<ThemeStore>,
    pub on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_complete: Rc<dyn Fn(MediaPickerAudioDraft, &mut Window, &mut App)>,
}

impl MediaPickerAudioMetadataEditor {
    pub fn new(
        id: impl Into<ElementId>,
        draft: MediaPickerAudioDraft,
        theme_store: Entity<ThemeStore>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_complete: impl Fn(MediaPickerAudioDraft, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            draft,
            theme_store,
            on_cancel: Rc::new(on_cancel),
            on_complete: Rc::new(on_complete),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn can_save(draft: &MediaPickerAudioDraft) -> bool {
        !trimming_whitespaces_and_newlines(&draft.title).is_empty()
            && !trimming_whitespaces_and_newlines(&draft.artist).is_empty()
            && draft.duration > 0.0
    }

    fn cover(&self, theme: &AppTheme, state: &Entity<MediaPickerAudioMetadataEditorState>, cx: &App) -> impl IntoElement {
        const COVER_RADIUS: f32 = 28.0;
        let artwork = match &state.read(cx).draft.cover_url {
            Some(cover_url) => CleverImageUi::new(SharedString::from(cover_url.absolute_string().to_owned()))
                .id(self.child_id("cover-image"))
                .content_mode(ContentMode::Fill)
                .corner_radius(COVER_RADIUS)
                .into_any_element(),
            None => div()
                .children(IconUi::sized("music.note", px(42.0), theme.primary._100, false))
                .into_any_element(),
        };
        let badge = div()
            .absolute()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .size(px(36.0))
            .rounded_full()
            .bg(theme.primary._100)
            .children(IconUi::sized("camera.fill", px(15.0), theme.content_on(theme.primary._100), true));
        let open = state.clone();
        div().id(self.child_id("cover-tooltip")).tooltip_label(tr("music_local_upload_cover_optional")).child(
            plain(self.child_id("cover-button"))
                .on_press(move |_, cx| {
                    open.update(cx, |state, cx| {
                        state.show_cover_picker = true;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .relative()
                        .size(px(132.0))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(theme.bg._300)
                                .continuous_corners(COVER_RADIUS)
                                .overflow_hidden()
                                .child(artwork),
                        )
                        .child(badge),
                ),
        )
    }

    fn metadata_rows(&self, theme: &AppTheme, state: &Entity<MediaPickerAudioMetadataEditorState>) -> Vec<GroupedBtn> {
        let title = Binding::entity(
            state.clone(),
            |state: &MediaPickerAudioMetadataEditorState| state.draft.title.clone(),
            |state, title, _| state.draft.title = title,
        );
        let artist = Binding::entity(
            state.clone(),
            |state: &MediaPickerAudioMetadataEditorState| state.draft.artist.clone(),
            |state, artist, _| state.draft.artist = artist,
        );
        vec![
            GroupedBtn::new("title", tr("music_upload_title"))
                .content_slot(self.field_row("title-field", tr("music_upload_title"), title, theme)),
            GroupedBtn::new("artist", tr("music_local_upload_artist"))
                .content_slot(self.field_row("artist-field", tr("music_local_upload_artist"), artist, theme)),
        ]
    }

    fn field_row(&self, name: &'static str, label: String, value: Binding<String>, theme: &AppTheme) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .min_h(px(56.0))
            .bg(theme.bg._200)
            .child(div().flex_none().child(MainText::new(label).px(14.0).color(theme.text.secondary).number_of_lines(1)))
            .child(
                div().flex_1().min_w_0().text_color(theme.text.primary).child(
                    ShapedTextField::new(self.child_id(name), "", value).text_align(TextAlign::Right),
                ),
            )
    }
}

impl RenderOnce for MediaPickerAudioMetadataEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme_store.read(cx).current_theme().clone();
        let draft = self.draft.clone();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MediaPickerAudioMetadataEditorState {
            draft,
            show_cover_picker: false,
        });

        let on_cancel = self.on_cancel.clone();
        let header = div().w_full().px(px(14.0)).py(px(10.0)).child(
            zstack(Alignment::Center)
                .w_full()
                .child(
                    MainText::new(tr("media_picker_audio_edit_title"))
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .child(
                    hstack(8.0)
                        .w_full()
                        .child(
                            PageHeaderIconButton::new(self.child_id("cancel"), "xmark", move |window, cx| {
                                on_cancel(window, cx)
                            })
                            .font_size(15.0),
                        )
                        .child(spacer()),
                ),
        );

        let cover_row = GroupedBtn::new("cover", tr("music_local_upload_cover_optional")).content_slot(
            div().w_full().flex().justify_center().py(px(16.0)).child(self.cover(&theme, &state, cx)),
        );

        let can_save = Self::can_save(&state.read(cx).draft);
        let (on_complete, send_state) = (self.on_complete.clone(), state.clone());
        let save_bar = div().w_full().px(px(16.0)).py(px(10.0)).bg(theme.bg._100.opacity(0.94)).child(
            ButtonUi::new(self.child_id("send"))
                .disabled(!can_save)
                .background_color(theme.primary._100)
                .height(56.0)
                .on_press(move |window, cx| {
                    let draft = send_state.read(cx).draft.clone();
                    on_complete(draft, window, cx);
                })
                .child(
                    MainText::new(tr("media_picker_audio_send")).px(16.0).font_weight(FontWeight::BOLD).color(theme.content_on(theme.primary._100)),
                ),
        );

        let show_cover_picker = Binding::entity(
            state.clone(),
            |state: &MediaPickerAudioMetadataEditorState| state.show_cover_picker,
            |state, show, _| state.show_cover_picker = show,
        );
        let cover_state = state.clone();
        let cover_picker = MediaPickerSheet::new(
            self.child_id("cover-picker"),
            show_cover_picker,
            MediaPickerConfiguration::track_cover(),
            self.theme_store.clone(),
            move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                if let Some(item) = result.items.first() {
                    let url = item.url.clone();
                    cover_state.update(cx, |state, cx| {
                        state.draft.cover_url = Some(url);
                        cx.notify();
                    });
                }
            },
        )
        .experimental_bottom_sheet(true);

        vstack(0.0)
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(header)
            .child(
                div().flex_1().min_h_0().w_full().child(
                    scroll_view(self.child_id("scroll")).child(
                        vstack(18.0)
                            .w_full()
                            .pt(px(8.0))
                            .pb(px(94.0))
                            .child(GroupedBtns::new(self.child_id("cover-group"), vec![cover_row]))
                            .child(GroupedBtns::new(self.child_id("metadata"), self.metadata_rows(&theme, &state))),
                    ),
                ),
            )
            .child(save_bar)
            .child(cover_picker)
    }
}
