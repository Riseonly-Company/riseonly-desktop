use std::rc::Rc;
use gpui::{AnyView, App, Window};
use rise_theme::AppTheme;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::native_bubbles::document::native_document_bubble::NativeDocumentBubble;
use crate::modules::chat::components::chat::native_bubbles::media::native_caption_media_bubble::NativeCaptionMediaBubble;
#[cfg(debug_assertions)]
use crate::modules::chat::components::chat::native_bubbles::native_bubble_row_audit::NativeBubbleRowAudit;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_sticker_bubble::NativeStickerBubble;
use crate::modules::chat::components::chat::native_bubbles::voice::native_video_note_bubble::NativeVideoNoteBubble;
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_bubble::NativeVoiceBubble;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// message shape the feed draws without the regular hosted row
/// list asks each kind in order for an exact row height, none means regular bubble
/// height must equal the regular row, view never measures itself
pub trait NativeBubbleKind: 'static {
    const REUSE_IDENTIFIER: &'static str;

    /// exact row height including the group paddings, or none when this kind doesnt draw the message
    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32>;

    #[allow(clippy::too_many_arguments)]
    fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView;
}

type NativeBubbleRowHeight =
    fn(&GetMessageMessage, ChatMessageRenderEntryGroupFlags, &RiseMessageCollectionConfiguration, f32, &mut App) -> Option<f32>;

type NativeBubbleMakeView = fn(
    Option<AnyView>,
    &GetMessageMessage,
    ChatMessageRenderEntryGroupFlags,
    &Rc<RiseMessageCollectionConfiguration>,
    f32,
    &AppTheme,
    &mut Window,
    &mut App,
) -> AnyView;

/// registered kind behind plain function pointers
/// so the registry stays a static list and a lookup allocates nothing
#[derive(Clone, Copy)]
pub struct NativeBubbleKindRef {
    pub name: &'static str,
    pub reuse_identifier: &'static str,
    row_height: NativeBubbleRowHeight,
    make_view: NativeBubbleMakeView,
}

impl NativeBubbleKindRef {
    pub const fn of<K: NativeBubbleKind>(name: &'static str) -> Self {
        Self { name, reuse_identifier: K::REUSE_IDENTIFIER, row_height: K::row_height, make_view: K::make_view }
    }

    pub fn row_height(
        &self,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        (self.row_height)(message, group, configuration, viewport_width, cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn make_view(
        &self,
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        (self.make_view)(reusing, message, group, configuration, viewport_width, theme, window, cx)
    }
}

/// the kind a lookup picked and the exact row height it answered
#[derive(Clone, Copy)]
pub struct NativeBubbleMatch {
    pub kind: NativeBubbleKindRef,
    pub height: f32,
}

#[cfg(debug_assertions)]
pub type NativeBubbleRowObserver =
    Rc<dyn Fn(&str, &ChatMessageRenderEntry, &RiseMessageCollectionConfiguration, f32, f32)>;

#[cfg(debug_assertions)]
thread_local! {
    static ROW_OBSERVER: std::cell::RefCell<Option<NativeBubbleRowObserver>> = const { std::cell::RefCell::new(None) };
}

pub enum NativeBubbleRegistry {}

impl NativeBubbleRegistry {
    pub const KINDS: &'static [NativeBubbleKindRef] = &[
        NativeBubbleKindRef::of::<NativeCaptionMediaBubble>("NativeCaptionMediaBubble"),
        NativeBubbleKindRef::of::<NativeVoiceBubble>("NativeVoiceBubble"),
        NativeBubbleKindRef::of::<NativeVideoNoteBubble>("NativeVideoNoteBubble"),
        NativeBubbleKindRef::of::<NativeStickerBubble>("NativeStickerBubble"),
        NativeBubbleKindRef::of::<NativeDocumentBubble>("NativeDocumentBubble"),
    ];

    pub fn kinds() -> &'static [NativeBubbleKindRef] {
        Self::KINDS
    }

    pub fn kind_for(
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<NativeBubbleMatch> {
        if entry.kind != ChatMessageRenderEntryKind::Message {
            return None;
        }
        let message = entry.message.as_ref()?;
        let group = entry.group?;
        Self::kinds().iter().find_map(|kind| {
            kind.row_height(message, group, configuration, viewport_width, cx)
                .map(|height| NativeBubbleMatch { kind: *kind, height })
        })
    }

    #[cfg(debug_assertions)]
    pub fn set_row_observer(observer: Option<NativeBubbleRowObserver>) {
        ROW_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
    }

    #[cfg(debug_assertions)]
    pub fn row_observer() -> Option<NativeBubbleRowObserver> {
        ROW_OBSERVER.with(|slot| slot.borrow().clone())
    }

    #[allow(unused_variables)]
    pub fn report_row(
        kind: &str,
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        height: f32,
        cx: &mut App,
    ) {
        #[cfg(debug_assertions)]
        {
            NativeBubbleRowAudit::check(kind, entry, configuration, viewport_width, height, cx);
            if let Some(observer) = Self::row_observer() {
                observer(kind, entry, configuration, viewport_width, height);
            }
        }
    }

    /// row paddings every bubble row shares
    pub fn row_paddings(group: ChatMessageRenderEntryGroupFlags) -> (f32, f32) {
        (
            if group.is_first {
                MessageSizes::list_first_in_group_top_padding()
            } else {
                MessageSizes::list_continuation_top_padding()
            },
            if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 },
        )
    }
}
