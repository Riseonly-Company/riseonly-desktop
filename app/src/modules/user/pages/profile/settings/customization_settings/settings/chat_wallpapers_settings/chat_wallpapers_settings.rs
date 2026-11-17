use std::sync::Arc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, Hsla, SharedString, Subscription, Window, black, white};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::theme;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::WallpaperTileMessages;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperLook, ChatWallpaperPreset, ChatWallpaperStore,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatWallpaperCatalogEntry, ChatWallpaperCatalogState, ChatWallpaperPlacement,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::RiseChatWallpaper;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::RiseThemeWallpapersLoadState;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_interactions::theme_wallpapers_interactions::ThemeWallpapersInteractionsStore;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_services::theme_wallpapers_services::ThemeWallpapersServicesStore;
use crate::modules::user::pages::profile::settings::customization_settings::settings::chat_wallpapers_settings::wallpaper_look_editor_sheet::{
    WallpaperLookEditorSheet, WallpaperLookEditorSheetTarget,
};
use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_well::ColorPickerWell;

const MIN_COLUMNS: usize = 2;
const MIN_TILE_WIDTH: f32 = 150.0;
const GRID_SPACING: f32 = 10.0;
const GRID_INSET: f32 = 12.0;
const TILE_ASPECT: f32 = 0.68;
const TILE_RADIUS: f32 = 16.0;
const TILE_PIXEL_DIMENSION: i64 = 960;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// where a picked wallpaper goes
/// account joins own wallpapers with presets and gradient controls
/// chat joins one chats catalogue and pins for everyone, photos only
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChatWallpapersScope {
    #[default]
    Account,
    Chat {
        chat_id: String,
        topic_id: Option<String>,
    },
}

impl ChatWallpapersScope {
    pub fn chat_id(&self) -> Option<&str> {
        match self {
            Self::Chat { chat_id, .. } => Some(chat_id),
            Self::Account => None,
        }
    }

    pub fn topic_id(&self) -> Option<&str> {
        match self {
            Self::Chat { topic_id: Some(topic_id), .. } if !topic_id.is_empty() => Some(topic_id),
            _ => None,
        }
    }
}

/// sheet item,
/// fresh identity per presentation and it outlives dismissal so the sheet keeps content while
/// animating out
#[derive(Clone)]
struct EditorTarget {
    id: String,
    target: WallpaperLookEditorSheetTarget,
}

impl EditorTarget {
    fn new(target: WallpaperLookEditorSheetTarget) -> Self {
        Self { id: Uuid::new_v4().uuid_string(), target }
    }
}

/// page state plus the reads of its body
struct ChatWallpapersSettingsState {
    picker_open: bool,
    editor: Option<EditorTarget>,
    is_editor_presented: bool,
    chat_apply_error: Option<String>,
    grid_width: f32,
    _subscriptions: [Subscription; 3],
}

impl ChatWallpapersSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let wallpaper_store = ChatWallpaperStore::shared(cx);
        let services = ThemeWallpapersServicesStore::shared(cx);
        let chat_services = ChatServicesStore::shared(cx);
        Self {
            picker_open: false,
            editor: None,
            is_editor_presented: false,
            chat_apply_error: None,
            grid_width: 0.0,
            _subscriptions: [
                cx.observe(&wallpaper_store, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&chat_services, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn present_editor(&mut self, target: WallpaperLookEditorSheetTarget, cx: &mut Context<Self>) {
        self.editor = Some(EditorTarget::new(target));
        self.is_editor_presented = true;
        cx.notify();
    }

    fn set_chat_apply_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.chat_apply_error = error;
        cx.notify();
    }

    /// at least two columns of portrait tiles, a wider window adds columns instead of stretching a tile
    fn column_count(&self) -> usize {
        let usable = self.grid_width - GRID_INSET * 2.0 + GRID_SPACING;
        MIN_COLUMNS.max((usable / (MIN_TILE_WIDTH + GRID_SPACING)) as usize)
    }
}

#[derive(IntoElement)]
pub struct ChatWallpapersSettings {
    id: ElementId,
    scope: ChatWallpapersScope,
}

impl Default for ChatWallpapersSettings {
    fn default() -> Self {
        Self::new(ChatWallpapersScope::Account)
    }
}

impl ChatWallpapersSettings {
    pub fn new(scope: ChatWallpapersScope) -> Self {
        Self { id: ElementId::Name(SharedString::new_static("chat-wallpapers-settings")), scope }
    }

    fn page_title(&self) -> String {
        match self.scope {
            ChatWallpapersScope::Account => tr("chat_wallpapers_title"),
            ChatWallpapersScope::Chat { .. } => {
                if self.scope.topic_id().is_none() {
                    tr("wallpaper_chat_pick_title")
                } else {
                    tr("wallpaper_topic_title")
                }
            }
        }
    }

    /// mounted always so the shell can animate out. a pick goes to the look editor first and the
    /// upload starts there
    fn picker(&self, state: &Entity<ChatWallpapersSettingsState>, cx: &mut App) -> MediaPickerSheet {
        let read_state = state.clone();
        let write_state = state.clone();
        let complete_state = state.clone();
        MediaPickerSheet::new(
            named_child(&self.id, "picker"),
            Binding::new(
                move |cx: &App| read_state.read(cx).picker_open,
                move |open: bool, _: &mut Window, cx: &mut App| {
                    write_state.update(cx, |state, cx| {
                        state.picker_open = open;
                        cx.notify();
                    });
                },
            ),
            MediaPickerConfiguration::chat_wallpaper(),
            ThemeStore::shared(cx),
            move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                complete_state.update(cx, |state, cx| {
                    state.picker_open = false;
                    cx.notify();
                });
                if result.was_cancelled {
                    return;
                }
                let Some(draft) = Self::draft(&result) else {
                    return;
                };
                let editor_state = complete_state.clone();
                cx.defer(move |cx| {
                    editor_state.update(cx, |state, cx| {
                        state.present_editor(WallpaperLookEditorSheetTarget::Draft(draft), cx);
                    });
                });
            },
        )
        .experimental_bottom_sheet(true)
    }

    fn editor_sheet(&self, state: &Entity<ChatWallpapersSettingsState>) -> BottomSheetUi {
        let read_state = state.clone();
        let write_state = state.clone();
        let content_state = state.clone();
        let dismissed_state = state.clone();
        let scope = self.scope.clone();
        let content_id = named_child(&self.id, "editor");
        BottomSheetUi::new(
            named_child(&self.id, "editor-sheet"),
            Binding::new(
                move |cx: &App| read_state.read(cx).is_editor_presented,
                move |presented: bool, _: &mut Window, cx: &mut App| {
                    write_state.update(cx, |state, cx| {
                        state.is_editor_presented = presented;
                        cx.notify();
                    });
                },
            ),
            move |_, cx| {
                let Some(editing) = content_state.read(cx).editor.clone() else {
                    return Empty.into_any_element();
                };
                let upload_scope = scope.clone();
                let upload_state = content_state.clone();
                let upload_target = editing.target.clone();
                let discard_target = editing.target.clone();
                let close_state = content_state.clone();
                env_scope::<DismissKey>(DismissAction::new(move |_, cx| {
                    close_state.update(cx, |state, cx| {
                        state.is_editor_presented = false;
                        cx.notify();
                    });
                }))
                .child(
                    div().id(named_child(&content_id, editing.id)).size_full().child(
                        WallpaperLookEditorSheet::new(editing.target)
                            .on_upload(move |look, _, cx| {
                                if let WallpaperLookEditorSheetTarget::Draft(draft) = &upload_target {
                                    Self::upload(&upload_scope, &upload_state, draft.clone(), look, cx);
                                }
                            })
                            .on_discard(move |_, _| {
                                if let WallpaperLookEditorSheetTarget::Draft(draft) = &discard_target {
                                    RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
                                }
                            }),
                    ),
                )
                .into_any_element()
            },
            |_, _| Empty,
        )
        .full_page(true)
        .full_page_shows_drag_indicator(true)
        .on_did_dismiss(move |_, cx| {
            dismissed_state.update(cx, |state, cx| {
                state.editor = None;
                cx.notify();
            });
        })
    }

    fn upload(
        scope: &ChatWallpapersScope,
        state: &Entity<ChatWallpapersSettingsState>,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        cx: &mut App,
    ) {
        match scope {
            ChatWallpapersScope::Account => {
                ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.add(draft, look, None, cx));
            }
            ChatWallpapersScope::Chat { chat_id, topic_id } => {
                state.update(cx, |state, cx| state.set_chat_apply_error(None, cx));
                let result_state = state.clone();
                let result_chat_id = chat_id.clone();
                let result_topic_id = topic_id.clone();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.add_chat_wallpaper(
                        chat_id,
                        topic_id.as_deref(),
                        draft,
                        look,
                        Some(Box::new(move |ok: bool, message: Option<String>, cx: &mut App| {
                            cx.defer(move |cx| {
                                let services = ChatServicesStore::shared(cx);
                                let catalog_error = services
                                    .read(cx)
                                    .wallpaper_catalog(&result_chat_id, result_topic_id.as_deref(), cx)
                                    .error;
                                if !ok && message.is_some() && catalog_error != message {
                                    result_state.update(cx, |state, cx| state.set_chat_apply_error(message, cx));
                                }
                            });
                        })),
                        cx,
                    );
                });
            }
        }
    }

    /// photos only, the preset refuses video at the fetch so anything else came from the document source
    pub fn draft(result: &MediaPickerResult) -> Option<RiseChatWallpaperDraft> {
        let item = result.items.iter().find(|item| item.kind == MediaPickerItemKind::Image && !item.is_file)?;
        let content_type = Self::image_content_type(item);
        Some(RiseChatWallpaperDraft::new(item.url.clone(), Self::filename(item, content_type), content_type.to_owned()))
    }

    /// named from the content type, not the picker
    /// upload policy refuses an extension that disagrees with bytes
    /// exporter re-encodes, eg heic leaves as png
    pub fn filename(item: &MediaPickerItem, content_type: &str) -> String {
        let file_extension = match content_type {
            "image/png" => "png",
            "image/heic" => "heic",
            "image/webp" => "webp",
            _ => "jpg",
        };
        let stem = Self::local_path(item)
            .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
            .unwrap_or_default();
        format!("{}.{file_extension}", if stem.is_empty() { "wallpaper" } else { &stem })
    }

    pub fn image_content_type(item: &MediaPickerItem) -> &'static str {
        if let Some(payload) = &item.image_payload {
            if payload.data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                return "image/png";
            }
            if payload.data.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg";
            }
        }
        let path_extension = Self::local_path(item)
            .and_then(|path| path.extension().map(|extension| extension.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        match path_extension.as_str() {
            "png" => "image/png",
            "heic" | "heif" => "image/heic",
            "webp" => "image/webp",
            _ => "image/jpeg",
        }
    }

    fn local_path(item: &MediaPickerItem) -> Option<std::path::PathBuf> {
        item.url.to_file_path()
    }
}

impl RenderOnce for ChatWallpapersSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state =
            window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| ChatWallpapersSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let task_scope = self.scope.clone();

        let wrapper = SettingsWrapperUi::new(self.page_title(), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(ChatWallpapersSettingsBody {
            id: named_child(&self.id, "body"),
            scope: self.scope.clone(),
            state: state.clone(),
        })
        .on_appear(named_child(&self.id, "task"), move |_, cx| match &task_scope {
            ChatWallpapersScope::Account => {
                ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.load(false, cx));
            }
            ChatWallpapersScope::Chat { chat_id, topic_id } => {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_chat_wallpaper_catalog(chat_id, topic_id.as_deref(), cx);
                });
            }
        });

        div()
            .size_full()
            .relative()
            .child(wrapper)
            .child(self.picker(&state, cx))
            .child(self.editor_sheet(&state))
    }
}

#[derive(IntoElement)]
struct ChatWallpapersSettingsBody {
    id: ElementId,
    scope: ChatWallpapersScope,
    state: Entity<ChatWallpapersSettingsState>,
}

impl ChatWallpapersSettingsBody {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, name)
    }

    fn current_placement(&self, cx: &mut App) -> Option<ChatWallpaperPlacement> {
        let chat_id = self.scope.chat_id()?;
        ChatServicesStore::shared(cx).update(cx, |services, cx| match self.scope.topic_id() {
            None => services.chat_info(chat_id, cx)?.wallpaper,
            Some(topic_id) => services.topic_from_any_cache(chat_id, topic_id, cx)?.wallpaper,
        })
    }

    fn catalog(&self, cx: &mut App) -> ChatWallpaperCatalogState {
        let Some(chat_id) = self.scope.chat_id() else {
            return ChatWallpaperCatalogState::default();
        };
        let services = ChatServicesStore::shared(cx);
        services.read(cx).wallpaper_catalog(chat_id, self.scope.topic_id(), cx)
    }

    fn grid(&self, columns: usize) -> Div {
        div()
            .w_full()
            .grid()
            .grid_cols(columns as u16)
            .gap(px(GRID_SPACING))
    }

    fn account_mode_card(&self, own_only: bool, store: Entity<ChatWallpaperStore>) -> GroupedBtns {
        let own_only_binding = Binding::new(
            move |cx: &App| store.read(cx).own_wallpapers_only(),
            |value: bool, _: &mut Window, cx: &mut App| {
                ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.set_own_only(value, cx));
            },
        );
        GroupedBtns::new(
            self.child_id("mode"),
            vec![
                GroupedBtn::new("wallpaper_own_only", tr("wallpaper_own_only_title"))
                    .icon("person.crop.rectangle")
                    .icon_background(SettingsIconPalette::customization())
                    .trailing_slot(SwitchUi::new(self.child_id("own-only-switch"), own_only_binding))
                    .callback(move |_, cx| {
                        ThemeWallpapersInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.set_own_only(!own_only, cx));
                    }),
            ],
        )
        .end_group_title(tr("wallpaper_own_only_hint"))
    }

    fn own_wallpapers(&self, uses_image_wallpaper: bool, columns: usize, theme: &AppTheme, cx: &mut App) -> Div {
        let services = ThemeWallpapersServicesStore::shared(cx);
        let wallpapers = services.read(cx).wallpapers(cx);
        let load_state = services.read(cx).load_state(cx);

        let body = if matches!(load_state, RiseThemeWallpapersLoadState::Loading) && wallpapers.is_empty() {
            self.grid_skeleton(columns).into_any_element()
        } else if matches!(load_state, RiseThemeWallpapersLoadState::Failed) && wallpapers.is_empty() {
            let message = services.read(cx).load_error(cx).unwrap_or_else(|| tr("chat_wallpaper_load_failed"));
            self.load_failure(message, theme).into_any_element()
        } else {
            self.own_grid(&wallpapers, uses_image_wallpaper, columns, theme, cx).into_any_element()
        };

        let mut column = vstack(10.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(Self::section_header(tr("chat_wallpaper_my_wallpapers")))
            .child(body)
            .child(
                div().px(px(16.0)).child(
                    MainText::new(tr("chat_wallpaper_add_hint"))
                        .px(12.0)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                ),
            );

        if let Some(mutation_error) = services.read(cx).mutation_error(cx).filter(|error| !error.is_empty()) {
            column = column.child(
                div()
                    .id(self.child_id("mutation-error"))
                    .px(px(16.0))
                    .cursor_pointer()
                    .on_click(|_, _, cx| {
                        ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.dismiss_mutation_error(cx);
                        });
                    })
                    .child(MainText::new(mutation_error).px(12.0).color(theme.primary._100).number_of_lines(3)),
            );
        }
        column
    }

    fn load_failure(&self, message: String, theme: &AppTheme) -> Div {
        div().w_full().px(px(GRID_INSET)).child(
            vstack(10.0)
                .w_full()
                .py(px(18.0))
                .px(px(16.0))
                .bg(theme.bg._200)
                .clip_shape(Shape::rounded_rect(14.0))
                .child(MainText::new(message).px(13.0).color(theme.text.secondary).number_of_lines(3))
                .child(
                    plain(self.child_id("retry"))
                        .on_press(|_, cx| {
                            ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_load(cx));
                        })
                        .child(
                            div().px(px(18.0)).py(px(9.0)).bg(theme.primary._100).rounded_full().child(
                                MainText::new(tr("common_retry"))
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.content_on(theme.primary._100)),
                            ),
                        ),
                ),
        )
    }

    fn own_grid(
        &self,
        wallpapers: &[RiseChatWallpaper],
        uses_image_wallpaper: bool,
        columns: usize,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Div {
        let can_add_more = ThemeWallpapersInteractionsStore::shared(cx).read(cx).can_add_more(cx);
        let first_id = wallpapers.first().map(|wallpaper| wallpaper.id.clone());

        let mut grid = self
            .grid(columns)
            .child(self.add_tile(can_add_more, false, theme).guide_anchor(RiseGuideAnchor::WallpaperAddPhoto));
        for (index, wallpaper) in wallpapers.iter().enumerate() {
            let anchor = (Some(&wallpaper.id) == first_id.as_ref()).then_some(RiseGuideAnchor::WallpaperOwnFirst);
            grid = grid.child(self.own_tile(wallpaper, index, uses_image_wallpaper, cx).guide_anchor(anchor));
        }

        let mut column = vstack(10.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(div().w_full().px(px(GRID_INSET)).child(grid));
        if wallpapers.is_empty() {
            column = column.child(
                div()
                    .px(px(16.0))
                    .child(MainText::new(tr("chat_wallpaper_empty")).px(12.0).color(theme.text.secondary).number_of_lines(2)),
            );
        }
        column
    }

    fn grid_skeleton(&self, columns: usize) -> Div {
        let mut grid = self.grid(columns);
        for _ in 0..columns {
            grid = grid.child(
                div().w_full().aspect_ratio(TILE_ASPECT).child(SkeletonBlock::new(None::<f32>, None::<f32>, TILE_RADIUS)),
            );
        }
        div().w_full().px(px(GRID_INSET)).child(grid.skeleton_shimmer(None))
    }

    fn add_tile(&self, enabled: bool, busy: bool, theme: &AppTheme) -> PlainButton {
        let state = self.state.clone();
        let label = if busy {
            LoaderUi::new().size(LoaderUiSize::Small).color(Some(theme.primary._100)).into_any_element()
        } else {
            vstack(8.0)
                .px(px(10.0))
                .children(IconUi::sized("plus", px(24.0), theme.primary._100, false))
                .child(
                    MainText::new(tr("chat_wallpaper_add"))
                        .px(12.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                )
                .into_any_element()
        };
        plain(self.child_id("add-tile"))
            .w_full()
            .opacity(if enabled { 1.0 } else { 0.45 })
            .disabled(!enabled || busy)
            .on_press(move |_, cx| {
                if !enabled || busy {
                    return;
                }
                state.update(cx, |state, cx| {
                    state.picker_open = true;
                    cx.notify();
                });
            })
            .child(
                div()
                    .w_full()
                    .aspect_ratio(TILE_ASPECT)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg._200)
                    .clip_shape(Shape::continuous(TILE_RADIUS))
                    .child(label),
            )
    }

    fn own_tile(
        &self,
        wallpaper: &RiseChatWallpaper,
        index: usize,
        uses_image_wallpaper: bool,
        cx: &mut App,
    ) -> PlainButton {
        let services = ThemeWallpapersServicesStore::shared(cx);
        let is_selected =
            uses_image_wallpaper && services.read(cx).selected_id(cx).as_deref() == Some(wallpaper.id.as_str());
        let is_busy = services.read(cx).is_mutating(&wallpaper.id, cx) || wallpaper.is_pending;
        let is_pending = wallpaper.is_pending;
        let wallpaper_id = wallpaper.id.clone();
        let state = self.state.clone();

        plain(self.child_id(format!("own-{}", wallpaper.id)))
            .w_full()
            .relative()
            .on_press(move |_, cx| {
                if is_pending {
                    return;
                }
                let id = wallpaper_id.clone();
                state.update(cx, |state, cx| {
                    state.present_editor(WallpaperLookEditorSheetTarget::Own { id }, cx);
                });
            })
            .child(
                WallpaperTile::new(
                    self.child_id(format!("own-tile-{}", wallpaper.id)),
                    None,
                    Some(wallpaper.display_url().to_owned()),
                    Some(wallpaper.look()),
                    is_selected,
                )
                .aspect(TILE_ASPECT)
                .radius(TILE_RADIUS)
                .accessibility_name(Self::tile_name(tr("chat_wallpaper_custom_name"), Some(index))),
            )
            .children(Self::busy_overlay(is_busy))
    }

    fn tile_name(group: String, index: Option<usize>) -> String {
        match index {
            Some(index) => format!("{group} {}", index + 1),
            None => group,
        }
    }

    fn busy_overlay(is_busy: bool) -> Option<Div> {
        is_busy.then(|| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.35))
                .clip_shape(Shape::continuous(TILE_RADIUS))
                .child(LoaderUi::new().size(LoaderUiSize::Small).color(Some(white())))
        })
    }

    fn chat_catalog(&self, columns: usize, theme: &AppTheme, cx: &mut App) -> Div {
        let catalog = self.catalog(cx);
        let placement_id = self.current_placement(cx).map(|placement| placement.id);

        let mut column = vstack(10.0).w_full().align(HorizontalAlignment::Leading);
        if !catalog.is_loaded && catalog.is_loading {
            column = column.child(self.grid_skeleton(columns));
        } else {
            let first_id = catalog.entries.first().map(|entry| entry.id.clone());
            let mut grid = self.grid(columns).child(self.add_tile(true, catalog.is_uploading, theme));
            for (index, entry) in catalog.entries.iter().enumerate() {
                let anchor =
                    (Some(&entry.id) == first_id.as_ref()).then_some(RiseGuideAnchor::WallpaperPickerFirstTile);
                grid = grid.child(self.catalog_tile(entry, index, &catalog, placement_id.as_deref()).guide_anchor(anchor));
            }
            column = column.child(div().w_full().px(px(GRID_INSET)).child(grid));
        }

        if let Some(error) = catalog.error.as_ref().filter(|error| !error.is_empty()) {
            column = column.child(
                div()
                    .px(px(16.0))
                    .child(MainText::new(error.clone()).px(12.0).color(theme.primary._100).number_of_lines(3)),
            );
        }
        column
    }

    fn catalog_tile(
        &self,
        entry: &ChatWallpaperCatalogEntry,
        index: usize,
        catalog: &ChatWallpaperCatalogState,
        placement_id: Option<&str>,
    ) -> PlainButton {
        let is_selected = placement_id == Some(entry.id.as_str());
        let is_busy = catalog.mutating_ids.contains(&entry.id);
        let state = self.state.clone();
        let tapped_entry = entry.clone();
        let chat_id = self.scope.chat_id().map(str::to_owned);
        let topic_id = self.scope.topic_id().map(str::to_owned);

        plain(self.child_id(format!("catalog-{}", entry.id)))
            .w_full()
            .relative()
            .on_press(move |_, cx| {
                let Some(chat_id) = chat_id.clone() else {
                    return;
                };
                if is_busy {
                    return;
                }
                let entry = tapped_entry.clone();
                let topic_id = topic_id.clone();
                state.update(cx, |state, cx| {
                    state.present_editor(WallpaperLookEditorSheetTarget::ChatEntry { entry, chat_id, topic_id }, cx);
                });
            })
            .child(
                WallpaperTile::new(
                    self.child_id(format!("catalog-tile-{}", entry.id)),
                    None,
                    Some(entry.display_url().to_owned()),
                    Some(entry.look),
                    is_selected,
                )
                .aspect(TILE_ASPECT)
                .radius(TILE_RADIUS)
                .accessibility_name(Self::tile_name(tr("wallpapers_title"), Some(index))),
            )
            .children(Self::busy_overlay(is_busy))
    }

    fn remove_chat_wallpaper_card(&self, cx: &mut App) -> Option<GroupedBtns> {
        self.current_placement(cx)?;
        let chat_id = self.scope.chat_id()?.to_owned();
        let topic_id = self.scope.topic_id().map(str::to_owned);
        let state = self.state.clone();
        Some(GroupedBtns::new(
            self.child_id("remove"),
            vec![
                GroupedBtn::new("chat_wallpaper_remove", tr("wallpaper_chat_remove"))
                    .icon("trash")
                    .icon_background(SettingsIconPalette::delete_account())
                    .destructive(true)
                    .show_chevron(false)
                    .callback(move |_, cx| Self::remove_from_chat(&chat_id, topic_id.as_deref(), &state, cx)),
            ],
        ))
    }

    fn remove_from_chat(
        chat_id: &str,
        topic_id: Option<&str>,
        state: &Entity<ChatWallpapersSettingsState>,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| state.set_chat_apply_error(None, cx));
        let result_state = state.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.clear_chat_wallpaper(
                chat_id,
                topic_id,
                Some(Box::new(move |ok: bool, message: Option<String>, cx: &mut App| {
                    if !ok {
                        let error = message.unwrap_or_else(|| tr("wallpaper_chat_apply_failed"));
                        result_state.update(cx, |state, cx| state.set_chat_apply_error(Some(error), cx));
                    }
                })),
                cx,
            );
        });
    }

    fn presets(&self, uses_image_wallpaper: bool, selected_preset_id: &str, columns: usize) -> Div {
        let mut grid = self.grid(columns);
        for (index, preset) in ChatWallpaperPreset::all().iter().enumerate() {
            let is_selected = !uses_image_wallpaper && selected_preset_id == preset.id;
            let tapped = preset.clone();
            grid = grid.child(
                plain(self.child_id(format!("preset-{}", preset.id)))
                    .w_full()
                    .on_press(move |_, cx| Self::select_preset(&tapped, cx))
                    .child(
                        WallpaperTile::new(
                            self.child_id(format!("preset-tile-{}", preset.id)),
                            Some(preset.clone()),
                            None,
                            None,
                            is_selected,
                        )
                        .aspect(TILE_ASPECT)
                        .radius(TILE_RADIUS)
                        .accessibility_name(Self::tile_name(tr("chat_wallpaper_presets"), Some(index))),
                    ),
            );
        }
        vstack(10.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(Self::section_header(tr("chat_wallpaper_presets")))
            .child(div().w_full().px(px(GRID_INSET)).child(grid))
    }

    fn chat_scope_footer(chat_apply_error: Option<&str>, theme: &AppTheme) -> Option<Div> {
        let error = chat_apply_error.filter(|error| !error.is_empty())?;
        Some(
            div()
                .w_full()
                .px(px(16.0))
                .child(MainText::new(error.to_owned()).px(12.0).color(theme.primary._100).number_of_lines(3)),
        )
    }

    fn section_header(text: String) -> Div {
        hstack(0.0)
            .w_full()
            .px(px(GRID_INSET))
            .child(MainText::new(text).px(14.0).font_weight(FontWeight::SEMIBOLD))
            .child(spacer())
    }

    fn gradient_card(&self, pattern_intensity: f64, theme: &AppTheme, cx: &mut App) -> Div {
        let store = ChatWallpaperStore::shared(cx);
        let intensity_read = store.clone();
        let intensity_write = store.clone();
        let motion_read = store.clone();
        let motion_write = store.clone();
        let motion_toggle = store.clone();

        let intensity = Binding::new(
            move |cx: &App| intensity_read.read(cx).pattern_intensity(),
            move |value: f64, _: &mut Window, cx: &mut App| {
                intensity_write.update(cx, |store, cx| store.set_pattern_intensity(value, cx));
            },
        );
        let motion = Binding::new(
            move |cx: &App| motion_read.read(cx).motion_enabled(),
            move |value: bool, _: &mut Window, cx: &mut App| {
                motion_write.update(cx, |store, cx| store.set_motion_enabled(value, cx));
            },
        );

        div().w_full().px(px(GRID_INSET)).child(
            vstack(14.0)
                .w_full()
                .px(px(16.0))
                .py(px(13.0))
                .bg(theme.bg._200)
                .clip_shape(Shape::rounded_rect(14.0))
                .child(
                    vstack(8.0)
                        .w_full()
                        .child(
                            hstack(0.0)
                                .w_full()
                                .child(MainText::new(tr("chat_wallpaper_pattern_intensity")).px(14.0))
                                .child(spacer())
                                .child(
                                    MainText::new(format!("{}%", (pattern_intensity * 100.0) as i64))
                                        .px(13.0)
                                        .color(theme.primary._100),
                                ),
                        )
                        .child(
                            div().w_full().child(
                                slider(self.child_id("intensity"), intensity, 0.18..=1.0).tint(theme.primary._100),
                            ),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .relative()
                        .child(
                            hstack(0.0)
                                .w_full()
                                .child(MainText::new(tr("chat_wallpaper_motion")).px(14.0))
                                .child(spacer())
                                .child(SwitchUi::new(self.child_id("motion-switch"), motion)),
                        )
                        .child(div().absolute().inset_0().occlude().child(
                            plain(self.child_id("motion-row")).size_full().on_press(move |_, cx| {
                                motion_toggle.update(cx, |store, cx| {
                                    let enabled = store.motion_enabled();
                                    store.set_motion_enabled(!enabled, cx);
                                });
                            }),
                        )),
                ),
        )
    }

    fn custom_palette_card(&self, current_preset: ChatWallpaperPreset, theme: &AppTheme, cx: &mut App) -> Div {
        let mut wells = hstack(0.0).w_full();
        for index in 0..4 {
            wells = wells.child(div().flex_1().flex().justify_center().child(ColorPickerWell::new(
                self.child_id(format!("custom-color-{index}")),
                Self::custom_color_binding(index, cx),
            )));
        }

        div().w_full().px(px(GRID_INSET)).child(
            vstack(12.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .p(px(14.0))
                .bg(theme.bg._200)
                .clip_shape(Shape::rounded_rect(14.0))
                .child(MainText::new(tr("chat_wallpaper_custom_palette")).px(14.0).font_weight(FontWeight::SEMIBOLD))
                .child(
                    div()
                        .w_full()
                        .h(px(180.0))
                        .relative()
                        .clip_shape(Shape::rounded_rect(14.0))
                        .child(
                            ChatDecorativeBackdrop::new()
                                .id(self.child_id("palette-backdrop"))
                                .corner_radius(14.0)
                                .ignores_safe_area(false)
                                .preview_preset(Some(current_preset))
                                .animates(false),
                        )
                        .child(div().absolute().inset_0().p(px(8.0)).child(WallpaperTileMessages::new())),
                )
                .child(wells)
                .child(
                    MainText::new(tr("chat_wallpaper_custom_palette_hint"))
                        .px(12.0)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                ),
        )
    }

    fn reset_card(&self) -> GroupedBtns {
        GroupedBtns::new(
            self.child_id("reset"),
            vec![
                GroupedBtn::new("chat_wallpapers_reset", tr("settings_reset_default"))
                    .icon("arrow.counterclockwise")
                    .icon_background(SettingsIconPalette::reset())
                    .show_chevron(false)
                    .callback(|_, cx| {
                        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.reset(cx));
                        ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_selection(cx));
                    }),
            ],
        )
    }

    fn custom_color_binding(index: usize, cx: &mut App) -> Binding<Hsla> {
        let store = ChatWallpaperStore::shared(cx);
        let read_store = store.clone();
        Binding::new(
            move |cx: &App| read_store.read(cx).custom_color(index),
            move |value: Hsla, _: &mut Window, cx: &mut App| {
                store.update(cx, |store, cx| store.update_custom_color(index, value, cx));
                ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_selection(cx));
            },
        )
    }

    fn select_preset(preset: &ChatWallpaperPreset, cx: &mut App) {
        haptics::selection_changed();
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.select(preset, cx));
        ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_selection(cx));
    }
}

impl RenderOnce for ChatWallpapersSettingsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx).clone();
        let store = ChatWallpaperStore::shared(cx);
        let (uses_image_wallpaper, own_only, selected_preset_id, pattern_intensity, current_preset) = {
            let store = store.read(cx);
            (
                store.uses_image_wallpaper(),
                store.own_wallpapers_only(),
                store.selected_preset_id(),
                store.pattern_intensity(),
                store.current_preset(),
            )
        };
        let (columns, chat_apply_error) = {
            let state = self.state.read(cx);
            (state.column_count(), state.chat_apply_error.clone())
        };

        let mut column = vstack(18.0).w_full().pt(px(4.0));
        match &self.scope {
            ChatWallpapersScope::Account => {
                column = column
                    .child(self.account_mode_card(own_only, store))
                    .child(self.own_wallpapers(uses_image_wallpaper, columns, &theme, cx))
                    .child(self.presets(uses_image_wallpaper, &selected_preset_id, columns));
                if !uses_image_wallpaper {
                    column = column
                        .child(
                            self.custom_palette_card(current_preset, &theme, cx)
                                .guide_anchor(RiseGuideAnchor::WallpaperPalette),
                        )
                        .child(
                            self.gradient_card(pattern_intensity, &theme, cx)
                                .guide_anchor(RiseGuideAnchor::WallpaperGradient),
                        );
                }
                column = column.child(self.reset_card());
            }
            ChatWallpapersScope::Chat { .. } => {
                column = column
                    .children(self.remove_chat_wallpaper_card(cx))
                    .child(self.chat_catalog(columns, &theme, cx))
                    .children(Self::chat_scope_footer(chat_apply_error.as_deref(), &theme));
            }
        }
        column = column.child(spacer_min_h(40.0));

        let width_state = self.state.clone();
        div()
            .size_full()
            .relative()
            .child(measure(self.child_id("width"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |state, cx| {
                    if state.grid_width == width {
                        return;
                    }
                    state.grid_width = width;
                    cx.notify();
                });
            }))
            .child(
                scroll_view(self.child_id("scroll"))
                    .size_full()
                    .bg(theme.bg._100)
                    .list_shell_scroll_under_chrome(cx)
                    .child(column),
            )
    }
}

/// one grid tile,
/// preset gradient or picture with its look. picture fills and is cropped,
/// never letterboxed
#[derive(IntoElement)]
pub struct WallpaperTile {
    id: ElementId,
    preset: Option<ChatWallpaperPreset>,
    image_url: Option<String>,
    look: Option<ChatWallpaperLook>,
    is_selected: bool,
    aspect: f32,
    radius: f32,
    accessibility_name: String,
}

impl WallpaperTile {
    pub fn new(
        id: impl Into<ElementId>,
        preset: Option<ChatWallpaperPreset>,
        image_url: Option<String>,
        look: Option<ChatWallpaperLook>,
        is_selected: bool,
    ) -> Self {
        Self {
            id: id.into(),
            preset,
            image_url,
            look,
            is_selected,
            aspect: TILE_ASPECT,
            radius: TILE_RADIUS,
            accessibility_name: String::new(),
        }
    }

    pub fn aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn accessibility_name(mut self, accessibility_name: impl Into<String>) -> Self {
        self.accessibility_name = accessibility_name.into();
        self
    }
}

impl RenderOnce for WallpaperTile {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx).clone();
        let shape = Shape::continuous(self.radius);
        let backdrop_id = named_child(&self.id, "backdrop");

        let mut tile = div()
            .id(self.id)
            .relative()
            .w_full()
            .aspect_ratio(self.aspect)
            .child(
                div().absolute().inset_0().clip_shape(shape).child(
                    ChatDecorativeBackdrop::new()
                        .id(backdrop_id)
                        .corner_radius(self.radius)
                        .ignores_safe_area(false)
                        .preview_preset(self.preset)
                        .preview_image_url(self.image_url)
                        .preview_look(self.look)
                        .preview_pixel_dimension(Some(TILE_PIXEL_DIMENSION))
                        .animates(false),
                ),
            )
            .child(div().absolute().inset_0().p(px(6.0)).child(WallpaperTileMessages::new().scale(0.7)));

        if self.is_selected {
            tile = tile
                .child(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .right(px(8.0))
                        .w(px(20.0))
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.primary._100)
                        .children(IconUi::sized("checkmark", px(12.0), theme.content_on(theme.primary._100), false)),
                )
                .child(div().absolute().inset_0().clip_shape(shape).stroke_border(theme.primary._100, 2.0));
        }

        if self.accessibility_name.is_empty() {
            tile
        } else {
            tile.tooltip_label(self.accessibility_name)
        }
    }
}
