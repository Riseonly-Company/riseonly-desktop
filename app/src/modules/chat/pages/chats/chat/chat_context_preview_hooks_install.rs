use gpui::{AnyElement, AnyView, App, AppContext, ElementId, Entity, IntoElement, StyleRefinement, Styled, Window};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreviewSession;
use crate::modules::chat::components::chat_item::chat_context_preview_hooks::{
    ChatContextPreviewHooks, set_chat_context_preview_hooks,
};
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen::ChatScreen;

struct ChatScreenHolder {
    screen: Entity<ChatScreen>,
}

fn held_screen(
    key: String,
    window: &mut Window,
    cx: &mut App,
    build: impl FnOnce(&mut gpui::Context<ChatScreen>) -> ChatScreen + 'static,
) -> AnyElement {
    let holder = window.use_keyed_state(ElementId::Name(key.into()), cx, move |_, cx| ChatScreenHolder {
        screen: cx.new(build),
    });
    AnyView::from(holder.read(cx).screen.clone())
        .cached(StyleRefinement::default().size_full())
        .replay_when_moved()
        .into_any_element()
}

fn chat_screen(
    chat_id: Option<String>,
    initial_all_view: bool,
    navigation_open_epoch: Option<Uuid>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let key = format!("chat-screen-{}-{}", chat_id.as_deref().unwrap_or_default(), initial_all_view);
    held_screen(key, window, cx, move |cx| {
        ChatScreen::new(chat_id, None, initial_all_view, None, navigation_open_epoch, None, cx)
    })
}

fn chat_screen_context_preview(
    chat_id: Option<String>,
    topic_id: Option<String>,
    initial_all_view: bool,
    context_preview: Entity<ChatContextPreviewSession>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let key = format!(
        "chat-screen-preview-{}-{}-{}",
        chat_id.as_deref().unwrap_or_default(),
        topic_id.as_deref().unwrap_or_default(),
        initial_all_view
    );
    held_screen(key, window, cx, move |cx| {
        ChatScreen::new(chat_id, topic_id, initial_all_view, None, None, Some(context_preview), cx)
    })
}

pub(crate) fn install_chat_context_preview_hooks() {
    set_chat_context_preview_hooks(ChatContextPreviewHooks { chat_screen, chat_screen_context_preview });
}
