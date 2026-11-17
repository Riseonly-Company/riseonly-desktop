use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AsyncWindowContext, Axis, ElementId, Entity, FontWeight, Hsla, ImageSource, ObjectFit, SharedString,
    Window, black, img, linear_color_stop, linear_gradient, transparent_black, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::pasteboard::{UIPasteboard, UIPasteboardItem, UIPasteboardOptions};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::swiftui::text_selection::SelectableText;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_theme::theme::theme_environment::theme_store;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotModel;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{ChatBackdropLayers, ChatBackdropLayout};
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

const ICON_TILE_SIDE: f32 = 29.0;
const ICON_TILE_RADIUS: f32 = 8.0;
const ICON_TILE_GLYPH: f32 = 14.0;
const BANNER_HEIGHT: f32 = 142.0;
const BANNER_RADIUS: f32 = 20.0;
const BANNER_PROGRESS_WIDTH: f32 = 120.0;
const BANNER_PREVIEW_PIXEL_DIMENSION: i64 = 1_536;
const BANNER_CONTENT_TYPE: &str = "image/jpeg";
const TOKEN_MASK_MIN: usize = 12;
const TOKEN_MASK_MAX: usize = 32;
const TOKEN_MASK_CHARACTER: &str = "\u{2022}";
const TOKEN_PASTEBOARD_LIFETIME: f64 = 120.0;
const TOKEN_HEIGHT_TOLERANCE: f32 = 0.5;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// lucide shapes with something inside turn into a blob when filled
/// so only plain silhouettes of the fill symbols get the filled form
pub fn bot_symbol_is_solid(system_image: &str) -> bool {
    system_image.ends_with(".fill")
        && ![".circle.fill", ".triangle.fill", "photo.fill", "tray.", "eye.", "lock."]
            .iter()
            .any(|inner| system_image.contains(inner))
}

fn icon_tile(icon: &str, background: Hsla, content: Hsla) -> Div {
    Shape::continuous(ICON_TILE_RADIUS)
        .round(div())
        .flex_none()
        .size(px(ICON_TILE_SIDE))
        .bg(background)
        .flex()
        .items_center()
        .justify_center()
        .children(IconUi::sized(icon, px(ICON_TILE_GLYPH), content, bot_symbol_is_solid(icon)))
}

#[derive(IntoElement)]
pub struct BotStateView {
    id: ElementId,
    system_image: SharedString,
    title: String,
    message: Option<String>,
    action_title: Option<String>,
    action: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl BotStateView {
    pub fn new(id: impl Into<ElementId>, system_image: impl Into<SharedString>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            system_image: system_image.into(),
            title: title.into(),
            message: None,
            action_title: None,
            action: None,
        }
    }

    pub fn message(mut self, message: impl Into<Option<String>>) -> Self {
        self.message = message.into();
        self
    }

    pub fn action_title(mut self, action_title: impl Into<Option<String>>) -> Self {
        self.action_title = action_title.into();
        self
    }

    pub fn action(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }
}

impl RenderOnce for BotStateView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let message = self.message.filter(|message| !message.is_empty());
        let button = self.action_title.zip(self.action);
        let button_id = child_id(&self.id, "action");
        vstack(12.0)
            .size_full()
            .px(px(20.0))
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex_none()
                    .size(px(66.0))
                    .rounded_full()
                    .bg(theme.primary._100.opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(
                        self.system_image.as_ref(),
                        px(34.0),
                        theme.primary._100,
                        bot_symbol_is_solid(self.system_image.as_ref()),
                    )),
            )
            .child(MainText::new(self.title).px(18.0).tac(TextAlignT::Center).font_weight(FontWeight::SEMIBOLD))
            .when_some(message, |column, message| {
                column.child(div().px(px(30.0)).child(SecondaryText::new(message).px(13.0).tac(TextAlignT::Center)))
            })
            .when_some(button, |column, (action_title, action)| {
                column.child(
                    ButtonUi::new(button_id)
                        .fit_content(true)
                        .padding_horizontal(24.0)
                        .on_press(move |window, cx| action(window, cx))
                        .child(MainText::new(action_title).px(14.0).font_weight(FontWeight::SEMIBOLD)),
                )
            })
    }
}

#[derive(IntoElement)]
pub struct BotAvatarView {
    id: ElementId,
    bot: BotModel,
    size: f32,
}

impl BotAvatarView {
    pub const DEFAULT_SIZE: f32 = 52.0;

    pub fn new(id: impl Into<ElementId>, bot: BotModel) -> Self {
        Self { id: id.into(), bot, size: Self::DEFAULT_SIZE }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for BotAvatarView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let user_id = if self.bot.bot_user_id.is_empty() { self.bot.id } else { self.bot.bot_user_id };
        UserLogoUi::new(self.id)
            .source(SharedString::from(self.bot.avatar_url))
            .name(SharedString::from(self.bot.name))
            .user_id(SharedString::from(user_id))
            .size(self.size)
    }
}

#[derive(IntoElement)]
pub struct BotSettingsFieldRow {
    id: ElementId,
    icon: SharedString,
    icon_background: Hsla,
    placeholder: String,
    text: Binding<String>,
    axis: Axis,
    trailing: Option<AnyElement>,
    rule: RiseFieldRule,
    error_message: Option<String>,
    is_secure: bool,
    on_submit: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_cancel: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl BotSettingsFieldRow {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        icon_background: Hsla,
        placeholder: impl Into<String>,
        text: Binding<String>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            icon_background,
            placeholder: placeholder.into(),
            text,
            axis: Axis::Horizontal,
            trailing: None,
            rule: RiseFieldRule::NONE,
            error_message: None,
            is_secure: false,
            on_submit: None,
            on_cancel: None,
        }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn autocorrection_disabled(self, _autocorrection_disabled: bool) -> Self {
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    pub fn rule(mut self, rule: RiseFieldRule) -> Self {
        self.rule = rule;
        self
    }

    pub fn error_message(mut self, error_message: Option<String>) -> Self {
        self.error_message = error_message;
        self
    }

    pub fn is_secure(mut self, is_secure: bool) -> Self {
        self.is_secure = is_secure;
        self
    }
}

impl RenderOnce for BotSettingsFieldRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let reveals_secure_text = window.use_keyed_state(child_id(&self.id, "reveal"), cx, |_, _| false);
        let revealed = *reveals_secure_text.read(cx);
        let is_vertical = self.axis == Axis::Vertical;
        let remaining = self.rule.remaining(&self.text.get(cx));
        let reveal_label = tr(if revealed { "bot_hide_token" } else { "bot_reveal_token" });
        let reveal_icon = if revealed { "eye.slash" } else { "eye" };
        let reveal_id = child_id(&self.id, "reveal-button");
        let reveal_hint_id = child_id(&self.id, "reveal-hint");

        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .py(px(if is_vertical { 14.0 } else { 12.0 }))
            .min_h(px(if is_vertical { 82.0 } else { 54.0 }))
            .when(is_vertical, |row| row.items_start())
            .when(!is_vertical, |row| row.items_center())
            .child(
                div().flex_none().when(is_vertical, |tile| tile.pt(px(2.0))).child(icon_tile(
                    self.icon.as_ref(),
                    self.icon_background,
                    theme.content_on(self.icon_background),
                )),
            )
            .child(
                ShapedTextField::new(child_id(&self.id, "field"), self.placeholder, self.text)
                    .axis(self.axis)
                    .max_visible_lines(if is_vertical { 5 } else { 1 })
                    .font_size(15.0)
                    .text_color(theme.text.primary)
                    .is_secure(self.is_secure && !revealed)
                    .when_some(self.on_submit, |field, on_submit| {
                        field.on_submit(move |window, cx| on_submit(window, cx))
                    })
                    .when_some(self.on_cancel, |field, on_cancel| {
                        field.on_cancel(move |window, cx| on_cancel(window, cx))
                    }),
            )
            .child(InputFieldAccessory::new(child_id(&self.id, "accessory"), remaining, self.error_message))
            .when(self.is_secure, |row| {
                row.child(
                    div().id(reveal_hint_id).flex_none().tooltip_label(reveal_label).child(
                        plain(reveal_id)
                            .on_press(move |_, cx| {
                                reveals_secure_text.update(cx, |revealed, cx| {
                                    *revealed = !*revealed;
                                    cx.notify();
                                });
                            })
                            .size(px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(reveal_icon, px(15.0), theme.text.secondary, false)),
                    ),
                )
            })
            .children(self.trailing)
    }
}

#[derive(IntoElement)]
pub struct BotToggleRow {
    id: ElementId,
    icon: SharedString,
    color: Hsla,
    title: String,
    subtitle: String,
    is_on: Binding<bool>,
}

impl BotToggleRow {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        color: Hsla,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        is_on: Binding<bool>,
    ) -> Self {
        Self { id: id.into(), icon: icon.into(), color, title: title.into(), subtitle: subtitle.into(), is_on }
    }
}

impl RenderOnce for BotToggleRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(16.0))
            .py(px(11.0))
            .child(icon_tile(self.icon.as_ref(), self.color, theme.content_on(self.color)))
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(MainText::new(self.title).px(15.0))
                    .child(SecondaryText::new(self.subtitle).px(12.0)),
            )
            .child(spacer_min_w(8.0).flex_grow_0())
            .child(SwitchUi::new(child_id(&self.id, "switch"), self.is_on))
    }
}

/// picker presented flag plus the turn of the picked file whose preview may still publish
/// decode runs off the main thread and can lose to a newer pick
struct BotBannerPickerState {
    is_picker_presented: bool,
    preview_generation: u64,
}

#[derive(IntoElement)]
pub struct BotBannerPicker {
    id: ElementId,
    local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    preview: Binding<Option<CleverImage>>,
    remote_url: String,
    upload_progress: f64,
    is_uploading: bool,
}

impl BotBannerPicker {
    pub fn new(
        id: impl Into<ElementId>,
        local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        preview: Binding<Option<CleverImage>>,
        remote_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            local_file,
            preview,
            remote_url: remote_url.into(),
            upload_progress: 0.0,
            is_uploading: false,
        }
    }

    pub fn upload_progress(mut self, upload_progress: f64) -> Self {
        self.upload_progress = upload_progress;
        self
    }

    pub fn is_uploading(mut self, is_uploading: bool) -> Self {
        self.is_uploading = is_uploading;
        self
    }

    fn banner_image(preview: Option<CleverImage>, remote_url: &str) -> AnyElement {
        if let Some(preview) = preview {
            return img(ImageSource::Render(preview)).size_full().object_fit(ObjectFit::Cover).into_any_element();
        }
        let backdrop = ChatBackdropLayers::new(ChatBackdropLayout::ProfileBanner);
        if remote_url.is_empty() {
            return div().size_full().child(backdrop).into_any_element();
        }
        div()
            .relative()
            .size_full()
            .child(div().absolute().inset_0().child(backdrop))
            .child(
                div().absolute().inset_0().child(
                    CleverImageUi::new(remote_url.to_owned())
                        .content_mode(ContentMode::Fill)
                        .corner_radius(0.0)
                        .shows_progress_indicator(false)
                        .shows_failure_icon(false),
                ),
            )
            .into_any_element()
    }

    fn handle_picker_result(
        state: &Entity<BotBannerPickerState>,
        local_file: &Binding<Option<SaiWsOptimisticLocalFile>>,
        preview: &Binding<Option<CleverImage>>,
        result: MediaPickerResult,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.is_picker_presented = false;
            cx.notify();
        });
        if result.was_cancelled {
            return;
        }
        let Some(item) = result.items.first() else {
            return;
        };
        let generation = state.update(cx, |state, _| {
            state.preview_generation += 1;
            state.preview_generation
        });
        let data = item.image_payload.as_ref().map(|payload| payload.data.clone());
        let path = item.url.to_file_path();
        let preview = preview.clone();
        let state = state.clone();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let decoded = CleverImageBitmapLoader::prepare_bitmap(
                    move || {
                        if let Some(decoded) = data
                            .and_then(|data| CleverImageBitmapDecoder::decode(&data, Some(BANNER_PREVIEW_PIXEL_DIMENSION)))
                        {
                            return Some(decoded);
                        }
                        let data = std::fs::read(path?).ok()?;
                        CleverImageBitmapDecoder::decode(&data, Some(BANNER_PREVIEW_PIXEL_DIMENSION))
                    },
                    cx,
                )
                .await;
                let Some(decoded) = decoded else {
                    return;
                };
                let _ = cx.update(|window, cx| {
                    if state.read(cx).preview_generation == generation {
                        preview.set(Some(decoded), window, cx);
                    }
                });
            })
            .detach();
        local_file.set(
            Some(SaiWsOptimisticLocalFile::new(
                item.url.clone(),
                item.suggested_file_name.clone(),
                BANNER_CONTENT_TYPE.to_owned(),
            )),
            window,
            cx,
        );
    }
}

impl RenderOnce for BotBannerPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| BotBannerPickerState {
            is_picker_presented: false,
            preview_generation: 0,
        });
        let banner = Self::banner_image(self.preview.get(cx), &self.remote_url);
        let press_state = state.clone();
        let complete_state = state.clone();
        let local_file = self.local_file.clone();
        let preview = self.preview.clone();
        let progress = self.upload_progress.clamp(0.0, 1.0);

        div()
            .relative()
            .w_full()
            .h(px(BANNER_HEIGHT))
            .child(
                plain(child_id(&self.id, "button"))
                    .on_press(move |_, cx| {
                        press_state.update(cx, |state, cx| {
                            state.is_picker_presented = true;
                            cx.notify();
                        });
                    })
                    .w_full()
                    .h(px(BANNER_HEIGHT))
                    .child(
                        Shape::continuous(BANNER_RADIUS)
                            .round(div())
                            .relative()
                            .size_full()
                            .overflow_hidden()
                            .child(banner)
                            .child(div().absolute().inset_0().bg(linear_gradient(
                                180.0,
                                linear_color_stop(transparent_black(), 0.0),
                                linear_color_stop(black().opacity(0.48), 1.0),
                            )))
                            .child(
                                hstack(7.0)
                                    .absolute()
                                    .inset_0()
                                    .items_center()
                                    .justify_center()
                                    .children(IconUi::sized("photo.fill", px(17.0), white(), false))
                                    .child(
                                        MainText::new(tr("bot_change_banner"))
                                            .px(13.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(white()),
                                    ),
                            )
                            .when(self.is_uploading, |card| {
                                card.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .bg(black().opacity(0.38))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .w(px(BANNER_PROGRESS_WIDTH))
                                                .child(progress_view_value(progress).tint(white())),
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                pass_through().absolute().inset_0().child(
                    MediaPickerSheet::new(
                        child_id(&self.id, "picker"),
                        Binding::entity(
                            state,
                            |state: &BotBannerPickerState| state.is_picker_presented,
                            |state: &mut BotBannerPickerState, is_picker_presented, cx| {
                                state.is_picker_presented = is_picker_presented;
                                cx.notify();
                            },
                        ),
                        MediaPickerConfiguration::track_cover(),
                        theme_store(cx),
                        move |result: MediaPickerResult, window, cx| {
                            Self::handle_picker_result(&complete_state, &local_file, &preview, result, window, cx);
                        },
                    )
                    .experimental_bottom_sheet(true),
                ),
            )
    }
}

struct BotOneTimeTokenSheetState {
    is_revealed: bool,
    copied: bool,
    content_height: f32,
}

/// content of the sheet, the presenter owns the detent
/// starts from INITIAL_CONTENT_HEIGHT, follows on_content_height no drag indicator and no
/// interactive dismissal
#[derive(IntoElement)]
pub struct BotOneTimeTokenSheet {
    id: ElementId,
    bot_name: String,
    token: String,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
    on_content_height: Option<Rc<dyn Fn(f32, &mut Window, &mut App)>>,
}

impl BotOneTimeTokenSheet {
    pub const INITIAL_CONTENT_HEIGHT: f32 = 480.0;

    pub fn new(
        id: impl Into<ElementId>,
        bot_name: impl Into<String>,
        token: impl Into<String>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            bot_name: bot_name.into(),
            token: token.into(),
            on_close: Rc::new(on_close),
            on_content_height: None,
        }
    }

    pub fn on_content_height(mut self, on_content_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_content_height = Some(Rc::new(on_content_height));
        self
    }

    fn masked(token: &str) -> String {
        TOKEN_MASK_CHARACTER.repeat(token.chars().count().clamp(TOKEN_MASK_MIN, TOKEN_MASK_MAX))
    }

    fn copy_token(token: &str, state: &Entity<BotOneTimeTokenSheetState>, cx: &mut App) {
        UIPasteboard::general().set_items(
            vec![UIPasteboardItem::PlainText(token.to_owned())],
            UIPasteboardOptions {
                local_only: true,
                expiration_date: Some(RiseClock::now().adding_time_interval(TOKEN_PASTEBOARD_LIFETIME)),
            },
            cx,
        );
        state.update(cx, |state, cx| {
            state.copied = true;
            cx.notify();
        });
    }
}

impl RenderOnce for BotOneTimeTokenSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| BotOneTimeTokenSheetState {
            is_revealed: false,
            copied: false,
            content_height: Self::INITIAL_CONTENT_HEIGHT,
        });
        let (is_revealed, copied) = {
            let state = state.read(cx);
            (state.is_revealed, state.copied)
        };
        let shown_token = if is_revealed { self.token.clone() } else { Self::masked(&self.token) };
        let reveal_state = state.clone();
        let copy_state = state.clone();
        let height_state = state;
        let copied_token = self.token;
        let on_close = self.on_close;
        let on_content_height = self.on_content_height;
        let primary = theme.primary._100;
        let on_primary = theme.content_on(primary);

        vstack(20.0)
            .relative()
            .w_full()
            .items_center()
            .px(px(20.0))
            .pt(px(24.0))
            .pb(px(24.0))
            .bg(theme.bg._100)
            .child(
                div()
                    .flex_none()
                    .size(px(58.0))
                    .rounded_full()
                    .bg(primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("key.fill", px(27.0), on_primary, true)),
            )
            .child(
                vstack(7.0)
                    .w_full()
                    .items_center()
                    .child(
                        MainText::new(tr("bot_token_created_title"))
                            .px(20.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(SecondaryText::new(tr("bot_token_created_message")).px(13.0).tac(TextAlignT::Center)),
            )
            .child(
                vstack(12.0)
                    .w_full()
                    .child(
                        Shape::continuous(14.0)
                            .round(hstack(10.0))
                            .w_full()
                            .items_center()
                            .p(px(14.0))
                            .bg(theme.bg._200)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .line_clamp(2)
                                    .text_color(theme.text.primary)
                                    .swiftui_font(Font::system_weight(13.0, FontWeight::MEDIUM).monospaced())
                                    .child(SelectableText::new(child_id(&self.id, "token"), shown_token)),
                            )
                            .child(spacer_min_w(8.0))
                            .child(
                                plain(child_id(&self.id, "reveal"))
                                    .on_press(move |_, cx| {
                                        reveal_state.update(cx, |state, cx| {
                                            state.is_revealed = !state.is_revealed;
                                            cx.notify();
                                        });
                                    })
                                    .flex_none()
                                    .size(px(36.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(IconUi::sized(
                                        if is_revealed { "eye.slash.fill" } else { "eye.fill" },
                                        px(17.0),
                                        primary,
                                        false,
                                    )),
                            )
                            .guide_anchor(RiseGuideAnchor::BotTokenValue),
                    )
                    .child(
                        ButtonUi::new(child_id(&self.id, "copy"))
                            .on_press(move |_, cx| Self::copy_token(&copied_token, &copy_state, cx))
                            .child(
                                hstack(7.0)
                                    .items_center()
                                    .children(IconUi::sized(
                                        if copied { "checkmark" } else { "doc.on.doc" },
                                        px(17.0),
                                        on_primary,
                                        false,
                                    ))
                                    .child(
                                        MainText::new(tr(if copied { "bot_token_copied" } else { "bot_token_copy" }))
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD),
                                    ),
                            ),
                    ),
            )
            .child(MainText::new(self.bot_name).px(13.0).color(theme.text.secondary))
            .child(
                plain(child_id(&self.id, "done"))
                    .on_press(move |window, cx| on_close(window, cx))
                    .w_full()
                    .min_h(px(44.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        MainText::new(tr("bot_token_done"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(primary),
                    )
                    .guide_anchor(RiseGuideAnchor::BotTokenDone),
            )
            .child(measure(child_id(&self.id, "height"), move |bounds, window, cx| {
                let height = f32::from(bounds.size.height);
                let changed = height_state.update(cx, |state, _| {
                    let changed = (height - state.content_height).abs() > TOKEN_HEIGHT_TOLERANCE;
                    if changed {
                        state.content_height = height;
                    }
                    changed
                });
                if changed && let Some(on_content_height) = on_content_height {
                    on_content_height(height, window, cx);
                }
            }))
    }
}
