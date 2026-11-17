use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AccessibleAction, AnyElement, App, Background, BoxShadow, Context, Hsla, PathBuilder, Pixels, Point, Role, SharedString,
    Window, black, canvas, div, linear_color_stop, linear_gradient, point, px, size,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::message_bubble_geometry::{MessageBubbleDescriptor, MessageBubbleGeometry, MessageBubbleShape};

use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuContainer, HoldContextMenuStringHandler,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{HoldContextMenuPresenter, HoldContextMenuRequestID};
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListReusableView, RiseManualListSelectionPresentable,
};
use crate::modules::chat::components::chat::bubble::message_call_view::{
    MessageCallAccent, MessageCallActionRouter, MessageCallBubbleGeometry, MessageCallPresentationFactory,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// one call row of the message list
/// press squish and lift and right click belong to HoldContextMenuContainer
pub struct RiseNativeCallMessageCell {
    message: Option<Arc<GetMessageMessage>>,
    chat: Option<Rc<ChatInfo>>,
    content: Option<Rc<RiseNativeCallMessageCellContent>>,
    is_own: bool,
    reserves_avatar_space: bool,
    top_padding: f32,
    interactions_enabled: bool,
    menu_actions: Rc<Vec<HoldContextMenuAction>>,
    on_action: Option<HoldContextMenuStringHandler>,
    selection_translation_x: f32,
    active_hold_request_id: Rc<Cell<Option<HoldContextMenuRequestID>>>,
}

impl RiseNativeCallMessageCell {
    pub const REUSE_IDENTIFIER: &'static str = "RiseNativeCallMessageCell";

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_selection_presentable::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self {
            message: None,
            chat: None,
            content: None,
            is_own: false,
            reserves_avatar_space: false,
            top_padding: 0.0,
            interactions_enabled: false,
            menu_actions: Rc::new(Vec::new()),
            on_action: None,
            selection_translation_x: 0.0,
            active_hold_request_id: Rc::default(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        message: Arc<GetMessageMessage>,
        chat: &ChatInfo,
        is_own: bool,
        is_first_in_group: bool,
        is_last_in_group: bool,
        viewport_width: f32,
        theme: &AppTheme,
        interactions_enabled: bool,
        allows_message_selection: bool,
        on_action: HoldContextMenuStringHandler,
        cx: &mut Context<Self>,
    ) {
        let Some(presentation) = MessageCallPresentationFactory::make(&message, is_own, cx) else { return };

        let reserves_avatar_space = MessageSizes::reserves_avatar_space(chat, is_own);
        let top_padding = if is_first_in_group {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let layout_unchanged = self.is_own == is_own
            && self.reserves_avatar_space == reserves_avatar_space
            && self.top_padding == top_padding
            && self.interactions_enabled == interactions_enabled
            && self.message.as_ref().is_some_and(|current| Arc::ptr_eq(current, &message) || **current == *message);
        self.chat = Some(Rc::new(chat.clone()));
        self.is_own = is_own;
        self.reserves_avatar_space = reserves_avatar_space;
        self.top_padding = top_padding;
        self.interactions_enabled = interactions_enabled;
        self.on_action = Some(on_action);
        let menu_actions =
            MessagePermissions::new(&message, chat, is_own).context_menu_actions(allows_message_selection, true, cx);
        let layout_unchanged = layout_unchanged && *self.menu_actions == menu_actions;
        self.menu_actions = Rc::new(menu_actions);

        let direction_color =
            if is_own { theme.content_on(theme.primary._100) } else { Self::accent_color(presentation.accent, theme) };
        let start_color =
            if is_own { theme.primary._100.opacity(0.96) } else { theme.bg._300.opacity(0.96) };
        let end_color = if is_own { theme.primary._200.opacity(0.92) } else { start_color };

        let content = RiseNativeCallMessageCellContent {
            accessibility_label: SharedString::from(format!("{}, {}", presentation.title, presentation.detail)),
            direction_system_image: presentation.direction_system_image(),
            media_system_image: presentation.media_system_image(),
            bubble_width: MessageCallBubbleGeometry::bubble_width(viewport_width, chat, is_own, &presentation, cx),
            bubble_height: MessageCallBubbleGeometry::BUBBLE_HEIGHT,
            descriptor: MessageBubbleDescriptor::grouped(
                is_own,
                is_first_in_group,
                is_last_in_group,
                px(MessageSizes::bubble_corner_radius()),
                px(MessageSizes::bubble_stacked_corner_radius()),
            ),
            title_color: if is_own { theme.content_on(theme.primary._100) } else { theme.text.primary },
            detail_color: if is_own { theme.content_on(theme.primary._100).opacity(0.7) } else { theme.text.secondary },
            direction_color,
            media_color: if is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 },
            start_color,
            end_color,
            border_color: (!is_own).then(|| theme.border._200.opacity(0.45)),
            shadow_opacity: if is_own { 0.07 } else { 0.04 },
            title: SharedString::from(presentation.title),
            detail: SharedString::from(presentation.detail),
        };
        if layout_unchanged && self.content.as_deref() == Some(&content) {
            return;
        }
        self.content = Some(Rc::new(content));
        self.message = Some(message);
        cx.notify();
    }

    fn accent_color(accent: MessageCallAccent, theme: &AppTheme) -> Hsla {
        match accent {
            MessageCallAccent::Success => theme.semantic.success_100,
            MessageCallAccent::Error => theme.semantic.error_100,
            MessageCallAccent::Neutral => theme.primary._100,
        }
    }
}

impl RiseManualListReusableView for RiseNativeCallMessageCell {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        if let Some(request_id) = self.active_hold_request_id.take() {
            cx.defer(move |cx| HoldContextMenuPresenter::dismiss_if_active(request_id, cx));
        }
        self.message = None;
        self.chat = None;
        self.content = None;
        self.menu_actions = Rc::new(Vec::new());
        self.on_action = None;
        self.interactions_enabled = false;
        self.selection_translation_x = 0.0;
        cx.notify();
    }
}

impl RiseManualListSelectionPresentable for RiseNativeCallMessageCell {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut Context<Self>,
    ) {
        let translation_x = if is_active { content_translation_x } else { 0.0 };
        if translation_x == self.selection_translation_x {
            return;
        }
        self.selection_translation_x = translation_x;
        cx.notify();
    }
}

impl Render for RiseNativeCallMessageCell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div().relative().size_full();
        let (Some(message), Some(_), Some(content)) = (&self.message, &self.chat, &self.content) else {
            return row;
        };

        let leading_inset = MessageSizes::row_side_inset()
            + if self.reserves_avatar_space { MessageSizes::avatar_column_width() } else { 0.0 };
        let trailing_inset = MessageSizes::own_trailing_inset();

        let call_again: Rc<dyn Fn(&mut App)> = {
            let this = cx.weak_entity();
            Rc::new(move |cx: &mut App| {
                let Some((message, chat)) = this.upgrade().and_then(|cell| {
                    let cell = cell.read(cx);
                    Some((cell.message.clone()?, cell.chat.clone()?))
                }) else {
                    return;
                };
                MessageCallActionRouter::call_again(&message, &chat, cx);
            })
        };

        let actions = self.menu_actions.clone();
        let source = content.clone();
        let on_action = self.on_action.clone();
        let tap = call_again.clone();
        let interactions_enabled = self.interactions_enabled;
        let descriptor = content.descriptor;

        let container = HoldContextMenuContainer::new(
            SharedString::from(format!("rise-native-call-cell-hold-{}", message.id)),
            if self.is_own { HoldContextMenuAlignment::Trailing } else { HoldContextMenuAlignment::Leading },
            Rc::new(move |_: &App| (*actions).clone()),
            Rc::new(move |_: &mut Window, _: &mut App| source.element()),
        )
        .show_emoji_list(false)
        .emojis(Vec::new())
        .disabled(!interactions_enabled)
        .source_identity(Some(message.id.clone()))
        .active_request_cell(self.active_hold_request_id.clone())
        .source_bubble_clip(Some(HoldSourceBubbleClip::with_tail(
            descriptor.radii.top_leading,
            descriptor.radii.top_trailing,
            descriptor.radii.bottom_leading,
            descriptor.radii.bottom_trailing,
            descriptor.tail_side,
            descriptor.draws_tail,
        )))
        .on_action(Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
            if let Some(on_action) = &on_action {
                on_action(action, window, cx);
            }
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            if interactions_enabled {
                tap(cx);
            }
        }));

        let slot = div()
            .id(SharedString::from(format!("rise-native-call-cell-{}", message.id)))
            .absolute()
            .top(px(self.top_padding))
            .w(px(content.bubble_width))
            .h(px(content.bubble_height))
            .role(Role::Button)
            .aria_label(content.accessibility_label.clone())
            .aria_description(tr("chat_call_again"))
            .when(interactions_enabled, |slot| {
                slot.on_a11y_action(AccessibleAction::Click, move |_, _, cx| call_again(cx))
            });
        let slot = if self.is_own {
            slot.right(px(trailing_inset - self.selection_translation_x))
        } else {
            slot.left(px(leading_inset + self.selection_translation_x))
        };

        row.child(slot.child(container))
    }
}

/// what the bubble draws, apart from the cell because the menu host draws the same source
/// again while lifted
#[derive(PartialEq)]
struct RiseNativeCallMessageCellContent {
    title: SharedString,
    detail: SharedString,
    accessibility_label: SharedString,
    direction_system_image: &'static str,
    media_system_image: &'static str,
    bubble_width: f32,
    bubble_height: f32,
    descriptor: MessageBubbleDescriptor,
    title_color: Hsla,
    detail_color: Hsla,
    direction_color: Hsla,
    media_color: Hsla,
    start_color: Hsla,
    end_color: Hsla,
    border_color: Option<Hsla>,
    shadow_opacity: f32,
}

impl RiseNativeCallMessageCellContent {
    fn element(&self) -> AnyElement {
        const MEDIA_WIDTH: f32 = 34.0;
        const MEDIA_HEIGHT: f32 = 38.0;
        const DIRECTION_SIDE: f32 = 13.0;

        let bubble_bounds =
            gpui::Bounds::new(point(px(0.0), px(0.0)), size(px(self.bubble_width), px(self.bubble_height)));
        let body = MessageBubbleGeometry::body_rect(bubble_bounds, self.descriptor.tail_side);
        let body_min_x = f32::from(body.left());
        let body_max_x = f32::from(body.right());

        let content_leading = body_min_x + 13.0;
        let media_x = body_max_x - MEDIA_WIDTH - 12.0;
        let direction_max_x = content_leading + DIRECTION_SIDE;

        let radii = self.descriptor.radii;
        let shape = MessageBubbleShape {
            radii,
            tail_side: self.descriptor.tail_side,
            draws_tail: self.descriptor.draws_tail,
        };
        let (start_color, end_color, border_color) = (self.start_color, self.end_color, self.border_color);

        let shadow = div()
            .absolute()
            .left(body.left())
            .top(body.top())
            .w(body.size.width)
            .h(body.size.height)
            .rounded_tl(radii.top_leading)
            .rounded_tr(radii.top_trailing)
            .rounded_bl(radii.bottom_leading)
            .rounded_br(radii.bottom_trailing)
            .shadow(vec![BoxShadow {
                color: black().opacity(self.shadow_opacity),
                offset: point(px(0.0), px(1.0)),
                blur_radius: px(1.0),
                spread_radius: px(0.0),
                inset: false,
            }]);

        let fill = canvas(
            |_, _, _| {},
            move |bounds, _, window: &mut Window, _| {
                let dx = f32::from(bounds.size.width);
                let dy = f32::from(bounds.size.height);
                let angle = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
                let background: Background =
                    linear_gradient(angle, linear_color_stop(start_color, 0.0), linear_color_stop(end_color, 1.0));
                shape.paint(bounds, background, window);
                if let Some(border_color) = border_color {
                    let mut builder = PathBuilder::stroke(px(0.5));
                    shape.path(bounds).replay_into(&mut builder);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, border_color);
                    }
                }
            },
        )
        .absolute()
        .size_full();

        div()
            .relative()
            .w(px(self.bubble_width))
            .h(px(self.bubble_height))
            .child(shadow)
            .child(fill)
            .child(
                div()
                    .absolute()
                    .left(px(media_x))
                    .top(px(16.0))
                    .w(px(MEDIA_WIDTH))
                    .h(px(MEDIA_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(self.media_system_image, px(MEDIA_WIDTH), self.media_color, true)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(content_leading))
                    .top(px(9.0))
                    .w(px((media_x - content_leading - 10.0).max(1.0)))
                    .h(px(23.0))
                    .line_height(px(23.0))
                    .text_size(px(MessageCallBubbleGeometry::TITLE_FONT_SIZE))
                    .font_weight(MessageCallBubbleGeometry::TITLE_FONT_WEIGHT)
                    .text_color(self.title_color)
                    .truncate()
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(content_leading))
                    .top(px(41.0))
                    .w(px(DIRECTION_SIDE))
                    .h(px(DIRECTION_SIDE))
                    .children(IconUi::sized(self.direction_system_image, px(DIRECTION_SIDE), self.direction_color, false)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(direction_max_x + 4.0))
                    .top(px(37.0))
                    .w(px((media_x - direction_max_x - 8.0).max(1.0)))
                    .h(px(21.0))
                    .line_height(px(21.0))
                    .text_size(px(MessageCallBubbleGeometry::DETAIL_FONT_SIZE))
                    .font_weight(MessageCallBubbleGeometry::DETAIL_FONT_WEIGHT)
                    .text_color(self.detail_color)
                    .truncate()
                    .child(self.detail.clone()),
            )
            .into_any_element()
    }
}
