use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::cell::{Cell, RefCell};

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, App, Bounds, BoxShadow, Context, CursorStyle, DispatchPhase, ElementId, Entity, EntityId,
    FocusHandle, Hitbox, HitboxBehavior, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder,
    Pixels, Point, ScrollDelta, ScrollWheelEvent, SharedString, TouchPhase, WeakEntity, Window, canvas, div, fill,
    hsla, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::ns_range::{byte_to_utf16_offset, utf16_to_byte_offset};
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::{
    HoldContextMenuMetrics,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{
    HoldContextMenuAccessory, HoldContextMenuAction, HoldContextMenuAlignment, HoldSourceBubbleClip,
};
use rise_ui::input_ui::input_actions;
use rise_ui::message_bubble_geometry::{
    MessageBubbleDescriptor, MessageBubbleGeometry, MessageBubblePath, MessageBubbleRadii, MessageBubbleTailSide,
};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::text_selection::{TextSelection, mark_selectable_text_press};
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::view_screen_snapshot::ViewScreenSource;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuActionHandler, HoldContextMenuEmojiHandler, HoldContextMenuPresenter, HoldContextMenuRequest,
    HoldContextMenuRequestID, HoldContextMenuSourceLift,
};
use crate::core::ui::hold_context_menu_ui::hold_gesture_plate::HoldGesturePlate;
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::async_text_bubble_node::AsyncTextBubbleView;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleCorners, AsyncTextBubbleFill, AsyncTextBubblePreparedLayout, AsyncTextBubbleVisualStyle,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_list_jump_debug::RiseListJumpDebug;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListReusableView, RiseManualListSelectionPresentable,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_vertical_scroll_gesture_gate::cancel_enclosing_scroll_pan_for_current_touch;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::{BubbleTapRouter, BubbleTapView};
use crate::modules::chat::components::chat::message_read_receipt_menu::MessageReadReceiptMenuAccessory;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageDeliveryStatus};

const REPLY_HINT_SYMBOL: &str = "arrowshape.turn.up.left.fill";
const REPLY_HINT_SIZE: f32 = 18.0;
const REPLY_MAXIMUM_PULL: f32 = 80.0;
const REPLY_ARMED_PULL: f32 = 56.0;
const REPLY_DAMPING_PAST_ARMED: f32 = 0.35;
const REPLY_RETURN_DURATION: f64 = 0.3;
const REPLY_RETURN_DAMPING: f64 = 0.78;
const BUBBLE_SHADOW_RADIUS: f32 = 1.0;
const BUBBLE_SHADOW_OFFSET_Y: f32 = 1.0;
const OWN_SHADOW_OPACITY: f32 = 0.07;
const INCOMING_SHADOW_OPACITY: f32 = 0.04;
const INCOMING_BORDER_WIDTH: f32 = 0.5;
pub(crate) const OWN_SELECTION_OPACITY: f32 = 0.35;

pub type RiseNativeTextMessageCanDoubleTap = Rc<dyn Fn(&App) -> bool>;
pub type RiseNativeTextMessageDoubleTap = Rc<dyn Fn(&mut Window, &mut App)>;

/// x of a bubble in its row, own bubbles hug the trailing edge so their distance from it is
/// known before layout
/// translation is the sideways transform of the row
fn bubble_horizontal_position<E: Styled>(
    element: E,
    is_own: bool,
    reserves_avatar_space: bool,
    translation: f32,
) -> E {
    if is_own {
        element.right(px(MessageSizes::own_trailing_inset() - translation))
    } else {
        let avatar_space = if reserves_avatar_space { MessageSizes::avatar_column_width() } else { 0.0 };
        element.left(px(MessageSizes::row_side_inset() + avatar_space + translation))
    }
}

fn top_padding_for(is_first_in_group: bool) -> f32 {
    if is_first_in_group {
        MessageSizes::list_first_in_group_top_padding()
    } else {
        MessageSizes::list_continuation_top_padding()
    }
}

pub struct RiseNativeTextMessagePlaceholderView {
    prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
    style: AsyncTextBubbleVisualStyle,
    corners: AsyncTextBubbleCorners,
    is_own: bool,
    reserves_avatar_space: bool,
    top_padding: f32,
}

impl RiseNativeTextMessagePlaceholderView {
    pub fn new(
        prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
        chat: &ChatInfo,
        is_own: bool,
        is_first_in_group: bool,
        is_last_in_group: bool,
        theme: &AppTheme,
    ) -> Self {
        let (style, corners) = RiseNativeTextMessageCell::appearance(theme, is_own, is_first_in_group, is_last_in_group);
        Self {
            prepared_layout,
            style,
            corners,
            is_own,
            reserves_avatar_space: MessageSizes::reserves_avatar_space(chat, is_own),
            top_padding: top_padding_for(is_first_in_group),
        }
    }
}

impl Render for RiseNativeTextMessagePlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let bubble = div().absolute().top(px(self.top_padding)).child(AsyncTextBubbleView {
            prepared_layout: self.prepared_layout.clone(),
            style: self.style.clone(),
            corners: self.corners,
        });
        div().relative().size_full().child(bubble_horizontal_position(
            bubble,
            self.is_own,
            self.reserves_avatar_space,
            0.0,
        ))
    }
}

/// the part of the row the hold menu carries away, so its a view of its own
struct RiseNativeTextMessageBubbleSource {
    prepared_layout: Option<Arc<AsyncTextBubblePreparedLayout>>,
    style: Option<AsyncTextBubbleVisualStyle>,
    corners: Option<AsyncTextBubbleCorners>,
    is_own: bool,
    border_color: Hsla,
    border_width: f32,
    shadow_opacity: f32,
    allows_text_selection: bool,
    can_double_tap: Option<RiseNativeTextMessageCanDoubleTap>,
    selection: Rc<RefCell<TextSelection>>,
    focus_handle: FocusHandle,
}

impl RiseNativeTextMessageBubbleSource {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            prepared_layout: None,
            style: None,
            corners: None,
            is_own: false,
            border_color: hsla(0.0, 0.0, 0.0, 0.0),
            border_width: 0.0,
            shadow_opacity: INCOMING_SHADOW_OPACITY,
            allows_text_selection: false,
            can_double_tap: None,
            selection: Rc::new(RefCell::new(TextSelection::default())),
            focus_handle: cx.focus_handle(),
        }
    }

    fn clear(&mut self) {
        self.prepared_layout = None;
        self.style = None;
        self.corners = None;
        self.can_double_tap = None;
        self.allows_text_selection = false;
        self.selection.borrow_mut().clear();
    }

    fn bubble_path(corners: Option<&AsyncTextBubbleCorners>, rect: Bounds<Pixels>) -> MessageBubblePath {
        let Some(corners) = corners else {
            return MessageBubbleGeometry::path(
                rect,
                MessageBubbleRadii::uniform(px(MessageSizes::bubble_corner_radius())),
                MessageBubbleTailSide::default(),
                false,
            );
        };
        MessageBubbleGeometry::path(
            rect,
            MessageBubbleRadii {
                top_leading: corners.top_left,
                top_trailing: corners.top_right,
                bottom_leading: corners.bottom_left,
                bottom_trailing: corners.bottom_right,
            },
            corners.tail_side,
            corners.draws_tail,
        )
    }

    fn paint_border(
        corners: AsyncTextBubbleCorners,
        border_width: f32,
        border_color: Hsla,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) {
        if border_width <= 0.0 {
            return;
        }
        let path = Self::bubble_path(Some(&corners), bounds);
        let mut builder = PathBuilder::stroke(px(border_width));
        path.replay_into(&mut builder);
        if let Ok(stroke) = builder.build() {
            window.paint_path(stroke, border_color);
        }
    }

    fn install_selection_listeners(
        hitbox: &Hitbox,
        prepared_layout: &Arc<AsyncTextBubblePreparedLayout>,
        origin: &Rc<Cell<Point<Pixels>>>,
        selection: &Rc<RefCell<TextSelection>>,
        focus_handle: &FocusHandle,
        can_double_tap: Option<RiseNativeTextMessageCanDoubleTap>,
        window: &mut Window,
    ) {
        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let layout = prepared_layout.clone();
            let selection = selection.clone();
            let focus = focus_handle.clone();
            let origin = origin.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !hitbox.is_hovered(window) {
                    if selection.borrow_mut().clear() {
                        window.refresh();
                    }
                    return;
                }
                if event.button != MouseButton::Left || HoldContextMenuPresenter::has_active_request(cx) {
                    return;
                }
                mark_selectable_text_press(event.position, cx);
                let click_count =
                    if can_double_tap.as_ref().is_some_and(|can| can(cx)) { 1 } else { event.click_count };
                let index = text_index_at(&layout, event.position - origin.get());
                let mut selection = selection.borrow_mut();
                selection.press(&layout.seed.text, index, click_count, event.modifiers.shift);
                if !selection.is_empty() {
                    window.focus(&focus, cx);
                }
                window.refresh();
            }
        });

        window.on_mouse_event({
            let layout = prepared_layout.clone();
            let selection = selection.clone();
            let focus = focus_handle.clone();
            let origin = origin.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !selection.borrow().is_dragging() {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    selection.borrow_mut().release();
                    return;
                }
                if HoldContextMenuPresenter::has_active_request(cx) {
                    selection.borrow_mut().release();
                    selection.borrow_mut().clear();
                    window.refresh();
                    return;
                }
                let index = text_index_at(&layout, event.position - origin.get());
                let mut selection = selection.borrow_mut();
                if selection.drag_to(&layout.seed.text, index) {
                    if !selection.is_empty() {
                        window.focus(&focus, cx);
                    }
                    window.refresh();
                }
            }
        });

        window.on_mouse_event({
            let selection = selection.clone();
            move |event: &MouseUpEvent, phase, _window, _cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    selection.borrow_mut().release();
                }
            }
        });
    }
}

/// byte index of seed.text closest to local, a point in bubble coordinates
fn text_index_at(layout: &AsyncTextBubblePreparedLayout, local: Point<Pixels>) -> usize {
    let mut closest = None;
    let mut closest_distance = f32::MAX;
    for placement in layout.text_lines.iter() {
        let top = placement.glyph_bounds.origin.y;
        let bottom = placement.glyph_bounds.bottom();
        let distance = if local.y < top {
            f32::from(top - local.y)
        } else if local.y > bottom {
            f32::from(local.y - bottom)
        } else {
            0.0
        };
        if distance < closest_distance {
            closest_distance = distance;
            closest = Some(placement);
        }
    }
    let Some(placement) = closest else {
        return 0;
    };
    let line = &placement.line;
    let target = f32::from(local.x - placement.baseline_origin.x);
    let mut utf16 = line.string_range.location;
    let mut best = utf16;
    let mut best_distance = (f32::from(line.offset_for_string_index(utf16)) - target).abs();
    for character in line.text.chars() {
        utf16 += character.len_utf16();
        let distance = (f32::from(line.offset_for_string_index(utf16)) - target).abs();
        if distance >= best_distance {
            break;
        }
        best = utf16;
        best_distance = distance;
    }
    utf16_to_byte_offset(&layout.seed.text, best).unwrap_or(layout.seed.text.len())
}

fn selection_highlight_rects(layout: &AsyncTextBubblePreparedLayout, range: Range<usize>) -> Vec<Bounds<Pixels>> {
    let start = byte_to_utf16_offset(&layout.seed.text, range.start);
    let end = byte_to_utf16_offset(&layout.seed.text, range.end);
    let mut rects = Vec::new();
    for placement in layout.text_lines.iter() {
        let line = &placement.line;
        let line_start = line.string_range.location;
        let line_end = line_start + line.string_range.length;
        let from = usize::max(start, line_start);
        let to = usize::min(end, line_end);
        if from >= to {
            continue;
        }
        let left = placement.baseline_origin.x + line.offset_for_string_index(from);
        let right = placement.baseline_origin.x + line.offset_for_string_index(to);
        if left >= right {
            continue;
        }
        rects.push(Bounds::from_corners(
            point(left, placement.glyph_bounds.origin.y),
            point(right, placement.glyph_bounds.bottom()),
        ));
    }
    rects
}

impl Render for RiseNativeTextMessageBubbleSource {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (Some(prepared_layout), Some(style), Some(corners)) =
            (self.prepared_layout.clone(), self.style.clone(), self.corners)
        else {
            return div().into_any_element();
        };
        self.selection.borrow_mut().fit(&prepared_layout.seed.text);

        let bubble_size = prepared_layout.size;
        let body = MessageBubbleGeometry::body_rect(Bounds::new(Point::default(), bubble_size), corners.tail_side);
        let shadow = div()
            .absolute()
            .left(body.origin.x)
            .top(body.origin.y)
            .w(body.size.width)
            .h(body.size.height)
            .rounded_tl(corners.top_left)
            .rounded_tr(corners.top_right)
            .rounded_bl(corners.bottom_left)
            .rounded_br(corners.bottom_right)
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, self.shadow_opacity),
                offset: point(px(0.0), px(BUBBLE_SHADOW_OFFSET_Y)),
                blur_radius: px(BUBBLE_SHADOW_RADIUS),
                spread_radius: px(0.0),
                inset: false,
            }]);

        let selection_color = {
            let theme = rise_ui::theme(cx);
            if self.is_own { theme.content_on(theme.primary._100).opacity(OWN_SELECTION_OPACITY) } else { theme.input.selection }
        };
        let overlay = canvas(
            |bounds, window, _cx| {
                let origin = Rc::new(Cell::new(bounds.origin));
                let moved = origin.clone();
                window.on_replayed_move(move |offset, _, _, _| moved.set(bounds.origin + offset));
                (window.insert_hitbox(bounds, HitboxBehavior::Normal), origin)
            },
            {
                let layout = prepared_layout.clone();
                let selection = self.selection.clone();
                let focus_handle = self.focus_handle.clone();
                let can_double_tap = self.can_double_tap.clone();
                let allows_text_selection = self.allows_text_selection;
                let (border_width, border_color) = (self.border_width, self.border_color);
                move |bounds, (hitbox, origin): (Hitbox, Rc<Cell<Point<Pixels>>>), window, _cx| {
                    Self::paint_border(corners, border_width, border_color, bounds, window);
                    if !allows_text_selection {
                        return;
                    }
                    for rect in selection_highlight_rects(&layout, selection.borrow().range()) {
                        window.paint_quad(fill(rect + bounds.origin, selection_color));
                    }
                    window.set_cursor_style(CursorStyle::IBeam, &hitbox);
                    Self::install_selection_listeners(
                        &hitbox,
                        &layout,
                        &origin,
                        &selection,
                        &focus_handle,
                        can_double_tap,
                        window,
                    );
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let copied = self.selection.clone();
        let copied_layout = prepared_layout.clone();
        let selected_all = self.selection.clone();
        let selected_all_layout = prepared_layout.clone();
        div()
            .relative()
            .flex_none()
            .w(bubble_size.width)
            .h(bubble_size.height)
            .track_focus(&self.focus_handle)
            .key_context(input_actions::KEY_CONTEXT)
            .on_action(move |_: &input_actions::Copy, _window, cx| {
                match copied.borrow().selected_text(&copied_layout.seed.text) {
                    Some(selected) => UIPasteboard::general().set_string(selected, cx),
                    None => cx.propagate(),
                }
            })
            .on_action(move |_: &input_actions::SelectAll, window, cx| {
                if selected_all.borrow().is_empty() {
                    cx.propagate();
                    return;
                }
                selected_all.borrow_mut().select_all(&selected_all_layout.seed.text);
                window.refresh();
            })
            .child(shadow)
            .child(AsyncTextBubbleView { prepared_layout, style, corners })
            .child(overlay)
            .into_any_element()
    }
}

struct MenuSeed {
    message: Arc<GetMessageMessage>,
    chat: Arc<ChatInfo>,
    is_own: bool,
    allows_message_selection: bool,
}

#[derive(Clone)]
struct ResolvedMenu {
    reaction_emojis: Vec<String>,
    expanded_reaction_emojis: Option<Vec<String>>,
    shows_reaction_menu: bool,
    allows_custom_emoji_reactions: bool,
    actions: Vec<HoldContextMenuAction>,
    accessory: Option<HoldContextMenuAccessory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplyPanState {
    Idle,
    Possible,
    Active,
}

pub struct RiseNativeTextMessageCell {
    bubble_view: Entity<RiseNativeTextMessageBubbleSource>,
    bubble_probe: ViewAnchorProbe,
    bubble_tap_view: Rc<BubbleTapView>,

    prepared_layout: Option<Arc<AsyncTextBubblePreparedLayout>>,
    visual_style: Option<AsyncTextBubbleVisualStyle>,
    corners: Option<AsyncTextBubbleCorners>,
    is_own: bool,
    reserves_avatar_space: bool,
    top_padding: f32,
    bottom_padding: f32,
    allows_reply: bool,
    interactions_enabled: bool,
    menu_seed: Option<MenuSeed>,
    resolved_menu: Option<ResolvedMenu>,
    on_reaction: Option<HoldContextMenuEmojiHandler>,
    on_action: Option<HoldContextMenuActionHandler>,
    can_double_tap: Option<RiseNativeTextMessageCanDoubleTap>,
    on_double_tap: Option<RiseNativeTextMessageDoubleTap>,
    reply_hint_color: Hsla,

    reply_pan_state: ReplyPanState,
    reply_translation_x: f32,
    reply_armed: bool,
    reply_offset: Animated<f32>,
    reply_hint_alpha: Animated<f32>,
    selection_translation_x: f32,
    press_scale: Animated<f32>,
    bubble_alpha: f32,
    active_hold_request_id: Option<HoldContextMenuRequestID>,
    active_source_lift: Option<Rc<HoldContextMenuSourceLift>>,
}

impl RiseNativeTextMessageCell {
    pub const REUSE_IDENTIFIER: &'static str = "RiseNativeTextMessageCell";

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_selection_presentable::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self {
            bubble_view: cx.new(RiseNativeTextMessageBubbleSource::new),
            bubble_probe: ViewAnchorProbe::new(),
            bubble_tap_view: Rc::new(BubbleTapView::new()),
            prepared_layout: None,
            visual_style: None,
            corners: None,
            is_own: false,
            reserves_avatar_space: false,
            top_padding: 0.0,
            bottom_padding: 0.0,
            allows_reply: false,
            interactions_enabled: false,
            menu_seed: None,
            resolved_menu: None,
            on_reaction: None,
            on_action: None,
            can_double_tap: None,
            on_double_tap: None,
            reply_hint_color: hsla(0.0, 0.0, 0.0, 0.0),
            reply_pan_state: ReplyPanState::Idle,
            reply_translation_x: 0.0,
            reply_armed: false,
            reply_offset: Animated::new(0.0),
            reply_hint_alpha: Animated::new(0.0),
            selection_translation_x: 0.0,
            press_scale: Animated::new(1.0),
            bubble_alpha: 1.0,
            active_hold_request_id: None,
            active_source_lift: None,
        }
    }

    /// part of the row height the list measures, the row draws nothing in it
    pub fn bottom_padding(&self) -> f32 {
        self.bottom_padding
    }

    fn resolve_menu_if_needed(&mut self, cx: &mut App) -> Option<ResolvedMenu> {
        if let Some(resolved_menu) = &self.resolved_menu {
            return Some(resolved_menu.clone());
        }
        let seed = self.menu_seed.as_ref()?;
        let (message, chat, is_own, allows_message_selection) =
            (seed.message.clone(), seed.chat.clone(), seed.is_own, seed.allows_message_selection);
        let reaction_menu =
            ChatServicesStore::shared(cx).read(cx).reaction_config(&chat.id, &chat.allowed_reactions);
        let is_sent = !message.is_pending_outgoing() && message.delivery_status != MessageDeliveryStatus::Failed;
        let permissions = MessagePermissions::new(&message, &chat, is_own);
        let can_react = reaction_menu.enabled
            && !chat.is_community_preview()
            && is_sent
            && permissions.can_add_reactions(cx);
        let resolved = ResolvedMenu {
            reaction_emojis: reaction_menu.quick_bar_emojis(),
            expanded_reaction_emojis: reaction_menu.restricted_emojis.clone(),
            shows_reaction_menu: can_react,
            allows_custom_emoji_reactions: reaction_menu.allows_custom_emoji && can_react,
            actions: permissions.context_menu_actions(allows_message_selection, true, cx),
            accessory: if chat.is_community_preview() {
                None
            } else {
                MessageReadReceiptMenuAccessory::configuration(&message, &chat, is_own, cx)
            },
        };
        self.resolved_menu = Some(resolved.clone());
        Some(resolved)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        message: Arc<GetMessageMessage>,
        chat: Arc<ChatInfo>,
        is_own: bool,
        is_first_in_group: bool,
        is_last_in_group: bool,
        prepared_layout: Arc<AsyncTextBubblePreparedLayout>,
        theme: &AppTheme,
        allows_reply: bool,
        interactions_enabled: bool,
        allows_message_selection: bool,
        on_reaction: HoldContextMenuEmojiHandler,
        on_action: HoldContextMenuActionHandler,
        can_double_tap: RiseNativeTextMessageCanDoubleTap,
        on_double_tap: RiseNativeTextMessageDoubleTap,
        cx: &mut Context<Self>,
    ) {
        self.prepared_layout = Some(prepared_layout.clone());
        self.is_own = is_own;
        self.reserves_avatar_space = MessageSizes::reserves_avatar_space(&chat, is_own);
        self.top_padding = top_padding_for(is_first_in_group);
        self.bottom_padding = if is_last_in_group { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };
        self.allows_reply = allows_reply;
        self.interactions_enabled = interactions_enabled;
        self.on_reaction = Some(on_reaction);
        self.on_action = Some(on_action);
        self.can_double_tap = Some(can_double_tap.clone());
        self.on_double_tap = Some(on_double_tap);

        let allows_text_selection = MessagePermissions::new(&message, &chat, is_own).can_copy();
        self.menu_seed = Some(MenuSeed { message, chat, is_own, allows_message_selection });
        self.resolved_menu = None;

        let (style, corners) = Self::appearance(theme, is_own, is_first_in_group, is_last_in_group);
        self.visual_style = Some(style.clone());
        self.corners = Some(corners);
        self.reply_hint_color = theme.primary._100;

        let (border_color, border_width) = if is_own {
            (hsla(0.0, 0.0, 0.0, 0.0), 0.0)
        } else {
            (theme.border._200.opacity(0.45), INCOMING_BORDER_WIDTH)
        };
        let shadow_opacity = if is_own { OWN_SHADOW_OPACITY } else { INCOMING_SHADOW_OPACITY };
        self.bubble_view.update(cx, |bubble, cx| {
            let changes_message = bubble
                .prepared_layout
                .as_ref()
                .is_none_or(|current| current.seed.id != prepared_layout.seed.id);
            if changes_message {
                bubble.selection.borrow_mut().clear();
            }
            bubble.prepared_layout = Some(prepared_layout);
            bubble.style = Some(style);
            bubble.corners = Some(corners);
            bubble.is_own = is_own;
            bubble.border_color = border_color;
            bubble.border_width = border_width;
            bubble.shadow_opacity = shadow_opacity;
            bubble.allows_text_selection = allows_text_selection;
            bubble.can_double_tap = Some(can_double_tap);
            cx.notify();
        });
        cx.notify();
    }

    fn reply_pan_should_begin(&self, delta: Point<Pixels>, cx: &mut App) -> bool {
        if !self.allows_reply || HoldContextMenuPresenter::has_active_request(cx) {
            return false;
        }
        let (x, y) = (f32::from(delta.x), f32::from(delta.y));
        x < 0.0 && x.abs() > y.abs()
    }

    fn handle_reply_pan(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return;
        };
        if matches!(event.touch_phase, TouchPhase::Started) {
            self.reply_pan_state = ReplyPanState::Possible;
            self.reply_translation_x = 0.0;
        }
        if matches!(event.touch_phase, TouchPhase::Started | TouchPhase::Moved) {
            if self.reply_pan_state == ReplyPanState::Possible
                && (f32::from(delta.x) != 0.0 || f32::from(delta.y) != 0.0)
            {
                if self.reply_pan_should_begin(delta, cx) {
                    self.reply_pan_state = ReplyPanState::Active;
                    cancel_enclosing_scroll_pan_for_current_touch(cx);
                } else {
                    self.reply_pan_state = ReplyPanState::Idle;
                }
            }
            if self.reply_pan_state == ReplyPanState::Active {
                self.reply_translation_x += f32::from(delta.x);
                self.update_reply_pull(cx);
            }
            return;
        }
        if self.reply_pan_state != ReplyPanState::Active {
            self.reply_pan_state = ReplyPanState::Idle;
            return;
        }
        let should_reply = self.reply_armed && matches!(event.touch_phase, TouchPhase::Ended);
        self.reply_pan_state = ReplyPanState::Idle;
        self.reply_armed = false;
        let now = RiseClock::monotonic();
        let reduces_motion = reduce_motion(cx);
        self.reply_offset.set(0.0, spring(REPLY_RETURN_DURATION, REPLY_RETURN_DAMPING), now, reduces_motion);
        self.reply_hint_alpha.set(0.0, spring(REPLY_RETURN_DURATION, REPLY_RETURN_DAMPING), now, reduces_motion);
        cx.notify();
        if should_reply && let Some(on_action) = self.on_action.clone() {
            window.defer(cx, move |window, cx| on_action(ChatMessageAction::REPLY.to_owned(), window, cx));
        }
    }

    fn update_reply_pull(&mut self, cx: &mut Context<Self>) {
        let pull = f32::min(f32::max(0.0, -self.reply_translation_x), REPLY_MAXIMUM_PULL);
        let damped = if pull <= REPLY_ARMED_PULL {
            pull
        } else {
            REPLY_ARMED_PULL + (pull - REPLY_ARMED_PULL) * REPLY_DAMPING_PAST_ARMED
        };
        self.reply_offset.jump(-damped);
        self.reply_hint_alpha.jump(f32::min(1.0, pull / REPLY_ARMED_PULL));
        let next_armed = pull >= REPLY_ARMED_PULL;
        if next_armed != self.reply_armed {
            self.reply_armed = next_armed;
            if next_armed {
                let _ = impact(ImpactStyle::Medium);
            }
        }
        cx.notify();
    }

    fn handle_tap(this: &WeakEntity<Self>, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if HoldContextMenuPresenter::has_active_request(cx) {
            return;
        }
        let Ok((failed_send_action, tap_view)) = this.update(cx, |cell, cx| {
            (cell.failed_send_action(cx), cell.bubble_tap_view.clone())
        }) else {
            return;
        };
        if let Some(on_action) = failed_send_action {
            on_action(ChatMessageAction::FAILED_SEND_OPTIONS.to_owned(), window, cx);
            return;
        }
        BubbleTapRouter::dispatch(location, Some(&tap_view), window, cx);
    }

    fn failed_send_action(&self, cx: &mut App) -> Option<HoldContextMenuActionHandler> {
        if !self.interactions_enabled {
            return None;
        }
        let seed = self.menu_seed.as_ref()?;
        let is_current_user = self.is_own
            || AuthServicesStore::shared(cx).read(cx).is_current_user(&seed.message.sender_id);
        if !is_current_user
            || seed.chat.is_community_preview()
            || seed.message.delivery_status != MessageDeliveryStatus::Failed
        {
            return None;
        }
        self.on_action.clone()
    }

    fn begin_hold_press(&mut self, cx: &mut Context<Self>) {
        if !self.interactions_enabled
            || self.active_hold_request_id.is_some()
            || HoldContextMenuPresenter::has_active_request(cx)
        {
            return;
        }
        self.resolved_menu = None;
        if let Some(accessory) = self.resolve_menu_if_needed(cx).and_then(|menu| menu.accessory) {
            (accessory.on_prefetch)(cx);
        }
    }

    fn release_hold_press(&mut self, cx: &mut Context<Self>) {
        if *self.press_scale.target() == 1.0 {
            return;
        }
        let reduces_motion = reduce_motion(cx);
        self.press_scale.set(
            1.0,
            spring(HoldContextMenuMetrics::PRESS_RELEASE_RESPONSE, HoldContextMenuMetrics::PRESS_RELEASE_DAMPING),
            RiseClock::monotonic(),
            reduces_motion,
        );
        cx.notify();
    }

    fn make_hold_request(
        &mut self,
        this: &WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(HoldContextMenuRequest, ViewScreenSource)> {
        let source = ViewScreenSource::new(self.bubble_view.clone(), self.bubble_probe.clone());
        let source_frame = source.window_frame();
        let ready = self.interactions_enabled && !HoldContextMenuPresenter::has_active_request(cx);
        let (Some(prepared_layout), Some(visual_style), Some(corners), Some(source_frame), true) = (
            self.prepared_layout.clone(),
            self.visual_style.clone(),
            self.corners,
            source_frame,
            ready,
        ) else {
            self.release_hold_press(cx);
            return None;
        };
        self.resolved_menu = None;
        let full_width = f32::from(source_frame.size.width);
        let full_height = f32::from(source_frame.size.height);
        if full_width <= 1.0 || full_height <= 1.0 {
            self.release_hold_press(cx);
            return None;
        }
        let initial_lift_scale = 1.0;
        let id = HoldContextMenuRequestID::new();
        let source_lift: Option<Rc<HoldContextMenuSourceLift>> = None;
        let initial_source_center = source_frame.center();
        let snapshot = None;

        let menu = self.resolve_menu_if_needed(cx);
        let clipped_emojis = Self::emojis_fitting_screen(
            menu.as_ref().map_or(&[][..], |menu| menu.reaction_emojis.as_slice()),
            window,
        );
        let reaction = self.on_reaction.clone();
        let action = self.on_action.clone();
        self.active_hold_request_id = Some(id);
        self.active_source_lift = source_lift.clone();

        let restored = this.clone();
        let restored_lift = source_lift.clone();
        let request = HoldContextMenuRequest {
            id,
            source_frame,
            initial_source_center,
            alignment: if self.is_own { HoldContextMenuAlignment::Trailing } else { HoldContextMenuAlignment::Leading },
            show_emoji_list: menu.as_ref().is_some_and(|menu| menu.shows_reaction_menu),
            emojis: clipped_emojis,
            allows_custom_emoji: menu.as_ref().is_some_and(|menu| menu.allows_custom_emoji_reactions),
            expanded_emojis: menu.as_ref().and_then(|menu| menu.expanded_reaction_emojis.clone()),
            actions: menu.as_ref().map(|menu| menu.actions.clone()).unwrap_or_default(),
            accessory: menu.as_ref().and_then(|menu| menu.accessory.clone()),
            on_emoji: Rc::new(move |emoji: String, window: &mut Window, cx: &mut App| {
                if let Some(reaction) = &reaction {
                    reaction(emoji, window, cx);
                }
            }),
            on_action: Rc::new(move |picked: String, window: &mut Window, cx: &mut App| {
                if let Some(action) = &action {
                    action(picked, window, cx);
                }
            }),
            on_lift_source: Rc::new(|_: &mut App| {}),
            on_restore_source: Rc::new(move |cx: &mut App| {
                if let Some(lift) = restored_lift.as_ref() {
                    lift.restore(cx);
                }
                restored.update(cx, |cell, cx| cell.restore_source(id, cx)).ok();
            }),
            source_lift,
            snapshot,
            source_bubble_clip: Some(HoldSourceBubbleClip::with_tail(
                corners.top_left,
                corners.top_right,
                corners.bottom_left,
                corners.bottom_right,
                corners.tail_side,
                corners.draws_tail,
            )),
            initial_lift_scale,
            uses_flat_chrome: false,
            content: Rc::new(move |_: &mut Window, _: &mut App| -> AnyElement {
                AsyncTextBubbleView {
                    prepared_layout: prepared_layout.clone(),
                    style: visual_style.clone(),
                    corners,
                }
                .into_any_element()
            }),
            is_centered: false,
            live_actions: None,
            on_dismiss: Rc::new(|_: &mut App| {}),
            controller_preview: None,
            prefers_dark_chrome: false,
            keeps_source_editing: false,
            max_visible_rows: None,
        };
        Some((request, source))
    }

    fn handle_hold(this: &WeakEntity<Self>, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let prepared = this.update(cx, |cell, cx| cell.make_hold_request(this, window, cx)).ok().flatten();
        if let Some((request, source)) = prepared {
            HoldContextMenuPresenter::present(request, Some(&source), Some(location), window, cx);
        }
    }

    fn restore_source(&mut self, request_id: HoldContextMenuRequestID, cx: &mut Context<Self>) {
        if self.active_hold_request_id != Some(request_id) {
            return;
        }
        self.active_hold_request_id = None;
        self.active_source_lift = None;
        if self.bubble_alpha == 1.0 && *self.press_scale.target() == 1.0 {
            return;
        }
        self.press_scale.jump(1.0);
        self.bubble_alpha = 1.0;
        cx.notify();
    }

    fn emojis_fitting_screen(list: &[String], window: &Window) -> Vec<String> {
        let available = f32::from(window.viewport_size().width)
            - HoldContextMenuMetrics::EDGE_INSET * 2.0
            - HoldContextMenuMetrics::EMOJI_BAR_HORIZONTAL_PADDING * 2.0
            - HoldContextMenuMetrics::EMOJI_SLOT_WIDTH;
        let maximum = usize::max(1, f32::max(0.0, available / HoldContextMenuMetrics::EMOJI_SLOT_WIDTH) as usize);
        list.iter().take(maximum).cloned().collect()
    }

    pub fn appearance(
        theme: &AppTheme,
        is_own: bool,
        is_first_in_group: bool,
        is_last_in_group: bool,
    ) -> (AsyncTextBubbleVisualStyle, AsyncTextBubbleCorners) {
        let fill = if is_own {
            AsyncTextBubbleFill::LinearGradient {
                start: theme.primary._100.opacity(0.96),
                end: theme.primary._200.opacity(0.92),
                start_point: point(px(0.0), px(0.0)),
                end_point: point(px(1.0), px(1.0)),
            }
        } else {
            AsyncTextBubbleFill::Solid(theme.bg._300.opacity(0.96))
        };
        let text_color = if is_own { theme.content_on(theme.primary._100).opacity(0.98) } else { theme.text.primary };
        let meta_color = if is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary };
        let style = AsyncTextBubbleVisualStyle::new(
            fill,
            text_color,
            meta_color,
            meta_color,
            if is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 },
            theme.semantic.error_100,
        );

        let descriptor = MessageBubbleDescriptor::grouped(
            is_own,
            is_first_in_group,
            is_last_in_group,
            px(MessageSizes::bubble_corner_radius()),
            px(MessageSizes::bubble_stacked_corner_radius()),
        );
        let corners = AsyncTextBubbleCorners::new(
            descriptor.radii.top_leading,
            descriptor.radii.top_trailing,
            descriptor.radii.bottom_leading,
            descriptor.radii.bottom_trailing,
            descriptor.tail_side,
            descriptor.draws_tail,
        );
        (style, corners)
    }

    fn gesture_plate(&self, id: ElementId, cx: &mut Context<Self>) -> HoldGesturePlate {
        let this = cx.weak_entity();
        let anticipated = this.clone();
        let confirmed = this.clone();
        let released = this.clone();
        let cancelled = this.clone();
        let tapped = this.clone();
        let can_double_tap = self.can_double_tap.clone();
        let double_tap_handler = self.on_double_tap.clone();
        HoldGesturePlate::new((id, "plate"))
            .context_click_across_row(true)
            .on_anticipate(Rc::new(move |_: &ViewAnchorProbe, _: &mut Window, cx: &mut App| {
                anticipated.update(cx, |cell, cx| cell.begin_hold_press(cx)).ok();
            }))
            .on_confirm(Rc::new(
                move |_: &ViewAnchorProbe, location: Point<Pixels>, window: &mut Window, cx: &mut App| {
                    Self::handle_hold(&confirmed, location, window, cx);
                },
            ))
            .on_update(Rc::new(|location: Point<Pixels>, _: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::update_drag(location, cx);
                }
            }))
            .on_release(Rc::new(move |location: Option<Point<Pixels>>, _: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::release_at(location, cx);
                } else {
                    released.update(cx, |cell, cx| cell.release_hold_press(cx)).ok();
                }
            }))
            .on_cancel(Rc::new(move |_: &mut Window, cx: &mut App| {
                if HoldContextMenuPresenter::has_active_request(cx) {
                    HoldContextMenuPresenter::release_at(None, cx);
                } else {
                    cancelled.update(cx, |cell, cx| cell.release_hold_press(cx)).ok();
                }
            }))
            .on_tap(Rc::new(move |location: Point<Pixels>, window: &mut Window, cx: &mut App| {
                Self::handle_tap(&tapped, location, window, cx);
            }))
            .can_double_tap(Rc::new(move |cx: &App| can_double_tap.as_ref().is_some_and(|can| can(cx))))
            .on_double_tap(double_tap_handler.map(|on_double_tap| {
                let handler: Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)> =
                    Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| on_double_tap(window, cx));
                handler
            }))
    }
}

impl RiseManualListReusableView for RiseNativeTextMessageCell {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        let active_hold_request_id = self.active_hold_request_id.take();
        let active_source_lift = self.active_source_lift.take();
        if active_hold_request_id.is_some() || active_source_lift.is_some() {
            cx.defer(move |cx| {
                if let Some(request_id) = active_hold_request_id {
                    HoldContextMenuPresenter::dismiss_if_active(request_id, cx);
                }
                if let Some(lift) = active_source_lift {
                    lift.restore(cx);
                }
            });
        }
        self.prepared_layout = None;
        self.visual_style = None;
        self.corners = None;
        self.reserves_avatar_space = false;
        self.on_reaction = None;
        self.on_action = None;
        self.can_double_tap = None;
        self.on_double_tap = None;
        self.menu_seed = None;
        self.resolved_menu = None;
        self.reply_armed = false;
        self.reply_pan_state = ReplyPanState::Idle;
        self.reply_translation_x = 0.0;
        self.selection_translation_x = 0.0;
        self.reply_offset.jump(0.0);
        self.reply_hint_alpha.jump(0.0);
        self.press_scale.jump(1.0);
        self.bubble_alpha = 1.0;
        self.bubble_view.update(cx, |bubble, cx| {
            bubble.clear();
            cx.notify();
        });
        cx.notify();
    }
}

impl RiseManualListSelectionPresentable for RiseNativeTextMessageCell {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut Context<Self>,
    ) {
        let translation_x = if is_active { content_translation_x } else { 0.0 };
        let now = RiseClock::monotonic();
        let settled = |value: &Animated<f32>| !value.is_running(now) && *value.target() == 0.0;
        if translation_x == self.selection_translation_x
            && !self.reply_armed
            && self.reply_pan_state == ReplyPanState::Idle
            && settled(&self.reply_offset)
            && settled(&self.reply_hint_alpha)
        {
            return;
        }
        self.selection_translation_x = translation_x;
        self.reply_armed = false;
        self.reply_pan_state = ReplyPanState::Idle;
        self.reply_offset.jump(0.0);
        self.reply_hint_alpha.jump(0.0);
        cx.notify();
    }
}

impl Render for RiseNativeTextMessageCell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(prepared_layout) = self.prepared_layout.clone() else {
            return div().size_full().into_any_element();
        };
        let now = RiseClock::monotonic();
        let translation = self.selection_translation_x + self.reply_offset.frame_value(now, window);
        let press_scale = self.press_scale.frame_value(now, window);
        let reply_hint_alpha = self.reply_hint_alpha.frame_value(now, window);
        let bubble_size = prepared_layout.size;
        let bubble_height = f32::from(bubble_size.height);
        let top_padding = self.top_padding;
        let row_id = ElementId::from(SharedString::from(prepared_layout.seed.id.clone()));

        let bubble_any: AnyView = self.bubble_view.clone().into();
        let bubble_content = HoldContextMenuSourceLift::lifted_placeholder(&bubble_any)
            .unwrap_or_else(|| bubble_any.clone().into_any_element());
        let bubble_slot = div()
            .absolute()
            .top(px(top_padding))
            .w(bubble_size.width)
            .h(bubble_size.height)
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(bubble_content)
                    .view_anchor_probe(&self.bubble_probe)
                    .opacity(self.bubble_alpha)
                    .scale_effect(press_scale),
            );

        let tap_view = self.bubble_tap_view.clone();
        let tap_row = row_id.clone();
        let gesture_slot = div()
            .absolute()
            .top(px(top_padding))
            .w(bubble_size.width)
            .h(bubble_size.height)
            .child(
                canvas(
                    move |bounds, window, _cx| tap_view.report_frame(bounds, Some(tap_row), window),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(self.gesture_plate(row_id, cx));

        let entity_id: EntityId = cx.entity_id();
        let is_own = self.is_own;
        let layout_recorder = canvas(
            move |bounds, _window, cx| {
                RiseListJumpDebug::record_bubble_layout(
                    entity_id,
                    top_padding + bubble_height * 0.5,
                    top_padding,
                    bubble_height,
                    f32::from(bounds.size.height),
                    is_own,
                    cx,
                );
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let side_tap_view = self.bubble_tap_view.clone();
        let side_can_double_tap = self.can_double_tap.clone();
        let side_double_tap = self.on_double_tap.clone();

        div()
            .id(("rise-native-text-message-cell", entity_id.as_u64()))
            .relative()
            .size_full()
            .on_scroll_wheel(cx.listener(Self::handle_reply_pan))
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                if event.click_count < 2 || side_tap_view.bounds().is_some_and(|bounds| bounds.contains(&event.position)) {
                    return;
                }
                let (Some(can_double_tap), Some(on_double_tap)) = (&side_can_double_tap, &side_double_tap) else {
                    return;
                };
                if can_double_tap(cx) && !HoldContextMenuPresenter::has_active_request(cx) {
                    on_double_tap(window, cx);
                }
            })
            .child(layout_recorder)
            .child(
                div()
                    .absolute()
                    .right(px(MessageSizes::row_side_inset()))
                    .top(px(top_padding + (bubble_height - REPLY_HINT_SIZE) * 0.5))
                    .size(px(REPLY_HINT_SIZE))
                    .opacity(reply_hint_alpha)
                    .children(IconUi::sized(REPLY_HINT_SYMBOL, px(REPLY_HINT_SIZE), self.reply_hint_color, true)),
            )
            .child(bubble_horizontal_position(bubble_slot, is_own, self.reserves_avatar_space, translation))
            .child(bubble_horizontal_position(gesture_slot, is_own, self.reserves_avatar_space, translation))
            .into_any_element()
    }
}
