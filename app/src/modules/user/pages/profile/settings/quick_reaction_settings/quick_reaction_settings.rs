use gpui::{App, Context, Entity, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::theme::app_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageReaction, ReactionReactor,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const PREVIEW_CHAT_ID: &str = "quick-reaction-preview";
const PREVIEW_PEER_ID: &str = "quick-reaction-preview-peer";
const PREVIEW_MESSAGE_ID: &str = "quick-reaction-preview-1";
const PREVIEW_MESSAGE_AGE_SECONDS: f64 = 60.0;
const PREVIEW_HEIGHT: f32 = 176.0;
const EMOJI_PICKER_HEIGHT: f32 = 376.0;
const EMOJI_PICKER_BOTTOM_PADDING: f32 = 8.0;
const DOUBLE_TAP_CLICK_COUNT: usize = 2;

struct QuickReactionSettingsState {
    picker_open: bool,
    preview_reacted: bool,
    picker_motion: Animated<f32>,
    _subscriptions: Vec<Subscription>,
}

impl QuickReactionSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let user_service = UserServicesStore::shared(cx);
        Self {
            picker_open: false,
            preview_reacted: false,
            picker_motion: Animated::new(0.0),
            _subscriptions: vec![
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&user_service, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn set_picker_open(&mut self, picker_open: bool, response: f64, damping_fraction: f64, cx: &mut Context<Self>) {
        self.picker_open = picker_open;
        self.picker_motion.set_in(
            if picker_open { 1.0 } else { 0.0 },
            spring(response, damping_fraction),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }

    fn toggle_quick_reaction(&mut self, cx: &mut Context<Self>) {
        let _ = impact(ImpactStyle::Light);
        self.preview_reacted = !self.preview_reacted;
        cx.notify();
    }

    fn pick(state: &Entity<Self>, reaction: &str, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_quick_reaction(reaction, cx));
        state.update(cx, |state, cx| state.set_picker_open(false, 0.36, 0.88, cx));
    }
}

/// quick reaction preview,
/// the chat row over the shipped wallpaper,
/// double tap works like in a chat
#[derive(IntoElement)]
pub struct QuickReactionSettings;

impl QuickReactionSettings {
    pub fn new() -> Self {
        Self
    }

    fn quick_reaction(profile: Option<&ProfileModel>) -> String {
        EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(profile.map(|profile| profile.quick_reaction.as_str()))
            .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned())
    }

    fn preview_chat() -> ChatInfo {
        let mut chat = ChatInfo::new(PREVIEW_CHAT_ID.to_owned());
        chat.title = tr("settings_quick_reaction_title");
        chat.r#type = ChatType::Direct;
        chat.participant = User { id: PREVIEW_PEER_ID.to_owned(), ..User::default() };
        chat
    }

    fn preview_message(reacted: bool, quick_reaction: &str, profile: Option<&ProfileModel>) -> GetMessageMessage {
        let now = Date::now().time_interval_since_1970();
        let mut message = GetMessageMessage::new(PREVIEW_MESSAGE_ID.to_owned());
        message.sender_id = PREVIEW_PEER_ID.to_owned();
        message.text = tr("settings_quick_reaction_preview_message");
        message.content_type = MessageContentType::Text;
        message.created_at = now - PREVIEW_MESSAGE_AGE_SECONDS;
        message.is_read = true;
        if reacted {
            message.reactions = vec![MessageReaction {
                emoji: quick_reaction.to_owned(),
                count: 1,
                reacted_by_me: true,
                reactors: vec![ReactionReactor {
                    user_id: profile.map(|profile| profile.id.clone()).unwrap_or_default(),
                    name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
                    avatar_url: profile.and_then(|profile| {
                        profile.avatar_url.clone().or_else(|| Some(profile.more.logo.clone()))
                    }),
                    created_at: now,
                }],
            }];
        }
        message
    }

    fn preview_section(
        state: &Entity<QuickReactionSettingsState>,
        theme: &AppTheme,
        message: GetMessageMessage,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let store = theme_store(cx);
        let appearance = store.read(cx).resolved_appearance();
        let toggle_state = state.clone();

        vstack(8.0)
            .w_full()
            .items_start()
            .child(
                div().pl(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).pr(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                    MainText::new(tr("settings_quick_reaction_preview_header").to_uppercase())
                        .px(12.0)
                        .color(theme.text.secondary),
                ),
            )
            .child(
                div().w_full().pl(px(GroupedCardMetrics::HORIZONTAL_PADDING)).pr(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    Shape::continuous(GroupedCardMetrics::CORNER_RADIUS)
                        .round(div())
                        .id("quick-reaction-preview")
                        .relative()
                        .w_full()
                        .h(px(PREVIEW_HEIGHT))
                        .overflow_hidden()
                        .child(
                            div().absolute().inset_0().child(
                                ChatDecorativeBackdrop::new()
                                    .id("quick-reaction-preview-backdrop")
                                    .ignores_safe_area(false)
                                    .preview_preset(Some(ChatWallpaperPreset::default_preset().clone()))
                                    .animates(false)
                                    .appearance_override(Some(appearance)),
                            ),
                        )
                        .child(
                            pass_through().absolute().inset_0().py(px(12.0)).flex().flex_col().justify_end().child(
                                MessageList::new("quick-reaction-preview-list", vec![message], Self::preview_chat())
                                    .interactions_enabled(false)
                                    .allows_swipe_reply(false),
                            ),
                        )
                        .child(centred_stroke(
                            theme.border._200.opacity(0.6),
                            0.5,
                            Shape::continuous(GroupedCardMetrics::CORNER_RADIUS),
                        ))
                        .on_click(move |event, _, cx| {
                            if event.click_count() == DOUBLE_TAP_CLICK_COUNT {
                                toggle_state.update(cx, |state, cx| state.toggle_quick_reaction(cx));
                            }
                        }),
                ),
            )
            .child(
                div().pt(px(3.0)).pl(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).pr(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(
                    MainText::new(tr("settings_quick_reaction_preview_hint")).px(12.0).color(theme.text.secondary),
                ),
            )
    }

    fn selection_trailing(quick_reaction: String, is_pending: bool, secondary: Hsla) -> impl IntoElement {
        hstack(8.0)
            .items_center()
            .when(is_pending, |trailing| {
                trailing.child(progress_view().control_size(ControlSize::Small).tint(secondary))
            })
            .child(Self::quick_reaction_glyph(quick_reaction, 29.0))
    }

    fn quick_reaction_glyph(quick_reaction: String, side: f32) -> ReactionEmojiGlyph {
        ReactionEmojiGlyph::new(quick_reaction, side * 0.82)
    }

    fn emoji_picker(state: &Entity<QuickReactionSettingsState>, progress: f32) -> impl IntoElement + use<> {
        let pick_state = state.clone();
        let pick_custom_state = state.clone();
        div()
            .w_full()
            .overflow_hidden()
            .max_h(px((EMOJI_PICKER_HEIGHT + EMOJI_PICKER_BOTTOM_PADDING) * progress.max(0.0)))
            .child(
                div()
                    .w_full()
                    .h(px(EMOJI_PICKER_HEIGHT))
                    .pl(px(8.0))
                    .pr(px(8.0))
                    .mb(px(EMOJI_PICKER_BOTTOM_PADDING))
                    .child(
                        EmojiPickerUi::new("quick-reaction-emoji-picker", move |emoji: String, _, cx| {
                            let Some(emoji) = EmojiPickerCatalog::normalized_selectable_emoji(Some(&emoji)) else {
                                return;
                            };
                            QuickReactionSettingsState::pick(&pick_state, &emoji, cx);
                        })
                        .chrome(false)
                        .shows_custom_emoji(true)
                        .on_pick_custom(move |item: StickerItem, _, cx| {
                            let Some(reference) = EmojiPickerCatalog::custom_emoji_ref(&item.id) else {
                                return;
                            };
                            QuickReactionSettingsState::pick(&pick_custom_state, &reference, cx);
                        }),
                    )
                    .opacity(progress.clamp(0.0, 1.0))
                    .scale_effect_anchored(0.96 + 0.04 * progress, UnitPoint::TOP),
            )
    }
}

impl RenderOnce for QuickReactionSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state =
            window.use_keyed_state("quick-reaction-settings-state", cx, |_, cx| QuickReactionSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);

        let profile = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let quick_reaction = Self::quick_reaction(profile.as_ref());
        let (is_pending, is_failed) = {
            let user_service = UserServicesStore::shared(cx);
            let user_service = user_service.read(cx);
            (user_service.quick_reaction_mutation_pending, user_service.quick_reaction_mutation_failed)
        };
        let (preview_reacted, picker_open, picker_progress) = {
            let state = state.read(cx);
            (state.preview_reacted, state.picker_open, state.picker_motion.frame_value(RiseClock::monotonic(), window))
        };

        let message = Self::preview_message(preview_reacted, &quick_reaction, profile.as_ref());
        let preview_section = Self::preview_section(&state, &theme, message, cx);

        let toggle_state = state.clone();
        let mut picker_row = GroupedBtn::new("quick_reaction_picker", tr("settings_quick_reaction_choose"))
            .trailing_slot(Self::selection_trailing(quick_reaction, is_pending, theme.text.secondary))
            .callback(move |_, cx| {
                let _ = selection_changed();
                toggle_state.update(cx, |state, cx| {
                    let picker_open = !state.picker_open;
                    state.set_picker_open(picker_open, 0.38, 0.86, cx);
                });
            });
        if picker_open || picker_progress > 0.0 {
            picker_row = picker_row.expanded_slot(Self::emoji_picker(&state, picker_progress));
        }
        let selection_section = GroupedBtns::new("quick-reaction-selection", vec![picker_row]).end_group_title(tr(
            if is_failed { "settings_quick_reaction_save_failed" } else { "settings_quick_reaction_choose_hint" },
        ));

        SettingsWrapperUi::new(SharedString::from(tr("settings_quick_reaction_title")), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("quick-reaction-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(preview_section)
                        .child(selection_section)
                        .child(div().flex_none().h(px(80.0))),
                ),
        )
    }
}
