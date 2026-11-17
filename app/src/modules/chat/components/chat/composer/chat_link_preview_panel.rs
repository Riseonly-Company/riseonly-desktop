use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, ElementId, Entity, FontWeight, SharedString, Window, white};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::{IconUi, MainText};

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HOLD_CONTEXT_MENU_DEFAULT_MAXIMUM_PREVIEW_WIDTH, HoldContextMenuPresentationActions,
    HoldContextMenuPresentationModifiers, HoldContextMenuPresentationOnAction, HoldContextMenuPresentationPreview,
};
use crate::core::ui::link_preview_card_ui::LinkPreviewCardPresentation;
use crate::modules::chat::components::chat::bubble::message_content::ChatMessageViewportWidthKey;
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::ChatLinkPreviewDraft;
use crate::modules::chat::components::chat::message_bubble::MessageBubble;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn non_empty(value: &str) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value);
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(IntoElement)]
pub struct ChatLinkPreviewBar {
    id: ElementId,
    pub draft: Entity<ChatLinkPreviewDraft>,
    pub theme: AppTheme,
}

impl ChatLinkPreviewBar {
    pub fn new(id: impl Into<ElementId>, draft: Entity<ChatLinkPreviewDraft>, theme: AppTheme) -> Self {
        Self { id: id.into(), draft, theme }
    }

    fn title(draft: &ChatLinkPreviewDraft) -> String {
        if draft.is_loading() {
            return tr("chat_link_preview_loading").to_string();
        }
        if draft.failed() {
            return tr("chat_link_preview_failed").to_string();
        }
        draft
            .preview()
            .and_then(|preview| preview.title)
            .as_deref()
            .and_then(non_empty)
            .or_else(|| Url::from_string(draft.url().unwrap_or_default()).and_then(|url| url.host()))
            .or_else(|| draft.url().map(str::to_owned))
            .unwrap_or_default()
    }

    fn subtitle(draft: &ChatLinkPreviewDraft) -> String {
        if draft.is_loading() {
            return tr("chat_link_preview_waiting").to_string();
        }
        if draft.failed() {
            return tr("chat_link_preview_retry").to_string();
        }
        draft
            .preview()
            .and_then(|preview| preview.description)
            .as_deref()
            .and_then(non_empty)
            .or_else(|| draft.url().map(str::to_owned))
            .unwrap_or_default()
    }

    /// image source and clip radius
    /// a riseonly profile or chat link draws as a round avatar
    fn thumbnail(draft: &ChatLinkPreviewDraft) -> Option<(String, f32)> {
        if draft.is_loading() || draft.failed() {
            return None;
        }
        let image = draft.preview()?.image_url.as_deref().and_then(non_empty)?;
        let url = draft.url()?;
        let radius = if LinkPreviewCardPresentation::is_avatar_link_url(url) { 19.0 } else { 8.0 };
        Some((image, radius))
    }
}

impl RenderOnce for ChatLinkPreviewBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let (title, subtitle, thumbnail) = {
            let draft = self.draft.read(cx);
            (Self::title(draft), Self::subtitle(draft), Self::thumbnail(draft))
        };
        let image_id = child_id(&self.id, "image");
        let tap_draft = self.draft.clone();
        let remove_draft = self.draft;

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                plain(child_id(&self.id, "open"))
                    .flex_1()
                    .min_w_0()
                    .on_press(move |_, cx| {
                        tap_draft.update(cx, |draft, cx| {
                            if draft.failed() {
                                draft.load(cx);
                            } else {
                                draft.present_settings(cx);
                            }
                        });
                    })
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .py(px(7.0))
                            .child(
                                div()
                                    .w(px(3.0))
                                    .self_stretch()
                                    .flex_none()
                                    .clip_shape(Shape::rounded_rect(2.0))
                                    .bg(theme.primary._100),
                            )
                            .children(thumbnail.map(|(image, radius)| {
                                div().size(px(38.0)).flex_none().clip_shape(Shape::continuous(radius)).child(
                                    CleverImageUi::new(image)
                                        .id(image_id)
                                        .content_mode(ContentMode::Fill)
                                        .corner_radius(8.0)
                                        .persist_to_disk(true)
                                        .shows_progress_indicator(false)
                                        .maximum_decode_pixel_dimension(114),
                                )
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .gap(px(2.0))
                                    .child(
                                        MainText::new(title)
                                            .px(14.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.primary._100)
                                            .number_of_lines(1),
                                    )
                                    .child(
                                        MainText::new(subtitle)
                                            .px(13.0)
                                            .color(theme.text.secondary)
                                            .number_of_lines(1),
                                    ),
                            ),
                    ),
            )
            .child(
                div().id(child_id(&self.id, "remove-tooltip")).flex_none().tooltip_label(tr("chat_link_preview_remove")).child(
                    plain(child_id(&self.id, "remove"))
                        .on_press(move |_, cx| remove_draft.update(cx, |draft, cx| draft.suppress(cx)))
                        .child(
                            div()
                                .w(px(36.0))
                                .h(px(44.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("xmark", px(16.0), theme.text.secondary, false)),
                        ),
                ),
            )
    }
}

pub type ChatLinkPreviewSettingsBackdrop = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub struct ChatLinkPreviewSettings {
    id: ElementId,
    pub draft: Entity<ChatLinkPreviewDraft>,
    pub text: String,
    pub entities: Option<String>,
    pub chat: Arc<ChatInfo>,
    pub backdrop: ChatLinkPreviewSettingsBackdrop,
}

impl ChatLinkPreviewSettings {
    pub fn new(
        id: impl Into<ElementId>,
        draft: Entity<ChatLinkPreviewDraft>,
        text: String,
        entities: Option<String>,
        chat: Arc<ChatInfo>,
        backdrop: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self { id: id.into(), draft, text, entities, chat, backdrop: Rc::new(backdrop) }
    }

    fn preview_message(draft: &ChatLinkPreviewDraft, chat: &ChatInfo, text: &str, entities: Option<&String>) -> GetMessageMessage {
        GetMessageMessage {
            id: "composer-link-preview".to_owned(),
            chat_id: chat.id.clone(),
            text: text.to_owned(),
            entities: entities.cloned(),
            created_at: draft.settings_opened_at(),
            link_preview: draft.preview(),
            ..GetMessageMessage::default()
        }
    }

    fn actions(draft: &ChatLinkPreviewDraft) -> Vec<HoldContextMenuAction> {
        let is_above_text = draft.is_above_text();
        let is_large = draft.is_large();
        let mut result = vec![
            HoldContextMenuAction::new(
                "position",
                tr(if is_above_text { "chat_link_preview_move_below" } else { "chat_link_preview_move_above" }),
            )
            .system_image(if is_above_text { "text.append" } else { "text.insert" })
            .dismisses_menu(false),
        ];
        let has_image = draft
            .preview()
            .and_then(|preview| preview.image_url)
            .as_deref()
            .is_some_and(|image| !trimming_whitespaces_and_newlines(image).is_empty());
        if has_image {
            result.push(
                HoldContextMenuAction::new(
                    "size",
                    tr(if is_large { "chat_link_preview_shrink" } else { "chat_link_preview_enlarge" }),
                )
                .system_image(if is_large {
                    "arrow.down.right.and.arrow.up.left"
                } else {
                    "arrow.up.left.and.arrow.down.right"
                })
                .dismisses_menu(false),
            );
        }
        result.push(
            HoldContextMenuAction::new("apply", tr("chat_link_preview_apply"))
                .system_image("checkmark.circle")
                .section_break(true),
        );
        result.push(
            HoldContextMenuAction::new("remove", tr("chat_link_preview_remove"))
                .system_image("trash")
                .destructive(true),
        );
        result
    }

    fn select(draft: &Entity<ChatLinkPreviewDraft>, action: &str, cx: &mut App) {
        match action {
            "position" => draft.update(cx, |draft, cx| {
                let next = !draft.is_above_text();
                draft.set_is_above_text(next, cx);
            }),
            "size" => draft.update(cx, |draft, cx| {
                let next = !draft.is_large();
                draft.set_is_large(next, cx);
            }),
            "remove" => draft.update(cx, |draft, cx| draft.suppress(cx)),
            _ => {}
        }
    }
}

impl RenderOnce for ChatLinkPreviewSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let is_presented = Binding::entity(
            self.draft.clone(),
            |draft: &ChatLinkPreviewDraft| draft.settings_presented(),
            |draft, value, cx| draft.set_settings_presented(value, cx),
        );
        let actions_draft = self.draft.clone();
        let actions: HoldContextMenuPresentationActions =
            Rc::new(move |cx: &App| Self::actions(actions_draft.read(cx)));
        let select_draft = self.draft.clone();
        let on_action: HoldContextMenuPresentationOnAction =
            Rc::new(move |action: String, _window: &mut Window, cx: &mut App| Self::select(&select_draft, &action, cx));

        let preview_draft = self.draft;
        let id = self.id.clone();
        let text = self.text;
        let entities = self.entities;
        let chat = self.chat;
        let backdrop = self.backdrop;
        let preview: HoldContextMenuPresentationPreview =
            Rc::new(move |width: f32, window: &mut Window, cx: &mut App| -> AnyElement {
                let message =
                    Self::preview_message(preview_draft.read(cx), &chat, &text, entities.as_ref());
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(20.0))
                    .p(px(12.0))
                    .clip_shape(Shape::rounded_rect(24.0))
                    .child(div().absolute().inset_0().child(backdrop(window, cx)))
                    .child(
                        div().w_full().flex().justify_center().child(
                            div()
                                .relative()
                                .px(px(12.0))
                                .py(px(6.0))
                                .rounded_full()
                                .border_1()
                                .border_color(white().opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32))
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .child(HoldContextMenuGlass::new(child_id(&id, "title-glass")).corner_radius(100.0)),
                                )
                                .child(
                                    MainText::new(tr("chat_link_preview_settings"))
                                        .px(13.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(white()),
                                ),
                        ),
                    )
                    .child(
                        env_scope::<ChatMessageViewportWidthKey>((width - 24.0).max(1.0)).child(
                            div()
                                .relative()
                                .w_full()
                                .flex()
                                .justify_end()
                                .child(
                                    MessageBubble::new(child_id(&id, "bubble"), message, chat.clone())
                                        .is_own(true)
                                        .interactions_enabled(false),
                                )
                                .child(div().absolute().inset_0().occlude()),
                        ),
                    )
                    .into_any_element()
            });

        div().size_full().hold_context_menu(
            child_id(&self.id, "hold-menu"),
            is_presented,
            HOLD_CONTEXT_MENU_DEFAULT_MAXIMUM_PREVIEW_WIDTH,
            actions,
            on_action,
            preview,
        )
    }
}
