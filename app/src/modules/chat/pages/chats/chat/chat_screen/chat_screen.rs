use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, AppContext, BACKDROP_BLUR_SUPPORTED, Context, Corners, DispatchPhase, ElementId, Empty, Entity, ExternalPaths, FocusHandle, FontWeight, KeyDownEvent, KeystrokeEvent, MouseButton, MouseDownEvent, Pixels,
    SharedString, StyleRefinement, Subscription, Task, WeakEntity, Window, canvas, div, px,
};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::{Locale, tr};
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_ui::animated_media_playback_scope::ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetMetrics;
use rise_ui::input_ui::input_actions::MoveUp;
use rise_ui::keyboard_dismiss::{KeyboardDismissModifiers, RISE_INPUT_PICKER_DISMISS_REQUESTED, ScreenFocus};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, ease_in, ease_out, spring};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::swiftui::numeric_text::NumericText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_ui::{IconUi, MainText};
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{RiseRTCRoomTarget, RootRoute};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseColdOpenTrace};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::navigations::bottom_chrome_insets::{BottomChromeInsetsModifiers, BottomChromeOwner};
use crate::core::widgets::wrappers::list_shell_ui::ListShellUi;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{
    BotInlineCommandsModel, BotInlineIdentityModel, BotInlineResultModel, BotInlineResultsPage, BotMentionCandidateModel,
};
use crate::modules::bot::stores::bot::bot_actions::bot_group_command_types::{ChatGroupBotCommandModel, ChatGroupBotCommands};
use crate::modules::bot::stores::bot::bot_actions::bot_types::{BotCommandModel, BotMiniAppModel, BotModel, BotTagRules};
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageScrollRequest;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatEdgeShadows;
use crate::modules::chat::components::chat::bot_chat_menu_sheet::{BotChatMenuPresentationContract, BotChatMenuSheet};
use crate::modules::chat::components::chat::bot_reply_keyboard_panel::BotReplyKeyboardPanel;
use crate::modules::chat::components::chat::calendar::chat_calendar_sheet::ChatCalendarSheet;
use crate::modules::chat::components::chat::chat_audio_now_playing_bar::ChatAudioNowPlayingBar;
use crate::modules::chat::components::chat::chat_group_voice_banner::{
    ChatGroupVoiceBanner, ChatGroupVoiceBannerPolicy, ChatHeaderAccessoryMetrics,
};
use crate::modules::chat::components::chat::chat_music_player_sheet::ChatMusicPlayerSheetLayer;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerMentionEntityCodec;
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::{
    ChatInlineComposerPanel, ChatInlineComposerPanelHost, ChatInlineComposerPanelMetrics,
};
use crate::modules::chat::components::chat::composer::chat_link_preview_panel::ChatLinkPreviewSettings;
use crate::modules::chat::components::chat::input_picker::chat_gif_search_overlay::ChatGifSearchOverlay;
use crate::modules::chat::components::chat::input_picker::chat_gif_search_portal::ChatGifSearchPortalAnchor;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_panel::ChatInputPickerPanel;
use crate::modules::chat::components::chat::join_requests::chat_join_requests_banner::ChatJoinRequestsBanner;
use crate::modules::chat::components::chat::join_requests::chat_join_requests_sheet::ChatJoinRequestsSheetContent;
use crate::modules::chat::components::chat::pinned::chat_pinned_bar::ChatPinnedBar;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::ChatPickerContentSafeAreaPolicy;
use crate::modules::chat::components::chat::video_note::chat_video_note_player_bar::ChatMediaPlaybackBar;
use crate::modules::chat::components::chat::video_note::chat_video_note_recording_overlay::ChatVideoNoteRecordingOverlay;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::chat::voice::chat_voice_recording_ui::{ChatVoiceLockHint, ChatVoiceLockHintHost, ChatVoiceTooShortToast};
use crate::modules::chat::components::chat_search::chat_search_overlay::ChatSearchHero;
use crate::modules::chat::engine::forum::rise_chat_voice_presence::RiseChatVoicePresentation;
use crate::modules::chat::engine::messages::rise_message_scope::{RiseMessageJumpOutcome, RiseMessageScope};
use crate::modules::chat::pages::chats::chat::chat_topics::chat_topic_bar::ChatTopicBar;
use crate::modules::chat::pages::chats::chat::chat_topics::topic_editor_sheet::TopicEditorSheet;
use crate::modules::chat::pages::chats::chats::ChatDeleteDialog;
use crate::modules::chat::shared::chat_message_jump_progress::ChatMessageJumpProgress;
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use rise_bot_api::ChatAction;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageJumpCompletion;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::chat::bubble::message_link_preview_view::MessageVoiceLinkGlimpse;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedOpenings;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperPreset;
use crate::modules::chat::components::chat::composer::chat_paste_media_sheet::{
    ChatPasteMediaEntry, ChatPasteMediaLoader, ChatPasteMediaProvider, ChatPasteMediaSheet,
};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::presence::stores::presence::presence_interactions::presence_interactions::PresenceInteractionsStore;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_interactions::theme_wallpapers_interactions::ThemeWallpapersInteractionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerInlineContext;
use crate::modules::chat::components::chat::composer::chat_composer_input_surface::ChatComposerInputSurface;
use crate::modules::chat::components::chat::composer::chat_composer_paste::ChatComposerPaste;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatComposerEditTarget, ChatComposerReplyTarget, ChatComposerUserMention, ChatDraftCustomEmoji,
    ChatShellInsetsReader, resign_chat_keyboard,
};
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::ChatInlineComposerPresentationContract;
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::ChatLinkPreviewDraft;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteRecorder;
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerCaptureMode;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceMessageRecorder;
use crate::modules::chat::components::chat::chat_title_row::ChatTitleRow;
use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreviewSession;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseConversationPresentationChange, RiseMessageScopePresentationSnapshot,
};
use crate::modules::chat::pages::chats::chat::chat_roles::role_mention_composer::RoleMentionComposer;
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatPerm, ChatPermExt, ChatPermKey, ChatRole, ChatTopic, ChatType, VoiceChannelState,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageBotKeyboardButton, MessageDeliveryStatus, MessageForwardLimits,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::{
    MessageScopePresentationSnapshot, MessageServicesStore,
};
use crate::modules::chat::stores::message::message_services::message_threads_services::ChannelPostRef;
use crate::modules::stickers::components::sticker_pack_add_cell::StickerAssetComposerStore;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_chat_send_target::StickerPackChatSendTargets;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use rise_canon::ChatMessageRequestAction;
use rise_errors::ErrorCode;
use rise_platform::haptics::{ImpactStyle, NotificationKind, impact, notification};
use rise_platform::host_os::HostOs;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::text_field_alert::TextFieldAlert;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::NotifierIcon;
use crate::core::widgets::sheets::message_external_share_materializer::MessageExternalShareCoordinator;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::chat::components::chat::chat_composer_context_bar::{ChatComposerContextBar, ChatComposerContextBarKind};
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerInputChromeModifiers;
use crate::modules::chat::components::chat::composer::chat_link_preview_panel::ChatLinkPreviewBar;
use crate::modules::chat::components::chat::composer::chat_slow_mode_countdown_button::ChatSlowModeCountdownButton;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::ChatComposerInsetAnimation;
use crate::modules::chat::components::chat::voice::chat_composer_send_mic_button::ChatComposerSendMicButton;
use crate::modules::chat::components::chat::voice::chat_voice_recording_ui::{
    ChatVoiceDraftTrack, ChatVoiceHoldTrack, ChatVoiceLockedTrack, ChatVoicePauseButton, ChatVoiceSendButton,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::utils::message_reply_preview::MessageReplyPreview;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatPublicJoinOutcome;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsDone;
use crate::modules::chat::stores::chats::chat_interactions::chat_invites_interactions::ChatInteractionsPublicJoinResult;
use crate::modules::media_picker::editor::media_reedit_launcher::{MediaReeditLauncher, MediaReeditResult};
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::moderation::stores::moderation_actions::moderation_actions::ModerationActionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use rise_core::foundation::foundation_url::Url;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::NotifierRetryOptions;
use crate::modules::bot::shared::bot_ui_validation::BotMiniAppURLPolicy;
use crate::modules::bot::stores::bot::bot_actions::bot_actions::{BotActionError, BotActionResult, BotInlineSendResult};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotMiniAppLaunchData;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiEditHydrator, ChatComposerCustomEmojiRun,
};
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatVoiceDraft;
use crate::modules::chat::components::chat::video_note::video_note_recorder::{
    VideoNoteFinishResult, VideoNoteMovieDelegate, VideoNoteMovieFinished,
};
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceRecordingFinishResult;
use crate::modules::chat::shared::utils::chat_composer_text_splitter::{ChatComposerTextSplitter, ChatComposerTextSplitterBudget};
use crate::modules::chat::stores::chats::chat_services::chat_topics_services::ChatRemovedTopicNotice;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageCustomEmojiEntity, MessageMediaItem, MessageMediaKind, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{CustomEmojiEntityWire, MessageEntitiesCodec};
use crate::modules::media_picker::export::video_note_composition::VideoNoteComposition;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem,
};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;
use std::time::Duration;
use gpui::{AnyWindowHandle, AsyncApp};
use rise_canon::RiseGuideAnchor;
use rise_engine::CancellationError;
use crate::app::shell::file_drop::{self, DropOutcome, DropRejection};
use rise_i18n::{FormatArg, tr_args};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::main_text::TextAlignT;
use rise_ui::rich_text::document::rise_document::RiseDocument;
use rise_ui::rich_text::document::rise_document_codec::RiseDocumentCodec;
use rise_ui::rich_text::document::rise_document_projection::RiseDocumentProjection;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::motion::ease_in_out;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::media_viewer::media_viewer_presentation_state::{MediaViewerPresentationState, chat_media_presentation};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatStartCard;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::{
    ChatComposerBottomPaddingPolicy, ChatPickerPanelHeightPolicy,
};
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteRecorderPhase;
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerRecordingUIPhase;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceMessageRecorderPhase;
use crate::modules::chat::components::chat_search::chat_message_search_results_list::ChatMessageSearchResultsList;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatMessageSearchPolicy, RiseMessageSearchPresentationState,
};
use crate::modules::chat::shared::chat_message_search_presenter::ChatMessageSearchPresenter;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionCompletion;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use super::chat_screen_support::*;

pub(super) fn chat_services<R>(cx: &mut App, read: impl FnOnce(&mut ChatServicesStore, &mut Context<ChatServicesStore>) -> R) -> R {
    ChatServicesStore::shared(cx).update(cx, read)
}

pub(super) fn message_services<R>(
    cx: &mut App,
    read: impl FnOnce(&mut MessageServicesStore, &mut Context<MessageServicesStore>) -> R,
) -> R {
    MessageServicesStore::shared(cx).update(cx, read)
}

pub(super) fn message_interactions<R>(
    cx: &mut App,
    run: impl FnOnce(&mut MessageInteractionsStore, &mut Context<MessageInteractionsStore>) -> R,
) -> R {
    MessageInteractionsStore::shared(cx).update(cx, run)
}

pub(super) fn chat_interactions<R>(
    cx: &mut App,
    run: impl FnOnce(&mut ChatInteractionsStore, &mut Context<ChatInteractionsStore>) -> R,
) -> R {
    ChatInteractionsStore::shared(cx).update(cx, run)
}

pub(super) fn account_id(cx: &mut App) -> String {
    AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
}

pub(super) fn is_blank(text: &str) -> bool {
    trimming_whitespaces_and_newlines(text).is_empty()
}

/// unread count taken when the scope opened
/// the divider stays above the same message for the whole visit
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct UnreadDivider {
    pub(super) scope: Option<String>,
    pub(super) count: i64,
    pub(super) message_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpeningLoad {
    pub(super) scope: String,
    pub(super) id: Uuid,
    pub(super) awaits_first_sync: bool,
}

pub(super) const OPENING_FIRST_SYNC_WAIT_CEILING: Duration = Duration::from_millis(1500);

/// scopes whose opening request already reached the server
/// cache waits for the opening request until read once, socket up
/// else the page swaps half the rows after the cache paints
#[derive(Default)]
pub(super) struct ChatOpeningSyncedScopes(HashSet<String>);

impl gpui::Global for ChatOpeningSyncedScopes {}

impl ChatOpeningSyncedScopes {
    pub(super) fn awaits_first_sync(scope: &str, cx: &mut App) -> bool {
        let synced = cx.try_global::<Self>().is_some_and(|scopes| scopes.0.contains(scope));
        !synced && RiseTransportStatus::is_connected(cx)
    }

    pub(super) fn mark_synced(scope: &str, cx: &mut App) {
        if !cx.try_global::<Self>().is_some_and(|scopes| scopes.0.contains(scope)) {
            cx.default_global::<Self>().0.insert(scope.to_owned());
        }
    }
}

pub(super) const OPENING_DIVIDER_WAIT_CEILING: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReadTrigger {
    pub(super) latest_id: Option<String>,
    pub(super) latest_is_own: bool,
    pub(super) newest_incoming_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnSend {
    pub(super) id: String,
    pub(super) sent_at: TimeInterval,
}

#[derive(Clone, Copy)]
pub(super) enum ChatScreenAppEvent {
    ConnectionReady,
    InputPickerDismissRequested,
    DidEnterBackground,
    WillEnterForeground,
    CallTookAudioSession,
}

pub struct ChatScreen {
    pub(super) chat_id: Option<String>,
    pub(super) topic_id: Option<String>,
    pub(super) initial_all_view: bool,
    pub(super) thread_root_id: Option<String>,
    pub(super) navigation_open_epoch: Option<Uuid>,
    pub(super) context_preview: Option<Entity<ChatContextPreviewSession>>,

    pub(super) state: ChatScreenState,
    pub(super) rise_message_observation_owner_id: Uuid,
    pub(super) is_message_media_playback_active: bool,
    pub(super) media_viewer_covers_feed: bool,
    pub(super) observed_group_voice: Option<VoiceChannelState>,
    pub(super) presented_chat_entry: Option<(Option<ChatInfo>, bool)>,
    pub(super) presented_scopes: HashMap<RiseMessageScope, Option<Arc<RiseMessageScopePresentationSnapshot>>>,
    pub(super) presented_pins: Option<ChatScreenPresentedPins>,
    pub(super) location_covers_feed: bool,
    pub(super) location_cover_watch: Option<(gpui::EntityId, gpui::Subscription)>,
    pub(super) thread_channel_post: Option<ChannelPostRef>,
    pub(super) link_preview_draft: Entity<ChatLinkPreviewDraft>,
    pub(super) composer_input_height: f32,
    pub(super) composer_input_width: f32,
    pub(super) navigation_store: Option<Entity<NavigationStore>>,
    pub(super) has_rendered: bool,
    pub(super) inline_panel_host_shown: bool,
    pub(super) inline_panel_host: Option<Entity<ChatInlineComposerPanelHost>>,
    pub(super) presentation_memo: Option<ChatScreenPresentationMemo>,
    pub(super) voice_hold_start_task: Option<Task<()>>,
    pub(super) video_hold_start_task: Option<Task<()>>,
    pub(super) voice_recorder: Entity<VoiceMessageRecorder>,
    pub(super) video_note_recorder: Entity<VideoNoteRecorder>,
    pub(super) capture_mode: ChatComposerCaptureMode,
    pub(super) capture_finger_down: bool,
    pub(super) capture_refused: bool,
    pub(super) picker_visible: bool,
    pub(super) composer_input_surface: ChatComposerInputSurface,
    pub(super) composer_focus_token: i64,
    pub(super) is_on_screen: bool,
    pub(super) composer_selection: NSRange,
    pub(super) composer_selection_text: String,
    pub(super) composer_requested_selection: Option<NSRange>,
    pub(super) composer_selection_token: i64,
    pub(super) inline_composer_context: ChatComposerInlineContext,
    pub(super) inline_candidates_answered_query: Option<String>,
    pub(super) inline_composer_expanded: bool,
    pub(super) inline_composer_interactive_height: Option<f32>,
    pub(super) inline_invocation_exclusion_tag: Option<String>,
    pub(super) inline_user_mentions: Vec<ChatComposerUserMention>,
    pub(super) inline_user_mentions_anchor_text: String,
    pub(super) inline_selected_bot_identity: Option<BotInlineIdentityModel>,
    pub(super) draft_custom_emojis: Vec<ChatDraftCustomEmoji>,
    pub(super) pasted_media: Vec<ChatPasteMediaEntry>,
    pub(super) pasted_media_shown: Vec<ChatPasteMediaEntry>,
    pub(super) pasted_media_presented: Animated<f32>,
    pub(super) saved_draft_custom_emojis: Option<Vec<ChatDraftCustomEmoji>>,
    pub(super) composer_overflows_message: bool,
    pub(super) show_send_text_file: bool,
    pub(super) send_text_file_name: String,
    pub(super) restored_draft_scope_key: Option<String>,
    pub(super) restored_draft_text: Option<String>,
    pub(super) show_gif_search: bool,
    pub(super) show_join_requests_sheet: bool,
    pub(super) public_join_submitting: bool,
    pub(super) public_join_request_sent: bool,
    pub(super) public_join_banned: bool,
    pub(super) outsider_card_refreshed_chat_id: String,
    pub(super) direct_peer_refreshed_chat_id: String,
    pub(super) pending_reedit_item: Option<MediaPickerItem>,
    pub(super) reedit_in_progress: bool,
    pub(super) show_replace_media_menu: bool,
    pub(super) picker_replaces_edit_media: bool,
    pub(super) dropped_files: Rc<RefCell<Vec<PathBuf>>>,
    pub(super) dismissed_bot_reply_keyboard_message_id: Option<String>,
    pub(super) bot_keyboard_capture_motion_value: bool,
    pub(super) bot_keyboard_composer_motion_value: bool,
    pub(super) handled_bot_force_reply_message_id: Option<String>,
    pub(super) show_bot_chat_menu: bool,
    pub(super) bot_menu_starts_expanded: bool,
    pub(super) bot_start_sent: bool,
    pub(super) first_frame_ready_epoch: Option<Uuid>,
    pub(super) first_frame_ready_scope: Option<String>,
    pub(super) initial_network_load_scope: Option<String>,
    pub(super) unread_divider: UnreadDivider,
    pub(super) opening_load: Option<OpeningLoad>,
    pub(super) message_viewport_chrome: Entity<ChatMessageViewportChromeState>,
    pub(super) resent_bottom_read_count: Option<i64>,
    pub(super) last_bottom_read_resend: Option<(i64, Duration)>,
    pub(super) message_list_scroll_request: Option<RiseMessageScrollRequest>,
    pub(super) pending_latest_jump_scope: Option<String>,
    pub(super) pending_pinned_jump_id: Option<Uuid>,
    pub(super) pending_date_jump_id: Option<Uuid>,
    pub(super) show_chat_calendar: bool,
    pub(super) pending_chat_calendar_day_start_ms: Option<i64>,
    pub(super) pending_chat_calendar_day_installed: bool,
    pub(super) delivered_jump_message_id: Option<String>,
    pub(super) completed_message_load_task_id: Option<ChatMessageLoadTaskID>,
    pub(super) header_status_refresh_revision: u64,
    pub(super) header_refresh_task: Option<Task<()>>,
    pub(super) pending_reedit_thumb_image: Option<CleverImage>,
    pub(super) chat_performance_trace_revision: u64,
    pub(super) content_height: f32,
    pub(super) observed_unread_marks: Option<(String, Option<String>)>,
    pub(super) focus_handle: FocusHandle,
    pub(super) page_focus: Entity<PageFocus>,
    pub(super) message_search_focus_requests: u64,
    pub(super) _subscriptions: Vec<Subscription>,
    pub(super) _app_events: Vec<RiseSubscription>,
    pub(super) _app_event_delivery: Task<()>,
}

pub(super) struct ChatScreenPresentedPins {
    chat_id: String,
    messages: Vec<GetMessageMessage>,
    has_more: bool,
    has_resolved_initial: bool,
    error: Option<String>,
}

impl ChatScreen {
    pub fn new(
        chat_id: Option<String>,
        topic_id: Option<String>,
        initial_all_view: bool,
        thread_root_id: Option<String>,
        navigation_open_epoch: Option<Uuid>,
        context_preview: Option<Entity<ChatContextPreviewSession>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_topic_tab = match topic_id.as_deref() {
            Some(topic_id) if !topic_id.is_empty() => ChatTopicTab::Topic(topic_id.to_owned()),
            _ if initial_all_view => ChatTopicTab::All,
            _ => ChatTopicTab::General,
        };
        let link_preview_draft = cx.new(|_| ChatLinkPreviewDraft::new());
        let voice_recorder = cx.new(VoiceMessageRecorder::new);
        let video_note_recorder = cx.new(VideoNoteRecorder::new);
        let message_viewport_chrome = cx.new(|_| ChatMessageViewportChromeState::default());
        let chats = ChatServicesStore::shared(cx);
        let messages = MessageServicesStore::shared(cx);
        let rise = RiseConversationPresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let users = UserServicesStore::shared(cx);
        let bots = BotServicesStore::shared(cx);
        let bot_actions = BotActionsStore::shared(cx);
        let jump_presenter = ChatMessageJumpPresenter::shared(cx);
        let search_presenter = ChatMessageSearchPresenter::shared(cx);
        let view_limited_openings = ChatViewLimitedOpenings::shared(cx);
        let voice_link_glimpse = MessageVoiceLinkGlimpse::shared(cx);
        let mut subscriptions = vec![
            cx.observe(&view_limited_openings, |_, _, cx| cx.notify()),
            cx.observe(&voice_link_glimpse, |_, _, cx| cx.notify()),
            cx.observe(&chats, |_, _, cx| cx.notify()),
            cx.observe(&messages, |_, _, cx| cx.notify()),
            cx.subscribe(&rise, |screen, _, change: &RiseConversationPresentationChange, cx| {
                if screen.reads_presentation_change(change, cx) {
                    tracing::warn!(target: "riseonly::X1746", ?change, "chat screen presentation wake");
                    cx.notify();
                }
            }),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&users, |_, _, cx| cx.notify()),
            cx.observe(&bots, |_, _, cx| cx.notify()),
            cx.observe(&bot_actions, |_, _, cx| cx.notify()),
            cx.observe(&jump_presenter, |_, _, cx| cx.notify()),
            cx.observe(&search_presenter, |_, _, cx| cx.notify()),
            cx.observe(&link_preview_draft, |_, _, cx| cx.notify()),
            cx.observe(&voice_recorder, |_, _, cx| cx.notify()),
            cx.observe(&video_note_recorder, |_, _, cx| cx.notify()),
            cx.observe(&message_viewport_chrome, |_, _, cx| cx.notify()),
        ];
        if let Some(context_preview) = &context_preview {
            subscriptions.push(cx.observe(context_preview, |_, _, cx| cx.notify()));
        }
        let push_deep_links = PushDeepLinkStore::shared(cx);
        subscriptions.push(cx.observe(&push_deep_links, |screen, _, cx| screen.consume_push_composer_focus(cx)));
        let media_viewer = chat_media_presentation(cx);
        let media_viewer_covers_feed = Self::media_viewer_covers_page(media_viewer.read(cx));
        subscriptions.push(cx.observe(&media_viewer, |screen, media_viewer, cx| {
            let covers = Self::media_viewer_covers_page(media_viewer.read(cx));
            if covers != screen.media_viewer_covers_feed {
                screen.media_viewer_covers_feed = covers;
                RiseAppEvents::default_center().post(ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE);
            }
        }));
        let typing_target = cx.weak_entity();
        subscriptions.push(cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
            let Some(screen) = typing_target.upgrade() else {
                return;
            };
            if screen.update(cx, |screen, cx| screen.type_into_composer(&event.keystroke, window, cx)) {
                cx.stop_propagation();
            }
        }));
        let voice_presentation = RiseChatVoicePresentation::shared(cx);
        subscriptions.push(cx.observe(&voice_presentation, |screen, presentation, cx| {
            let chat_id = screen.resolved_chat_id(cx);
            let state = presentation.read(cx).group_state(&chat_id);
            if state != screen.observed_group_voice {
                screen.observed_group_voice = state;
                cx.notify();
            }
        }));
        let (app_event_sender, mut app_event_receiver) = mpsc::unbounded::<ChatScreenAppEvent>();
        let center = RiseAppEvents::default_center();
        let observe = |name, event: ChatScreenAppEvent| {
            let sender = app_event_sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(event);
            })
        };
        let app_events = vec![
            observe(RISE_ENGINE_CONNECTION_READY, ChatScreenAppEvent::ConnectionReady),
            observe(RISE_INPUT_PICKER_DISMISS_REQUESTED, ChatScreenAppEvent::InputPickerDismissRequested),
            observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, ChatScreenAppEvent::DidEnterBackground),
            observe(UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, ChatScreenAppEvent::WillEnterForeground),
            observe(RiseCallAudioOwnership::DID_TAKE_SESSION_NOTIFICATION, ChatScreenAppEvent::CallTookAudioSession),
        ];
        let app_event_delivery = cx.spawn(async move |this, cx| {
            while let Some(event) = app_event_receiver.next().await {
                let delivered = this.update(cx, |screen, cx| match event {
                    ChatScreenAppEvent::ConnectionReady => screen.handle_connection_ready(cx),
                    ChatScreenAppEvent::InputPickerDismissRequested => {
                        if screen.picker_visible
                            && !StickerAssetComposerStore::shared(cx).read(cx).preserves_input_picker_presentation()
                        {
                            screen.close_picker(cx);
                        }
                    }
                    ChatScreenAppEvent::DidEnterBackground => screen.handle_did_enter_background(cx),
                    ChatScreenAppEvent::WillEnterForeground => screen
                        .video_note_recorder
                        .update(cx, |recorder, cx| recorder.resume_after_background(cx)),
                    ChatScreenAppEvent::CallTookAudioSession => screen.handle_call_took_audio_session(cx),
                });
                if delivered.is_err() {
                    return;
                }
            }
        });
        let focus_handle = cx.focus_handle();
        let page_focus = cx.new(|_| PageFocus::with_handle(focus_handle.clone()));
        Self {
            chat_id,
            topic_id,
            initial_all_view,
            thread_root_id,
            navigation_open_epoch,
            context_preview,
            state: ChatScreenState::new(initial_topic_tab, true),
            rise_message_observation_owner_id: Uuid::new_v4(),
            is_message_media_playback_active: false,
            media_viewer_covers_feed,
            observed_group_voice: None,
            presented_chat_entry: None,
            presented_scopes: HashMap::new(),
            presented_pins: None,
            location_covers_feed: false,
            location_cover_watch: None,
            thread_channel_post: None,
            link_preview_draft,
            composer_input_height: ChatBottomBarMetrics::CIRCLE,
            composer_input_width: 1.0,
            navigation_store: None,
            has_rendered: false,
            inline_panel_host_shown: false,
            inline_panel_host: None,
            presentation_memo: None,
            voice_hold_start_task: None,
            video_hold_start_task: None,
            voice_recorder,
            video_note_recorder,
            capture_mode: ChatComposerCaptureMode::Voice,
            capture_finger_down: false,
            capture_refused: false,
            picker_visible: false,
            composer_input_surface: ChatComposerInputSurface::SystemKeyboard,
            composer_focus_token: 0,
            is_on_screen: false,
            composer_selection: NSRange::new(0, 0),
            composer_selection_text: String::new(),
            composer_requested_selection: None,
            composer_selection_token: 0,
            inline_composer_context: ChatComposerInlineContext::Inactive,
            inline_candidates_answered_query: None,
            inline_composer_expanded: false,
            inline_composer_interactive_height: None,
            inline_invocation_exclusion_tag: None,
            inline_user_mentions: Vec::new(),
            inline_user_mentions_anchor_text: String::new(),
            inline_selected_bot_identity: None,
            draft_custom_emojis: Vec::new(),
            pasted_media: Vec::new(),
            pasted_media_shown: Vec::new(),
            pasted_media_presented: Animated::new(0.0),
            saved_draft_custom_emojis: None,
            composer_overflows_message: false,
            show_send_text_file: false,
            send_text_file_name: String::new(),
            restored_draft_scope_key: None,
            restored_draft_text: None,
            show_gif_search: false,
            show_join_requests_sheet: false,
            public_join_submitting: false,
            public_join_request_sent: false,
            public_join_banned: false,
            outsider_card_refreshed_chat_id: String::new(),
            direct_peer_refreshed_chat_id: String::new(),
            pending_reedit_item: None,
            reedit_in_progress: false,
            show_replace_media_menu: false,
            picker_replaces_edit_media: false,
            dropped_files: Rc::default(),
            dismissed_bot_reply_keyboard_message_id: None,
            bot_keyboard_capture_motion_value: false,
            bot_keyboard_composer_motion_value: false,
            handled_bot_force_reply_message_id: None,
            show_bot_chat_menu: false,
            bot_menu_starts_expanded: false,
            bot_start_sent: false,
            first_frame_ready_epoch: None,
            first_frame_ready_scope: None,
            initial_network_load_scope: None,
            unread_divider: UnreadDivider::default(),
            opening_load: None,
            message_viewport_chrome,
            resent_bottom_read_count: None,
            last_bottom_read_resend: None,
            message_list_scroll_request: None,
            pending_latest_jump_scope: None,
            pending_pinned_jump_id: None,
            pending_date_jump_id: None,
            show_chat_calendar: false,
            pending_chat_calendar_day_start_ms: None,
            pending_chat_calendar_day_installed: false,
            delivered_jump_message_id: None,
            completed_message_load_task_id: None,
            header_status_refresh_revision: 0,
            header_refresh_task: None,
            pending_reedit_thumb_image: None,
            chat_performance_trace_revision: 0,
            content_height: 0.0,
            observed_unread_marks: None,
            focus_handle,
            page_focus,
            message_search_focus_requests: 0,
            _subscriptions: subscriptions,
            _app_events: app_events,
            _app_event_delivery: app_event_delivery,
        }
    }

    pub(super) fn uses_context_preview_layout(&self, cx: &App) -> bool {
        self.context_preview.as_ref().is_some_and(|session| session.read(cx).uses_preview_layout())
    }

    pub(super) fn context_preview_is_transferring(&self, cx: &App) -> bool {
        self.context_preview.as_ref().is_some_and(|session| session.read(cx).is_transferring())
    }

    pub(super) fn is_thread_screen(&self) -> bool {
        self.resolved_thread_root_id().is_some()
    }

    pub(super) fn is_community_outsider(&self, cx: &mut App) -> bool {
        let chat = self.resolved_chat(cx);
        self.viewer_is_outside(&chat, cx)
    }
}

/// what body computes once per frame and hands to its stages
pub(super) struct ChatScreenFrame {
    pub(super) theme: AppTheme,
    pub(super) chat: ChatInfo,
    pub(super) viewer_blocked_peer: bool,
    pub(super) peer_blocked: bool,
    pub(super) bounded_presentation: Option<Rc<MessageScopePresentationSnapshot>>,
    pub(super) messages: Rc<Vec<GetMessageMessage>>,
    pub(super) feed_rows: Rc<Vec<Arc<GetMessageMessage>>>,
    pub(super) latest_message: Option<GetMessageMessage>,
    pub(super) reply_keyboard_message: Option<GetMessageMessage>,
    pub(super) reply_keyboard_panel_height: f32,
    pub(super) inline_panel_visible: bool,
    pub(super) inline_panel_detached: bool,
    pub(super) force_reply_message_id: Option<String>,
    pub(super) placeholder_key: String,
}

/// feed data a render derived from one immutable presentation box
pub(super) struct ChatScreenPresentationMemo {
    pub(super) source: Arc<RiseMessageScopePresentationSnapshot>,
    pub(super) chat_id: String,
    pub(super) thread_root_id: Option<String>,
    pub(super) snapshot: Rc<MessageScopePresentationSnapshot>,
    pub(super) messages: Rc<Vec<GetMessageMessage>>,
    pub(super) feed_rows: Rc<Vec<Arc<GetMessageMessage>>>,
}

#[derive(IntoElement)]
pub(super) struct ChatShellContent {
    pub(super) screen: Entity<ChatScreen>,
    pub(super) frame: Rc<ChatScreenFrame>,
    pub(super) safe_bottom: f32,
    pub(super) floating_top_inset: f32,
    pub(super) first_pass: bool,
}

impl RenderOnce for ChatShellContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { screen, frame, safe_bottom, floating_top_inset, first_pass } = self;
        CHAT_SCREEN_FIRST_PASS.set(first_pass);
        let content =
            screen.update(cx, |screen, cx| screen.shell_content(&frame, safe_bottom, floating_top_inset, window, cx));
        CHAT_SCREEN_FIRST_PASS.set(false);
        content
    }
}

pub(super) fn separator(theme: &AppTheme) -> gpui::Div {
    div().w_full().flex_none().h(px(0.5)).bg(theme.border._200.opacity(0.42))
}

impl ChatScreen {
    /// click handler that runs against the screen, a screen already gone takes no click
    pub(super) fn act<F>(this: WeakEntity<Self>, action: F) -> impl Fn(&mut Window, &mut App) + 'static
    where
        F: Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    {
        move |window, cx| {
            let _ = this.update(cx, |screen, cx| action(screen, window, cx));
        }
    }

    /// change watcher whose handler runs against the screen
    /// the slot lives as long as the view is drawn so it also fires on first draw with initial
    pub(super) fn watch<V, F>(
        this: &WeakEntity<Self>,
        id: &'static str,
        value: V,
        initial: bool,
        window: &mut Window,
        cx: &mut App,
        action: F,
    ) where
        V: Clone + PartialEq + 'static,
        F: FnOnce(&mut Self, &V, &V, &mut Window, &mut Context<Self>) + 'static,
    {
        let this = this.clone();
        on_change(id, value, initial, window, cx, move |old, new, window, cx| {
            let _ = this.update(cx, |screen, cx| action(screen, old, new, window, cx));
        });
    }

    /// shows the content with a transition, the slot is kept per id
    #[allow(clippy::too_many_arguments)]
    #[track_caller]
    pub(super) fn transitioned<E: Styled + IntoElement>(
        &mut self,
        id: &'static str,
        visible: bool,
        transition: Transition,
        spec: impl Into<Option<MotionSpec>>,
        window: &mut Window,
        cx: &mut Context<Self>,
        content: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> E,
    ) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        let initial = visible.then_some(());
        let presence = window.use_keyed_state(id, cx, move |_, _| Presence::new(initial).with_transition(transition));
        if presence.read(cx).is_present() != visible {
            presence.update(cx, |presence, cx| presence.set_in(visible.then_some(()), spec, now, cx));
        }
        presence.read(cx).value(now)?;
        let element = content(self, window, cx);
        presence
            .read(cx)
            .render_transitioned(now, window, move |_| element)
            .map(IntoElement::into_any_element)
    }

    pub(super) fn flag_binding(
        entity: Entity<Self>,
        get: fn(&Self) -> bool,
        set: fn(&mut Self, bool),
    ) -> Binding<bool> {
        Binding::entity(entity, get, move |screen, value, cx| {
            set(screen, value);
            cx.notify();
        })
    }

    /// handlers run outside render where the environment is gone
    /// so the nav store of this screen stack is kept from the last render
    pub(super) fn nav_store(&self) -> Option<Entity<NavigationStore>> {
        self.navigation_store.clone()
    }

    pub(super) fn push_route(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = self.nav_store() {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    pub(super) fn bodyframe(&mut self, cx: &mut Context<Self>) -> ChatScreenFrame {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let chat = self.resolved_chat(cx);
        let viewer_blocked_peer = self.viewer_has_blocked_peer(&chat, cx);
        let peer_blocked = viewer_blocked_peer || self.peer_restricts_viewer(&chat, cx);
        let scope = self.message_scope(cx);
        let (bounded_presentation, messages, feed_rows) = self.presentation_frame_data(&scope, cx);
        let latest_message = bounded_presentation
            .as_ref()
            .and_then(|snapshot| snapshot.latest_message.clone())
            .or_else(|| messages.last().cloned());
        let reply_keyboard_message = self.active_bot_reply_keyboard_message(bounded_presentation.as_deref(), cx);
        let reply_keyboard_panel_height =
            self.bot_reply_keyboard_panel_height(reply_keyboard_message.as_ref(), peer_blocked, cx);
        let inline_panel_visible = self.inline_composer_panel_visible(peer_blocked, cx);
        let inline_panel_detached = self.inline_composer_panel_detached();
        let force_reply_message_id = self.bot_force_reply_message_id(latest_message.as_ref(), cx);
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let text_switched_off = chat_services(cx, |store, cx| {
            store.member_send_switched_off_forum_topic_id(&chat_id, ChatPermKey::TEXT, post_topic_id.as_deref(), cx)
        });
        let placeholder_key = if text_switched_off {
            tr("chat_text_locked_placeholder").to_string()
        } else if self.is_topics_mode(cx) {
            self.topic_composer_placeholder(cx)
        } else {
            tr("funny_chat_placeholder").to_string()
        };
        ChatScreenFrame {
            theme,
            chat,
            viewer_blocked_peer,
            peer_blocked,
            bounded_presentation,
            messages,
            feed_rows,
            latest_message,
            reply_keyboard_message,
            reply_keyboard_panel_height,
            inline_panel_visible,
            inline_panel_detached,
            force_reply_message_id,
            placeholder_key,
        }
    }

    pub(super) fn presentation_frame_data(
        &mut self,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> (Option<Rc<MessageScopePresentationSnapshot>>, Rc<Vec<GetMessageMessage>>, Rc<Vec<Arc<GetMessageMessage>>>) {
        let source = message_services(cx, |store, cx| store.message_scope_presentation_source(scope, cx));
        let chat_id = self.resolved_chat_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        if let (Some(source), Some(memo)) = (source.as_ref(), self.presentation_memo.as_ref())
            && Arc::ptr_eq(source, &memo.source)
            && memo.chat_id == chat_id
            && memo.thread_root_id == thread_root_id
        {
            return (Some(memo.snapshot.clone()), memo.messages.clone(), memo.feed_rows.clone());
        }
        let bounded =
            message_services(cx, |store, cx| store.message_scope_presentation_snapshot(scope, cx)).map(Rc::new);
        let messages = Rc::new(self.visible_messages(bounded.as_deref(), cx));
        let feed_rows = Rc::new(messages.iter().cloned().map(Arc::new).collect::<Vec<_>>());
        self.presentation_memo = match (source, bounded.as_ref()) {
            (Some(source), Some(snapshot)) if !snapshot.slice.messages.is_empty() => Some(ChatScreenPresentationMemo {
                source,
                chat_id,
                thread_root_id,
                snapshot: snapshot.clone(),
                messages: messages.clone(),
                feed_rows: feed_rows.clone(),
            }),
            _ => None,
        };
        (bounded, messages, feed_rows)
    }

    pub(super) fn shell(&mut self, frame: &Rc<ChatScreenFrame>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = &frame.theme;
        let this = cx.weak_entity();
        let uses_preview_layout = self.uses_context_preview_layout(cx);
        let search_active = self.state.is_message_search_active;
        let selection_active = self.state.is_message_selection_active;
        let center_slot: AnyElement = if search_active {
            self.message_search_field(window, cx).into_any_element()
        } else if selection_active {
            let count = self.selected_messages(cx).len();
            MainText::new(format!("{} {}", tr("chat_selected_messages"), count))
                .px(15.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.primary)
                .into_any_element()
        } else {
            self.chat_header_center(&frame.chat, window, cx)
        };
        let mut shell = ListShellUi::new()
            .center_slot(center_slot)
            .center_slot_self_styled(!selection_active)
            .center_slot_fills_width(search_active)
            .use_safe_zone(true)
            .shows_back_button(
                !uses_preview_layout && !selection_active && !search_active && !self.is_opened_beside_chat_list(cx),
            )
            .on_back(Self::act(this.clone(), |screen, window, cx| {
                resign_chat_keyboard(window, cx);
                if let Some(nav_store) = screen.nav_store() {
                    nav_store.update(cx, |store, cx| store.pop(cx));
                }
            }))
            .fills_viewport_background(false)
            .respects_keyboard_safe_area(true)
            .header_chrome_shadow(None);
        if !uses_preview_layout {
            if search_active {
                shell = shell.right_jsx(self.message_search_close_button(theme, window, cx));
            } else if selection_active {
                shell = shell.right_jsx(self.message_selection_header_button(
                    "chat-selection-cancel",
                    tr("common_cancel").to_string(),
                    false,
                    Self::act(this.clone(), |screen, window, cx| screen.cancel_message_selection(window, cx)),
                    cx,
                ));
            } else if !self.is_message_request(cx) {
                shell = shell.right_jsx(self.chat_header_actions(&frame.chat, cx));
            }
        }
        if selection_active && !search_active {
            shell = shell.left_jsx(self.message_selection_header_button(
                "chat-selection-delete",
                tr("chat_delete_dialog_title").to_string(),
                true,
                Self::act(this.clone(), |screen, _, cx| {
                    screen.state.show_chat_delete_dialog = true;
                    cx.notify();
                }),
                cx,
            ));
        }
        if let Some(below_header) = self.below_header(frame, uses_preview_layout, window, cx) {
            shell = shell.below_header(below_header);
        }
        let screen = cx.entity();
        let frame = frame.clone();
        let first_pass = CHAT_SCREEN_FIRST_PASS.get();
        shell
            .content(ChatShellInsetsReader::new(move |safe_bottom, floating_top_inset| {
                ChatShellContent { screen, frame, safe_bottom, floating_top_inset, first_pass }.into_any_element()
            }))
            .into_any_element()
    }

    /// voice chat and player and pins and join requests form one pill
    /// voice goes first since a busy voice chat is the most urgent
    /// join requests always close the pill
    pub(super) fn below_header(
        &mut self,
        frame: &ChatScreenFrame,
        uses_preview_layout: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = &frame.theme;
        let chat = &frame.chat;
        let this = cx.weak_entity();
        let scope = self.message_scope(cx);
        let chat_id = self.resolved_chat_id(cx);
        let is_topics_mode = self.is_topics_mode(cx);
        let loaded_pins = match &frame.bounded_presentation {
            Some(snapshot) => snapshot.pinned_messages.clone(),
            None => message_services(cx, |store, cx| store.pinned_messages_in_loaded_history(&scope, cx)),
        };
        let uses_chat_pinned_index = is_topics_mode || scope == chat_id;
        let pinned_topic_id = if is_topics_mode { self.resolved_topic_id(cx) } else { None };
        let indexed_pins = if uses_chat_pinned_index {
            let topic_id = pinned_topic_id.clone();
            message_services(cx, |store, cx| store.pinned_messages_for_topic(&chat_id, topic_id.as_deref(), cx))
        } else {
            Vec::new()
        };
        let can_use_pinned_index =
            uses_chat_pinned_index && message_services(cx, |store, cx| store.has_pinned_messages_cache(&chat_id, cx));
        let pins = if can_use_pinned_index { indexed_pins } else { loaded_pins };
        let covered_by_search_list = self.state.is_message_search_active && self.state.message_search_shows_list;
        let shows_pins = !pins.is_empty()
            && !self.is_message_request(cx)
            && !covered_by_search_list
            && !self.viewer_removed_from_community(cx);
        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        let message_player_chat_id = if ChatMediaPlaybackBar::is_active(&chat.id, cx) {
            chat.id.clone()
        } else {
            let voice_player = voice_player.read(cx);
            if voice_player.is_active_in_chat() { voice_player.active_chat_id().to_owned() } else { String::new() }
        };
        let shows_message_player = !uses_preview_layout && !message_player_chat_id.is_empty();
        let shows_music_player = !uses_preview_layout
            && !shows_message_player
            && ChatMusicHooks::current_track(cx).is_some()
            && !ChatAudioNowPlayingBar::is_hidden(cx);
        let shows_any_player = shows_message_player || shows_music_player;
        let group_voice_state = RiseChatVoicePresentation::shared(cx).read(cx).group_state(&chat.id);
        let shows_group_voice = !uses_preview_layout
            && ChatGroupVoiceBannerPolicy::is_visible(
                chat,
                self.topic_id.as_deref(),
                group_voice_state.as_ref().map_or(0, |state| state.participant_count),
            );
        let shows_join_requests = !uses_preview_layout && chat_services(cx, |store, cx| store.can_show_join_banner(chat, cx));
        if !(shows_group_voice || shows_any_player || shows_pins || shows_join_requests) {
            return None;
        }
        let mut pill = vstack(0.0_f32);
        let has_group_banner = shows_group_voice && group_voice_state.is_some();
        if shows_group_voice && let Some(state) = group_voice_state {
            let group_chat_id = chat.id.clone();
            let nav_store = self.nav_store();
            pill = pill.child(ChatGroupVoiceBanner::new(state, move |window, cx| {
                resign_chat_keyboard(window, cx);
                if let Some(nav_store) = nav_store.as_ref() {
                    RiseRTCNavigationGate::open(RiseRTCRoomTarget::Group { chat_id: group_chat_id.clone() }, nav_store, cx);
                }
            }));
            if shows_any_player || shows_pins {
                pill = pill.child(separator(theme));
            }
        }
        if shows_message_player {
            pill = pill.child(ChatMediaPlaybackBar::new(message_player_chat_id));
        } else if shows_music_player {
            let mut bar = ChatAudioNowPlayingBar::new(
                "chat-audio-now-playing",
                Self::act(this.clone(), |screen, window, cx| {
                    resign_chat_keyboard(window, cx);
                    ChatMusicPlayerSheetLayer::present(screen.content_height, cx);
                }),
            );
            if !shows_pins && !shows_join_requests {
                bar = bar.clipped_by_card_bottom(ChatHeaderAccessoryMetrics::CARD_CORNER_RADIUS, !has_group_banner);
            }
            pill = pill.child(bar);
        }
        if shows_any_player && shows_pins {
            pill = pill.child(separator(theme));
        }
        if shows_pins {
            let on_open_message = {
                let this = this.clone();
                move |message: &GetMessageMessage, window: &mut Window, cx: &mut App| {
                    let message = message.clone();
                    let _ = this.update(cx, |screen, cx| screen.jump_to_pinned_message(&message, window, cx));
                }
            };
            let list_chat_id = chat.id.clone();
            let pinned_bar_id = SharedString::from(format!(
                "chat-pinned-bar-{}-{}",
                chat.id,
                pinned_topic_id.as_deref().unwrap_or_default()
            ));
            let on_open_list = Self::act(this.clone(), move |screen, window, cx| {
                resign_chat_keyboard(window, cx);
                screen.push_route(
                    RootRoute::PinnedMessages { chat_id: Some(list_chat_id.clone()), topic_id: pinned_topic_id.clone() },
                    cx,
                );
            });
            pill = pill.child(
                ChatPinnedBar::new(pinned_bar_id, pins, theme.clone(), on_open_message, on_open_list).embedded(true),
            );
        }
        if shows_join_requests {
            if shows_group_voice || shows_any_player || shows_pins {
                pill = pill.child(separator(theme));
            }
            let requests = chat_services(cx, |store, cx| store.join_requests(&chat.id, cx));
            let count_label = chat_services(cx, |store, cx| store.join_requests_count_label(&chat.id, cx));
            let dismiss_chat_id = chat.id.clone();
            pill = pill.child(ChatJoinRequestsBanner::new(
                requests,
                count_label,
                theme.clone(),
                Self::act(this.clone(), |screen, window, cx| {
                    resign_chat_keyboard(window, cx);
                    screen.show_join_requests_sheet = true;
                    cx.notify();
                }),
                move |_, cx| {
                    let chat_id = dismiss_chat_id.clone();
                    chat_services(cx, |store, cx| store.dismiss_join_banner(&chat_id, cx));
                },
            ));
        }
        let menu_shown = self.show_bot_chat_menu;
        let opacity = window.use_keyed_state("chat-header-accessory-opacity", cx, move |_, _| {
            Animated::new(BotChatMenuPresentationContract::header_accessory_opacity(menu_shown) as f32)
        });
        let now = RiseClock::monotonic();
        let target = BotChatMenuPresentationContract::header_accessory_opacity(menu_shown) as f32;
        opacity.update(cx, |opacity, cx| {
            opacity.set_in(target, ease_out(BottomSheetMetrics::BACKDROP_FADE_DURATION), now, cx);
        });
        if opacity.read(cx).is_running(now) {
            window.request_animation_frame();
        }
        let accessory_opacity = opacity.read(cx).value(now);
        let opaque_fill = theme.bg._100;
        let shaped = pill.liquid_chrome_rounded_rectangle(ChatHeaderAccessoryMetrics::CARD_CORNER_RADIUS);
        let mut wrapper = div()
            .id("chat-header-accessory")
            .px(px(10.0))
            .pt(px(4.0))
            .pb(px(6.0))
            .opacity(accessory_opacity)
            .child(
                div()
                    .relative()
                    .rounded(px(ChatHeaderAccessoryMetrics::CARD_CORNER_RADIUS))
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(ChatHeaderAccessoryMetrics::CARD_CORNER_RADIUS))
                            .bg(opaque_fill),
                    )
                    .child(shaped),
            );
        if menu_shown {
            wrapper = wrapper.occlude();
        }
        Some(wrapper.into_any_element())
    }

    pub(super) fn shell_content(
        &mut self,
        frame: &ChatScreenFrame,
        safe_bottom: f32,
        floating_top_inset: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &frame.theme;
        let this = cx.weak_entity();
        let peer_blocked = frame.peer_blocked;
        let reply_keyboard_panel_height = frame.reply_keyboard_panel_height;
        let inline_panel_visible = frame.inline_panel_visible;
        let inline_panel_detached = frame.inline_panel_detached;
        let uses_preview_layout = self.uses_context_preview_layout(cx);
        let picker_safe_bottom = if self.picker_visible {
            ChatPickerContentSafeAreaPolicy::resolved_safe_bottom(safe_bottom, 0.0)
        } else {
            safe_bottom
        };
        let base_composer_height = self.composer_total_height(safe_bottom, reply_keyboard_panel_height, peer_blocked, cx);
        let inline_available_height = (self.content_height - base_composer_height - floating_top_inset).max(0.0);
        let candidates_count = self.inline_composer_candidates(cx).len();
        let roles_count = self.inline_composer_roles(cx).len();
        let commands_count = self.inline_composer_commands(cx).map_or(0, |commands| commands.commands.len());
        let results = self.inline_composer_results(cx);
        let group_commands_count = self.inline_composer_group_commands(cx).len();
        let topics_count = self.inline_composer_topics(cx).len();
        let preferred_inline_panel_height = ChatInlineComposerPanelMetrics::preferred_collapsed_height(
            &self.inline_composer_context,
            candidates_count,
            roles_count,
            commands_count,
            &results.results,
            group_commands_count,
            topics_count,
        );
        let settled_inline_panel_height = ChatInlineComposerPanelMetrics::height(
            inline_available_height,
            self.inline_composer_expanded,
            preferred_inline_panel_height,
        );
        let inline_panel_height = if inline_panel_visible {
            inline_available_height.min(self.inline_composer_interactive_height.unwrap_or(settled_inline_panel_height).max(0.0))
        } else {
            0.0
        };
        let feed_inline_panel_height =
            if inline_panel_visible { inline_available_height.min(settled_inline_panel_height.max(0.0)) } else { 0.0 };
        let removed = self.viewer_removed_from_community(cx);

        let mut top_layer =
            div().absolute().inset_0().child(div().absolute().inset_0().child(self.chat_backdrop(&frame.chat, window, cx)));
        if !removed {
            top_layer = top_layer.child(self.message_feed_layer(
                &frame.chat,
                &frame.messages,
                &frame.feed_rows,
                frame.bounded_presentation.as_deref(),
                reply_keyboard_panel_height,
                feed_inline_panel_height,
                theme,
                safe_bottom,
                peer_blocked,
                window,
                cx,
            ));
        }
        let search_list_shown = self.state.message_search_shows_list;
        let search_results = self.transitioned(
            "chat-search-results",
            self.state.is_message_search_active,
            Transition::opacity(),
            ChatSearchHero::animation(),
            window,
            cx,
            |screen, window, cx| {
                let layer = screen.message_search_results_layer(floating_top_inset, base_composer_height, window, cx);
                let mut host = div().absolute().inset_0().opacity(if search_list_shown { 1.0 } else { 0.0 });
                if search_list_shown {
                    host = host.occlude();
                }
                host.child(layer)
            },
        );
        top_layer = top_layer.children(search_results);
        let scope = self.message_scope(cx);
        if !frame.messages.is_empty() {
            self.bot_start_sent = false;
        }
        if frame.chat.is_active_bot_chat()
            && frame.messages.is_empty()
            && !self.bot_start_sent
            && (self.is_pending_chat(cx) || message_services(cx, |store, cx| store.has_messages_in_memory(&scope, cx)))
            && (self.is_pending_chat(cx)
                || message_services(cx, |store, cx| store.initial_messages_status(&scope, cx)) != SaiWsSliceStatus::Rejected)
        {
            top_layer = top_layer.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(24.0))
                    .pb(px(base_composer_height * 0.4))
                    .child(self.bot_start_card(&frame.chat, window, cx)),
            );
        }

        let video_note_active =
            self.capture_mode == ChatComposerCaptureMode::VideoNote && self.video_note_recorder.read(cx).is_active();
        let backdrop_color = theme.bg._100.opacity(Material::UltraThin.painted_opacity());
        let video_backdrop = self.transitioned(
            "chat-video-note-backdrop",
            video_note_active,
            Transition::opacity(),
            ease_in(0.22),
            window,
            cx,
            move |_, _, _| div().absolute().inset_0().bg(backdrop_color),
        );

        let edge_shadows = ChatEdgeShadows::new(floating_top_inset, base_composer_height + inline_panel_height);

        let navigation_buttons = (!uses_preview_layout).then(|| {
            div()
                .absolute()
                .right(px(ChatBottomBarMetrics::H_PADDING))
                .bottom(px(base_composer_height + ChatBottomBarMetrics::ROW_GAP))
                .child(self.message_navigation_buttons(theme, window, cx))
        });

        let bot_menu_bottom = base_composer_height + ChatBottomBarMetrics::ROW_GAP;
        let bot_menu_available = inline_available_height - 2.0 * ChatBottomBarMetrics::ROW_GAP;
        let bot_menu = frame
            .chat
            .is_active_bot_chat()
            .then(|| self.bot_chat_menu(&frame.chat, bot_menu_bottom, bot_menu_available, cx));

        let composer_column =
            (!uses_preview_layout).then(|| self.composer_column(frame, safe_bottom, inline_available_height, preferred_inline_panel_height, inline_panel_height, window, cx));

        let total_height = base_composer_height;
        let video_overlay = self.transitioned(
            "chat-video-note-overlay",
            video_note_active,
            Transition::opacity(),
            ease_in(0.18),
            window,
            cx,
            move |screen, _, cx| {
                let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
                let recorder = screen.video_note_recorder.read(cx);
                let overlay = ChatVideoNoteRecordingOverlay::new(
                    "chat-video-note-recording-overlay",
                    theme,
                    recorder.capture_session().clone(),
                    recorder.elapsed_milliseconds(),
                    VideoNoteRecorder::MAX_DURATION_MILLISECONDS,
                    recorder.is_locked_or_finishing(),
                    recorder.shows_lock_hint(),
                    recorder.shows_cancel_hint(),
                    recorder.drag_translation(),
                );
                vstack(0.0_f32)
                    .absolute()
                    .inset_0()
                    .child(div().flex_1().w_full().child(overlay))
                    .child(div().flex_none().h(px(total_height)))
            },
        );

        let picker = self.transitioned(
            "chat-input-picker",
            self.picker_visible,
            Transition::move_edge(Edge::Bottom),
            ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION),
            window,
            cx,
            move |screen, _, cx| screen.input_picker(picker_safe_bottom, cx),
        );

        div()
            .relative()
            .size_full()
            .child(measure("chat-content-geometry", {
                let this = this.clone();
                move |bounds, _, cx| {
                    let _ = this.update(cx, |screen, cx| {
                        let height = f32::from(bounds.size.height);
                        if screen.content_height != height {
                            screen.content_height = height;
                            cx.notify();
                        }
                    });
                }
            }))
            .child(top_layer)
            .children(video_backdrop)
            .child(edge_shadows)
            .children(bot_menu)
            .children(composer_column)
            .children(video_overlay)
            .children(picker)
            .children(navigation_buttons)
            .into_any_element()
    }

    pub(super) fn bot_chat_menu(
        &mut self,
        chat: &ChatInfo,
        bottom_inset: f32,
        available_height: f32,
        cx: &mut Context<Self>,
    ) -> BotChatMenuSheet {
        let this = cx.weak_entity();
        let bot_chat_tag = self.bot_chat_tag(cx);
        let bot_user_id = chat.participant.id.clone();
        let cache_key = self.bot_public_mini_apps_cache_key(cx);
        let load_tag = bot_chat_tag.clone();
        let load_user_id = bot_user_id.clone();
        BotChatMenuSheet {
            id: "chat-bot-menu".into(),
            commands: self.bot_public_commands(cx),
            mini_apps: self.bot_public_mini_apps(cx),
            has_more_mini_apps: BotServicesStore::public_mini_apps_has_more(
                None,
                Some(&bot_chat_tag),
                Some(&bot_user_id),
                cx,
            ),
            is_loading_more_mini_apps: SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().is_loading,
            pagination_cache_key: cache_key,
            relative_id: BotServicesStore::public_mini_apps_relative_id(None, Some(&bot_chat_tag), Some(&bot_user_id), cx),
            status: self.bot_chat_menu_status(cx),
            error: self.bot_chat_menu_error(cx),
            starts_expanded: self.bot_menu_starts_expanded,
            bottom_inset,
            leading_inset: ChatBottomBarMetrics::H_PADDING,
            available_height,
            on_command: Rc::new({
                let this = this.clone();
                move |command: &BotCommandModel, window: &mut Window, cx: &mut App| {
                    let command = command.command.clone();
                    let _ = this.update(cx, |screen, cx| {
                        screen.dismiss_bot_chat_menu(window, cx);
                        let chat_id = screen.resolved_chat_id(cx);
                        let topic_id = screen.resolved_topic_id(cx);
                        let post_topic_id = screen.post_topic_id(cx);
                        let thread_root_id = screen.resolved_thread_root_id().map(str::to_owned);
                        let text = format!("/{command}");
                        message_interactions(cx, |store, cx| {
                            store.send_text(
                                &chat_id,
                                &text,
                                topic_id.as_deref(),
                                post_topic_id.as_deref(),
                                thread_root_id.as_deref(),
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                cx,
                            );
                        });
                        screen.follow_outgoing_message(window, cx);
                    });
                }
            }),
            on_mini_app: Rc::new({
                let this = this.clone();
                move |mini_app: &BotMiniAppModel, window: &mut Window, cx: &mut App| {
                    let mini_app = mini_app.clone();
                    let _ = this.update(cx, |screen, cx| {
                        screen.dismiss_bot_chat_menu(window, cx);
                        screen.open_public_mini_app(&mini_app, window, cx);
                    });
                }
            }),
            on_retry: Rc::new(Self::act(this.clone(), |screen, _, cx| screen.load_bot_chat_metadata(cx))),
            on_load_more_mini_apps: Rc::new(move |_: &mut Window, cx: &mut App| {
                let tag = load_tag.clone();
                let user_id = load_user_id.clone();
                BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_more_public_mini_apps(None, Some(&tag), Some(&user_id), cx);
                });
            }),
            is_presented: Self::flag_binding(
                cx.entity(),
                |screen| screen.show_bot_chat_menu,
                |screen, value| screen.show_bot_chat_menu = value,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn composer_column(
        &mut self,
        frame: &ChatScreenFrame,
        safe_bottom: f32,
        inline_available_height: f32,
        preferred_inline_panel_height: f32,
        inline_panel_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &frame.theme;
        let this = cx.weak_entity();
        let peer_blocked = frame.peer_blocked;
        let chat = &frame.chat;
        let mut column = vstack(0.0_f32).w_full();
        if self.show_topic_bar() {
            let topics = self.bar_topics(cx);
            let chat_id = self.resolved_chat_id(cx);
            let can_create = chat_services(cx, |store, cx| store.can_create_channel(&chat_id, cx));
            let shows_all = !self.all_topics_view_hidden(cx);
            column = column.child(
                ChatTopicBar::new(
                    "chat-topic-bar",
                    topics,
                    self.state.selected_topic_tab.clone(),
                    can_create,
                    {
                        let this = this.clone();
                        move |tab, window, cx| {
                            let _ = this.update(cx, |screen, cx| screen.select_topic_tab(tab, window, cx));
                        }
                    },
                    Self::act(this.clone(), |screen, _, cx| {
                        screen.state.show_topic_create = true;
                        cx.notify();
                    }),
                )
                .shows_all(shows_all),
            );
        }

        let keyboard = self
            .should_show_bot_reply_keyboard(peer_blocked, cx)
            .then(|| frame.reply_keyboard_message.clone())
            .flatten()
            .and_then(|message| {
                let rows = message.reply_markup.as_ref()?.keyboard.clone();
                Some((message, rows))
            });
        let capture_motion_value = self.is_any_capture_active(cx) && !self.active_capture_locked(cx);
        let composer_motion_value = self.picker_visible || self.inline_composer_context.is_presented();
        let keyboard_spec = if capture_motion_value != self.bot_keyboard_capture_motion_value {
            Some(spring(0.28, 0.82))
        } else if composer_motion_value != self.bot_keyboard_composer_motion_value {
            Some(ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION))
        } else {
            None
        };
        self.bot_keyboard_capture_motion_value = capture_motion_value;
        self.bot_keyboard_composer_motion_value = composer_motion_value;
        let keyboard_panel = self.transitioned(
            "chat-bot-reply-keyboard",
            keyboard.is_some(),
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
            keyboard_spec,
            window,
            cx,
            move |_, _, cx| {
                let mut host = div().w_full();
                if let Some((message, rows)) = keyboard {
                    let this = cx.weak_entity();
                    let press_message = message.clone();
                    let close_id = message.id.clone();
                    host = host.child(BotReplyKeyboardPanel {
                        id: "chat-bot-reply-keyboard-panel".into(),
                        rows,
                        on_press: Rc::new({
                            let this = this.clone();
                            move |button: &MessageBotKeyboardButton, window: &mut Window, cx: &mut App| {
                                let button = button.clone();
                                let message = press_message.clone();
                                let _ = this.update(cx, |screen, cx| {
                                    screen.handle_bot_reply_keyboard_button(&button, &message, window, cx);
                                });
                            }
                        }),
                        on_close: Rc::new(Self::act(this, move |screen, _, cx| {
                            screen.dismissed_bot_reply_keyboard_message_id = Some(close_id.clone());
                            cx.notify();
                        })),
                    });
                }
                host
            },
        );
        column = column.children(keyboard_panel);

        let voice_hint_visible = self.capture_mode == ChatComposerCaptureMode::Voice
            && self.is_any_capture_active(cx)
            && !self.active_capture_locked(cx);
        let lock_hint_host = window.with_global_id(ElementId::from("chat-voice-lock-hint-host"), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<ChatVoiceLockHintHost>>, _| {
                let host = held.unwrap_or_else(|| cx.new(|_| ChatVoiceLockHintHost::new()));
                (host.clone(), host)
            })
        });
        let gone_screen = cx.weak_entity();
        let on_lock_hint_gone: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = gone_screen.update(cx, |_, cx| cx.notify());
        });
        let hint_theme = theme.clone();
        lock_hint_host.update(cx, |host, cx| host.configure(voice_hint_visible, hint_theme, on_lock_hint_gone, cx));
        if voice_hint_visible || lock_hint_host.read(cx).is_shown(RiseClock::monotonic()) {
            let slot_size = hstack(None::<f32>)
                .w_full()
                .pb(px(8.0))
                .opacity(0.)
                .child(spacer())
                .child(ChatVoiceLockHint::new(theme.clone()))
                .child(spacer());
            column = column.child(
                div()
                    .relative()
                    .w_full()
                    .flex_none()
                    .child(slot_size)
                    .child(div().absolute().inset_0().child(lock_hint_host.cached(StyleRefinement::default().size_full()))),
            );
        }

        let detached = frame.inline_panel_detached;
        let inline_visible = frame.inline_panel_visible;
        let host = self
            .inline_panel_host
            .get_or_insert_with(|| cx.new(|_| ChatInlineComposerPanelHost::new(inline_visible)))
            .clone();
        if inline_visible {
            self.inline_panel_host_shown = true;
        }
        let inline_panel = if self.inline_panel_host_shown {
            let leading = if detached {
                ChatBottomBarMetrics::H_PADDING
            } else {
                ChatBottomBarMetrics::H_PADDING + self.composer_leading_controls_width(cx)
            };
            let trailing = if detached {
                ChatBottomBarMetrics::H_PADDING
            } else {
                ChatBottomBarMetrics::H_PADDING + ChatBottomBarMetrics::CIRCLE + ChatBottomBarMetrics::ROW_GAP
            };
            let panel = self.inline_composer_suggestion_panel(inline_available_height, preferred_inline_panel_height, cx);
            let gone_screen = cx.weak_entity();
            let on_gone: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                let _ = gone_screen.update(cx, |screen, cx| {
                    screen.inline_panel_host_shown = false;
                    cx.notify();
                });
            });
            let is_expanded = self.inline_composer_expanded;
            let motion = ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
            host.update(cx, |host, cx| {
                host.configure(panel, is_expanded, inline_visible, motion, (leading, trailing), on_gone, cx)
            });
            Some(
                div()
                    .w_full()
                    .flex_none()
                    .h(px(inline_panel_height))
                    .overflow_hidden()
                    .block_mouse_except_scroll()
                    .child(host.cached(StyleRefinement::default().size_full())),
            )
        } else {
            None
        };
        column = column.children(inline_panel);

        let bar: Option<AnyElement> = if self.state.is_message_search_active {
            Some(self.message_search_bar(theme, safe_bottom, window, cx))
        } else if self.state.is_message_selection_active {
            Some(self.message_selection_bar(theme, safe_bottom, window, cx))
        } else if self.is_message_request(cx) {
            Some(self.message_request_decision_bar(theme, safe_bottom, cx))
        } else if self.chat_unavailable(cx) || self.chat_metadata_loading(cx) {
            None
        } else if self.viewer_is_outside(chat, cx) {
            if self.viewer_removed_from_community(cx) {
                Some(self.community_banned_bar(chat, theme, safe_bottom, cx))
            } else {
                Some(self.community_preview_bar(chat, theme, safe_bottom, cx))
            }
        } else if peer_blocked {
            if frame.viewer_blocked_peer {
                Some(self.blocked_peer_bar(chat, theme, safe_bottom, cx))
            } else {
                let text = self.restricted_by_peer_text(chat, cx);
                Some(self.restricted_by_peer_bar(text, theme, safe_bottom, cx))
            }
        } else if self.channel_posting_locked(cx) {
            Some(self.channel_subscriber_bar(chat, theme, safe_bottom, cx))
        } else if self.topic_posting_locked(cx) {
            Some(self.topic_locked_bar(theme, safe_bottom, cx))
        } else {
            Some(self.bottom_bar(theme, &frame.placeholder_key, safe_bottom, peer_blocked, window, cx))
        };
        let toast_visible = self.state.voice_too_short_visible && !self.state.is_message_selection_active;
        let toast_theme = theme.clone();
        let toast = self.transitioned(
            "chat-voice-too-short",
            toast_visible,
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
            if toast_visible { spring(0.32, 0.86) } else { ease_out(0.2) },
            window,
            cx,
            move |_, _, _| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .bottom(px(ChatBottomBarMetrics::CIRCLE + ChatBottomBarMetrics::TOP_PADDING + 18.0))
                    .child(ChatVoiceTooShortToast::new(toast_theme))
            },
        );
        column = column.child(div().relative().w_full().children(bar).children(toast));

        let background = if frame.inline_panel_visible {
            Some(self.inline_panel_chrome(
                frame.inline_panel_detached,
                inline_panel_height,
                safe_bottom,
                peer_blocked,
                cx,
            ))
        } else {
            None
        };
        let column = column.keyboard_dismiss_excluded();
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .children(background)
            .child(column)
            .marks_bottom_chrome(BottomChromeOwner::ChatComposer)
            .into_any_element()
    }

    pub(super) fn inline_panel_chrome(
        &mut self,
        detached: bool,
        inline_panel_height: f32,
        safe_bottom: f32,
        peer_blocked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let composer_bottom_padding = self.composer_bottom_padding(safe_bottom, cx);
        let composer_row_height = self.composer_row_height(peer_blocked, cx);
        let opaque_fill = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        if detached {
            let chrome = div()
                .w_full()
                .h(px(inline_panel_height))
                .liquid_chrome_uneven_rounded_rectangle(22.0, 22.0, 22.0, 22.0, 0.12, true, None, true);
            return div()
                .absolute()
                .left(px(ChatBottomBarMetrics::H_PADDING))
                .right(px(ChatBottomBarMetrics::H_PADDING))
                .bottom(px(composer_bottom_padding + composer_row_height + ChatBottomBarMetrics::TOP_PADDING))
                .h(px(inline_panel_height))
                .rounded(px(22.0))
                .overflow_hidden()
                .child(div().absolute().inset_0().rounded(px(22.0)).bg(opaque_fill))
                .child(chrome)
                .occlude()
                .into_any_element();
        }
        let chrome = div().w_full().h(px(inline_panel_height + composer_row_height)).liquid_chrome_uneven_rounded_rectangle(
            22.0,
            ChatBottomBarMetrics::INPUT_CORNER,
            ChatBottomBarMetrics::INPUT_CORNER,
            22.0,
            0.12,
            true,
            None,
            true,
        );
        div()
            .absolute()
            .left(px(ChatBottomBarMetrics::H_PADDING + self.composer_leading_controls_width(cx)))
            .right(px(ChatBottomBarMetrics::H_PADDING + ChatBottomBarMetrics::CIRCLE + ChatBottomBarMetrics::ROW_GAP))
            .bottom(px(composer_bottom_padding))
            .h(px(inline_panel_height + composer_row_height))
            .rounded_tl(px(22.0))
            .rounded_tr(px(22.0))
            .rounded_bl(px(ChatBottomBarMetrics::INPUT_CORNER))
            .rounded_br(px(ChatBottomBarMetrics::INPUT_CORNER))
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_tl(px(22.0))
                    .rounded_tr(px(22.0))
                    .rounded_bl(px(ChatBottomBarMetrics::INPUT_CORNER))
                    .rounded_br(px(ChatBottomBarMetrics::INPUT_CORNER))
                    .bg(opaque_fill),
            )
            .child(chrome)
            .occlude()
            .into_any_element()
    }

    pub(super) fn input_picker(&mut self, picker_safe_bottom: f32, cx: &mut Context<Self>) -> gpui::Div {
        let this = cx.weak_entity();
        let panel_height = self.picker_panel_height(cx);
        let backing_fill = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let backing_blurs =
            BACKDROP_BLUR_SUPPORTED && !rise_ui::swiftui::environment_values::accessibility_reduce_transparency(cx);
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let restricted = |key: &str, cx: &mut Context<Self>| {
            chat_interactions(cx, |store, cx| {
                store.picker_send_switched_off(&chat_id, key, post_topic_id.as_deref(), cx)
            })
            .then(|| tr("chat_error_send_restricted").to_string())
        };
        let stickers_restricted_text = restricted(ChatPermKey::STICKERS, cx);
        let gif_restricted_text = restricted(ChatPermKey::GIF, cx);
        let typing_chat_id = self.resolved_chat_id(cx);
        let typing_is_pending_chat = self.is_pending_chat(cx);
        let host_salt = (typing_chat_id.clone(), typing_is_pending_chat, chat_id.clone(), post_topic_id.clone());
        let panel = ChatInputPickerPanel::new(
            "chat-input-picker-panel",
            panel_height,
            {
                let this = this.clone();
                move |emoji: String, window: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |screen, cx| screen.insert_composer_emoji(&emoji, window, cx));
                }
            },
            {
                let this = this.clone();
                move |item, window: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |screen, cx| screen.insert_composer_custom_emoji(item, window, cx));
                }
            },
            Self::act(this.clone(), |screen, window, cx| screen.composer_backspace(window, cx)),
            {
                let this = this.clone();
                move |sticker, window: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |screen, cx| screen.handle_picker_send_sticker(sticker, window, cx));
                }
            },
            {
                let this = this.clone();
                move |gif, window: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |screen, cx| screen.handle_picker_send_gif(gif, window, cx));
                }
            },
            Self::act(this.clone(), |screen, _, cx| {
                screen.show_gif_search = true;
                cx.notify();
            }),
            move |kind, window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |screen, cx| screen.handle_open_pack_settings(kind, window, cx));
            },
        )
        .safe_area_bottom(picker_safe_bottom)
        .over_backdrop(backing_blurs)
        .gif_search_active(self.show_gif_search)
        .stickers_restricted_text(stickers_restricted_text)
        .gif_restricted_text(gif_restricted_text)
        .on_tab_shown(move |tab, cx| {
            if typing_chat_id.is_empty() || typing_is_pending_chat {
                return;
            }
            message_interactions(cx, |store, cx| store.set_picker_typing(&typing_chat_id, tab, cx));
        });
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(panel_height))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(panel_height + ChatBottomBarMetrics::TOP_PADDING))
                    .when(!backing_blurs, |backing| backing.bg(backing_fill))
                    .when(backing_blurs, |backing| {
                        backing
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    |bounds, _, window, _| {
                                        window.paint_backdrop_blur(
                                            bounds,
                                            Corners::default(),
                                            px(ChatInputPickerPanel::BACKDROP_BLUR_RADIUS),
                                            px(0.0),
                                        );
                                    },
                                )
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full()
                                    .bg(backing_fill.opacity(ChatInputPickerPanel::backdrop_tint_peak(backing_fill.l < 0.5))),
                            )
                    })
                    .occlude(),
            )
            .child(panel.hosted(&host_salt).keyboard_dismiss_excluded())
    }

    pub(super) fn presented_overlays(&mut self, frame: &ChatScreenFrame, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = &frame.theme;
        let this = cx.weak_entity();
        let entity = cx.entity();
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let mut overlays: Vec<AnyElement> = Vec::new();

        let calendar_binding = Self::flag_binding(
            entity.clone(),
            |screen| screen.show_chat_calendar,
            |screen, value| screen.show_chat_calendar = value,
        );
        let sheet_binding = calendar_binding.clone();
        let content_this = this.clone();
        let retry_chat_id = chat_id.clone();
        let calendar_sheet = BottomSheetUi::new(
            "chat-calendar-sheet",
            calendar_binding,
            move |_, _| {
                let select_this = content_this.clone();
                let retry_chat_id = retry_chat_id.clone();
                let retry_topic_id = topic_id.clone();
                let retry_thread_root_id = thread_root_id.clone();
                ChatCalendarSheet::new(
                    "chat-calendar-sheet-content",
                    sheet_binding.clone(),
                    move |day, window: &mut Window, cx: &mut App| {
                        let _ = select_this.update(cx, |screen, cx| screen.handle_chat_calendar_day_tap(day, window, cx));
                    },
                    |_: &mut Window, cx: &mut App| {
                        let calendar = Calendar::current();
                        let locale = Locale::app_locale();
                        message_interactions(cx, |store, cx| store.load_older_chat_calendar_month(&calendar, &locale, cx));
                    },
                    move |_: &mut Window, cx: &mut App| {
                        let calendar = Calendar::current();
                        let locale = Locale::app_locale();
                        let now = Date::now();
                        message_interactions(cx, |store, cx| {
                            store.open_chat_calendar(
                                &retry_chat_id,
                                retry_topic_id.as_deref(),
                                retry_thread_root_id.as_deref(),
                                &calendar,
                                &locale,
                                &now,
                                cx,
                            );
                        });
                    },
                )
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![1.0])
        .full_page(true)
        .content_is_edge_to_edge(true)
        .full_page_allows_swipe_dismiss(true)
        .dismisses_on_backdrop_tap(false)
        .full_page_shows_drag_indicator(false)
        .full_page_hides_footer(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .full_page_covers_full_screen(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_dismiss(Self::act(this.clone(), |screen, window, cx| screen.finish_pending_chat_calendar_jump(window, cx)));
        overlays.push(calendar_sheet.into_any_element());

        let gif_binding = Self::flag_binding(
            entity.clone(),
            |screen| screen.show_gif_search,
            |screen, value| screen.show_gif_search = value,
        );
        let gif_this = this.clone();
        if self.show_gif_search {
            let sheet = ChatGifSearchOverlay::new(
                "chat-gif-search-overlay",
                gif_binding,
                move |gif, window: &mut Window, cx: &mut App| {
                    let _ = gif_this.update(cx, |screen, cx| screen.handle_picker_send_gif(gif, window, cx));
                },
            )
            .into_view(window, cx);
            overlays.push(
                div()
                    .absolute()
                    .inset_0()
                    .child(ChatGifSearchPortalAnchor::new("chat-gif-search-portal", sheet))
                    .into_any_element(),
            );
        }

        if let Some(pasted_media_sheet) = self.pasted_media_overlay(window, cx) {
            overlays.push(pasted_media_sheet);
        }

        let join_chat_id = chat_id.clone();
        let join_is_channel = frame.chat.r#type == ChatType::Channel;
        let join_this = this.clone();
        let join_sheet = BottomSheetUi::new(
            "chat-join-requests-sheet",
            Self::flag_binding(
                entity.clone(),
                |screen| screen.show_join_requests_sheet,
                |screen, value| screen.show_join_requests_sheet = value,
            ),
            move |window, cx| {
                let chat_id = join_chat_id.clone();
                let this = join_this.clone();
                window.use_keyed_state(
                    SharedString::from(format!("chat-join-requests-content-{chat_id}")),
                    cx,
                    move |_, cx| {
                        let profile_this = this.clone();
                        ChatJoinRequestsSheetContent::new(
                            chat_id,
                            join_is_channel,
                            move |user_id, tag, _, cx| {
                                let _ = profile_this.update(cx, |screen, cx| screen.open_join_request_profile(&user_id, &tag, cx));
                            },
                            Self::act(this, |screen, _, cx| {
                                screen.show_join_requests_sheet = false;
                                cx.notify();
                            }),
                            cx,
                        )
                    },
                )
            },
            |_, _| div().h(px(0.0)),
        )
        .snap_fractions(vec![0.55, 0.9])
        .footer_shows_top_border(false)
        .content_scrollable(true);
        overlays.push(join_sheet.into_any_element());

        overlays.push(
            TopicEditorSheet::new(
                Self::flag_binding(
                    entity.clone(),
                    |screen| screen.state.show_topic_create,
                    |screen, value| screen.state.show_topic_create = value,
                ),
                chat_id.clone(),
            )
            .on_saved({
                let this = this.clone();
                let handle = window.window_handle();
                move |topic: ChatTopic, cx: &mut App| {
                    let tab = ChatTopicTab::Topic(topic.id);
                    Self::deliver_in_window(&this, handle, cx, move |screen, window, cx| screen.select_topic_tab(tab, window, cx));
                }
            })
            .into_any_element(),
        );

        let picker_open = entity.clone();
        let picker_close = entity.clone();
        let complete_this = this.clone();
        overlays.push(
            MediaPickerSheet::new(
                "chat-media-picker-sheet",
                Binding::new(
                    move |cx| picker_open.read(cx).state.media_picker_mode != ChatMediaPickerMode::None,
                    move |value, _, cx| {
                        if !value {
                            picker_close.update(cx, |screen, cx| {
                                screen.state.media_picker_mode = ChatMediaPickerMode::None;
                                cx.notify();
                            });
                        }
                    },
                ),
                self.media_picker_configuration(cx),
                ThemeStore::shared(cx),
                move |result, window, cx| {
                    let _ = complete_this.update(cx, |screen, cx| {
                        let mode = screen.state.media_picker_mode;
                        screen.state.media_picker_mode = ChatMediaPickerMode::None;
                        cx.notify();
                        cx.defer_in(window, move |screen, window, cx| {
                            screen.handle_media_picker_result(result, mode, window, cx);
                        });
                    });
                },
            )
            .pending_external_paths(self.dropped_files.clone())
            .caption(Some(Binding::entity(
                entity.clone(),
                |screen: &Self| screen.state.input_text.clone(),
                |screen, value, cx| {
                    screen.state.input_text = value;
                    cx.notify();
                },
            )))
            .caption_custom_emojis(Some(Binding::entity(
                entity.clone(),
                |screen: &Self| screen.draft_custom_emojis.clone(),
                |screen, value, cx| {
                    screen.draft_custom_emojis = value;
                    cx.notify();
                },
            )))
            .experimental_bottom_sheet(true)
            .into_any_element(),
        );

        let link_preview = ChatLinkPreviewSettings::new(
            "chat-link-preview-settings",
            self.link_preview_draft.clone(),
            self.state.input_text.clone(),
            self.composer_entities(&self.state.input_text.clone(), cx),
            Arc::new(frame.chat.clone()),
            {
                let this = this.clone();
                let chat = frame.chat.clone();
                move |window, cx| {
                    this.update(cx, |screen, cx| screen.chat_backdrop(&chat, window, cx).into_any_element())
                        .unwrap_or_else(|_| Empty.into_any_element())
                }
            },
        );
        overlays.push(link_preview.into_any_element());

        if self.state.show_chat_delete_dialog {
            let chat = frame.chat.clone();
            let for_me = this.clone();
            let for_everyone = this.clone();
            overlays.push(
                ChatDeleteDialog::new(
                    chat,
                    Self::act(for_me, |screen, window, cx| {
                        if screen.resolved_chat(cx).r#type == ChatType::Favourites {
                            screen.clear_favourites_history(window, cx);
                        } else {
                            screen.delete_current_chat(false, window, cx);
                        }
                    }),
                    Self::act(for_everyone, |screen, window, cx| {
                        if screen.resolved_chat(cx).r#type == ChatType::Favourites {
                            screen.clear_favourites_history(window, cx);
                        } else {
                            screen.delete_current_chat(true, window, cx);
                        }
                    }),
                    Self::act(this, |screen, _, cx| {
                        screen.state.show_chat_delete_dialog = false;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            );
        }
        overlays
    }

    pub(super) fn present_selected_messages_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.show_selected_messages_delete_dialog = false;
        cx.notify();
        let can_everyone = self.selected_messages_can_delete_for_everyone(cx);
        let can_me = self.selected_messages_can_delete_for_me(cx);
        let (confirm_title, confirm_for_everyone, secondary_title) = match (can_everyone, can_me) {
            (true, true) => (tr("ctx_delete_for_everyone"), true, Some(tr("ctx_delete_for_me").to_string())),
            (true, false) => (tr("ctx_delete_for_everyone"), true, None),
            (false, true) => (tr("ctx_delete_for_me"), false, None),
            (false, false) => return,
        };
        let handle = window.window_handle();
        let this = cx.weak_entity();
        let delete_with = move |for_everyone: bool, this: WeakEntity<Self>, cx: &mut App| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |screen, cx| screen.delete_selected_messages(for_everyone, window, cx));
            });
        };
        let secondary_this = this.clone();
        let on_secondary: Option<NativeConfirmAlertAction> = secondary_title.is_some().then(|| {
            let action: NativeConfirmAlertAction = Box::new(move |cx: &mut App| delete_with(false, secondary_this, cx));
            action
        });
        NativeConfirmAlert::present_delete(
            &tr("ctx_delete"),
            "",
            Some(confirm_title.to_string()),
            secondary_title,
            true,
            on_secondary,
            None,
            move |cx| delete_with(confirm_for_everyone, this, cx),
            cx,
        );
    }

    pub(super) fn present_failed_send_dialog(&mut self, target: &ChatFailedSendTarget, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        let this = cx.weak_entity();
        let run = move |message_id: String, action: &'static str, this: WeakEntity<Self>, cx: &mut App| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |screen, cx| {
                    screen.state.failed_send_target = None;
                    screen.handle_message_action(&message_id, action, window, cx);
                });
            });
        };
        let retry_this = this.clone();
        let cancel_this = this.clone();
        let retry_id = target.message_id.clone();
        let delete_id = target.message_id.clone();
        let on_secondary: Option<NativeConfirmAlertAction> = target.can_retry.then(|| {
            let action: NativeConfirmAlertAction =
                Box::new(move |cx: &mut App| run(retry_id, ChatMessageAction::RETRY_SENDING, retry_this, cx));
            action
        });
        let on_cancel: NativeConfirmAlertAction = Box::new(move |cx: &mut App| {
            let _ = cancel_this.update(cx, |screen, cx| {
                screen.state.failed_send_target = None;
                cx.notify();
            });
        });
        NativeConfirmAlert::present_delete(
            &tr("chat_message_status_failed"),
            target.reason.as_deref().unwrap_or(""),
            Some(tr("ctx_delete").to_string()),
            target.can_retry.then(|| tr("ctx_retry_sending").to_string()),
            false,
            on_secondary,
            Some(on_cancel),
            move |cx| run(delete_id, ChatMessageAction::CANCEL_SENDING, this, cx),
            cx,
        );
    }

    pub(super) fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.secondary() && keystroke.key == "f" {
            if !self.is_context_preview(cx) && !self.state.is_message_search_active {
                self.activate_message_search(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if keystroke.key != "escape" || keystroke.modifiers.modified() {
            return;
        }
        if self.handle_escape(window, cx) {
            cx.stop_propagation();
        }
    }

    /// closes the selection, the search, the picker panel or the reply bar in that order
    /// returns whether one of them was up
    pub(super) fn handle_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.state.is_message_selection_active {
            self.cancel_message_selection(window, cx);
        } else if self.state.is_message_search_active {
            self.deactivate_message_search(window, cx);
        } else if !self.pasted_media.is_empty() {
            self.close_pasted_media(cx);
        } else if self.picker_visible {
            self.close_picker(cx);
        } else if self.state.reply_target.is_some() || self.state.edit_target.is_some() {
            self.cancel_composer_reply_or_edit(window, cx);
        } else {
            return false;
        }
        true
    }

    pub(super) fn apply_load_lifecycle(&mut self, _frame: &ChatScreenFrame, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let is_context_preview = self.is_context_preview(cx);
        Self::watch(&this, "chat-on-preview", is_context_preview, false, window, cx, |screen, _, previewing, window, cx| {
            if !*previewing {
                screen.handle_preview_finished(window, cx);
            }
        });
        let chat_id = self.resolved_chat_id(cx);
        let metadata_task_id = format!(
            "{}|{}|{}",
            chat_id,
            self.has_resolved_chat_metadata(cx),
            self.is_community_outsider(cx)
        );
        Self::watch(&this, "chat-metadata-task", metadata_task_id, true, window, cx, |screen, _, _, _, cx| {
            screen.run_chat_metadata_task(cx);
        });
        let load_task_id = self.message_load_task_id(cx);
        Self::watch(&this, "chat-message-load-task", load_task_id, true, window, cx, |screen, _, _, window, cx| {
            screen.run_message_load_task(window, cx);
        });
        let unread_marks_task_id = self.unread_marks_task_id(cx);
        Self::watch(&this, "chat-unread-marks-task", unread_marks_task_id, true, window, cx, |screen, _, _, _, cx| {
            screen.release_observed_unread_marks(cx);
            let chat_id = screen.resolved_chat_id(cx);
            if chat_id.is_empty() || screen.is_pending_chat(cx) {
                return;
            }
            let topic_id = screen.read_topic_id(cx);
            let observed = chat_id.clone();
            let observed_topic = topic_id.clone();
            message_interactions(cx, |store, cx| store.observe_unread_marks(&observed, observed_topic.as_deref(), cx));
            screen.observed_unread_marks = Some((chat_id, topic_id));
        });
        let all_hidden = self.all_topics_view_hidden(cx);
        Self::watch(&this, "chat-all-topics-hidden-task", all_hidden, true, window, cx, |screen, _, _, window, cx| {
            screen.leave_hidden_all_topics_view(window, cx);
        });
        Self::watch(
            &this,
            "chat-navigation-epoch-task",
            self.navigation_open_epoch,
            true,
            window,
            cx,
            |screen, _, _, _, cx| {
                if let Some(epoch) = screen.navigation_open_epoch
                    && screen.first_frame_ready_epoch != Some(epoch)
                {
                    screen.first_frame_ready_scope = None;
                    cx.notify();
                }
            },
        );
        let scope = self.message_scope(cx);
        Self::watch(&this, "chat-scope-change", scope.clone(), false, window, cx, |screen, old_scope, _, _, cx| {
            if old_scope.is_empty() {
                return;
            }
            let owner = screen.rise_message_observation_owner_id;
            let old = old_scope.clone();
            message_interactions(cx, |store, cx| {
                store.release_messages(&old, Some(owner), cx);
                store.cancel_pending_latest_messages_jump(&old, cx);
            });
            if screen.pending_latest_jump_scope.as_deref() == Some(old_scope.as_str()) {
                screen.pending_latest_jump_scope = None;
            }
        });
        Self::watch(&this, "chat-id-change", chat_id.clone(), false, window, cx, |_, old_chat_id, _, _, _| {
            ChatPerformanceTrace::shared().stop_if_recording(old_chat_id, "chat_switch");
        });
        let revision = message_services(cx, |store, _| store.latest_jump_completion_revision(&scope));
        Self::watch(&this, "chat-latest-jump-revision", revision, false, window, cx, |screen, _, revision, window, cx| {
            screen.complete_pending_latest_messages_jump(*revision, window, cx);
        });
        let pending_jump = ChatMessageJumpPresenter::shared(cx).read(cx).pending().cloned();
        Self::watch(&this, "chat-pending-jump", pending_jump, false, window, cx, |screen, _, request, window, cx| {
            let Some(request) = request else {
                return;
            };
            let current_task_id = screen.message_load_task_id(cx);
            if request.chat_id == screen.resolved_chat_id(cx)
                && screen.is_top_chat_screen(cx)
                && screen.completed_message_load_task_id.as_ref() == Some(&current_task_id)
            {
                screen.consume_pending_message_jump(window, cx);
            }
        });
        let pending_search = ChatMessageSearchPresenter::shared(cx).read(cx).pending_chat_id().map(str::to_owned);
        Self::watch(&this, "chat-pending-search", pending_search, false, window, cx, |screen, _, request, window, cx| {
            let Some(request) = request else {
                return;
            };
            let current_task_id = screen.message_load_task_id(cx);
            if *request == screen.resolved_chat_id(cx)
                && screen.is_top_chat_screen(cx)
                && screen.completed_message_load_task_id.as_ref() == Some(&current_task_id)
            {
                screen.consume_pending_message_search(window, cx);
            }
        });
        let eviction_revision = message_services(cx, |store, _| store.history_eviction_revision());
        Self::watch(&this, "chat-history-eviction", eviction_revision, false, window, cx, |screen, _, _, _, cx| {
            screen.retry_initial_messages(cx);
            screen.handle_connection_ready(cx);
        });
        let deletion_revision = message_services(cx, |store, _| store.confirmed_deletion_revision(&chat_id));
        Self::watch(&this, "chat-deletion-revision", deletion_revision, false, window, cx, |screen, _, _, _, cx| {
            screen.prune_confirmed_deleted_message_selection(cx);
        });
    }

    pub(super) fn handle_preview_finished(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope = self.message_scope(cx);
        if self.unread_divider.scope.as_deref() == Some(scope.as_str())
            && self.unread_divider.message_id.is_none()
            && self.unread_divider.count == 0
        {
            self.unread_divider.count = self.active_unread_count(cx).max(0);
            if self.unread_divider.count > 0 {
                let key = self.scroll_position_key(cx);
                message_interactions(cx, |store, cx| store.forget_reading_position(&key, cx));
            }
        }
        let chat_id = self.resolved_chat_id(cx);
        GuideFacts::set(RiseGuideSignal::ChatOpened, !chat_id.is_empty(), cx);
        let is_at_latest = self.message_viewport_chrome.read(cx).is_at_latest();
        self.handle_message_viewport_state(is_at_latest, false, window, cx);
        self.exit_floating_video_note(&chat_id, cx);
    }

    pub(super) fn exit_floating_video_note(&self, chat_id: &str, cx: &mut App) {
        let player = VideoNotePlayerStore::shared(cx);
        let floating = {
            let player = player.read(cx);
            player.is_active() && player.active_chat_id() == chat_id && player.is_floating()
        };
        if floating {
            player.update(cx, |player, cx| player.exit_floating(cx));
        }
    }

    pub(super) fn handle_appear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ChatCoveredFeedHold::resume(self.rise_message_observation_owner_id);
        self.is_message_media_playback_active = true;
        let chat_id = self.resolved_chat_id(cx);
        let root_id = self.resolved_thread_root_id().map(str::to_owned);
        if self.thread_channel_post.is_none()
            && let Some(root_id) = &root_id
        {
            self.thread_channel_post = message_services(cx, |store, cx| store.channel_post_for_thread(&chat_id, root_id, cx));
        }
        if let (Some(root_id), Some(post)) = (&root_id, self.thread_channel_post.clone())
            && let Some(count) = message_services(cx, |store, cx| store.comments_count(&post, root_id, cx))
        {
            message_services(cx, |store, cx| store.apply_thread_total(root_id, count, cx));
        }
        let is_preview = self.is_context_preview(cx);
        if !is_preview {
            GuideFacts::set(RiseGuideSignal::ChatOpened, !chat_id.is_empty(), cx);
            let read_topic_id = self.read_topic_id(cx);
            chat_interactions(cx, |store, cx| store.clear_delivered_notifications(&chat_id, read_topic_id.as_deref(), cx));
            GuideFacts::set(
                RiseGuideSignal::AttachmentPickerOpened,
                self.state.media_picker_mode == ChatMediaPickerMode::Attachments,
                cx,
            );
        }
        let task_id = self.message_load_task_id(cx);
        if self.completed_message_load_task_id.as_ref() == Some(&task_id) {
            self.consume_pending_message_jump(window, cx);
            self.consume_pending_message_search(window, cx);
        }
        PageFocus::did_appear(&self.page_focus, window, cx);
        self.is_on_screen = true;
        if !is_preview {
            let screen = cx.weak_entity();
            StickerPackChatSendTargets::register(
                cx.entity_id(),
                Rc::new(move |sticker: StickerItem, window: &mut Window, cx: &mut App| {
                    let _ = screen.update(cx, |screen, cx| screen.handle_picker_send_sticker(sticker, window, cx));
                }),
                cx,
            );
        }
        self.consume_push_composer_focus(cx);
        cx.notify();
    }

    fn reads_presentation_change(&mut self, change: &RiseConversationPresentationChange, cx: &mut App) -> bool {
        let chat_scoped = match change {
            RiseConversationPresentationChange::MessageScope(scope)
            | RiseConversationPresentationChange::UnreadMarks(scope) => scope.chat_id.as_str(),
            RiseConversationPresentationChange::PinnedMessages(chat_id) => chat_id.as_str(),
            RiseConversationPresentationChange::MessageTabs
            | RiseConversationPresentationChange::GlobalMessageSearch
            | RiseConversationPresentationChange::ChatOverlaySearch
            | RiseConversationPresentationChange::ChatSearchRecents
            | RiseConversationPresentationChange::FavouritesChat => return false,
            RiseConversationPresentationChange::ChatList(_) | RiseConversationPresentationChange::ChatDetail => {
                return self.presented_chat_entry_changed(cx);
            }
            RiseConversationPresentationChange::All | RiseConversationPresentationChange::MessageSearch => {
                return true;
            }
        };
        let raw = self.chat_id.as_deref().unwrap_or_default();
        let is_own = raw.is_empty() || chat_scoped == raw || chat_scoped == self.resolved_chat_id(cx);
        if !is_own {
            return false;
        }
        match change {
            RiseConversationPresentationChange::MessageScope(scope) => self.presented_scope_changed(scope, cx),
            RiseConversationPresentationChange::PinnedMessages(chat_id) => self.presented_pins_changed(chat_id, cx),
            _ => true,
        }
    }

    fn presented_scope_changed(&mut self, scope: &RiseMessageScope, cx: &mut App) -> bool {
        let next = RiseConversationPresentation::shared(cx).read(cx).message_scope_snapshot(scope);
        let previous = self.presented_scopes.insert(scope.clone(), next.clone());
        match (previous, next) {
            (Some(Some(previous)), Some(next)) => !Self::shows_same_window(&previous, &next),
            (Some(None), None) => false,
            _ => true,
        }
    }

    fn shows_same_window(
        previous: &RiseMessageScopePresentationSnapshot,
        next: &RiseMessageScopePresentationSnapshot,
    ) -> bool {
        let (a, b) = (&previous.source, &next.source);
        a.local_ids == b.local_ids
            && a.entity_stamps == b.entity_stamps
            && a.anchor_local_id == b.anchor_local_id
            && a.has_cached_before == b.has_cached_before
            && a.has_cached_after == b.has_cached_after
            && a.before_cursor == b.before_cursor
            && a.after_cursor == b.after_cursor
            && a.has_more_before == b.has_more_before
            && a.has_more_after == b.has_more_after
            && a.load_state == b.load_state
            && a.error_code == b.error_code
            && previous.visible_local_ids == next.visible_local_ids
            && previous.content_stamps_by_local_id == next.content_stamps_by_local_id
            && previous.anchor_offset == next.anchor_offset
            && previous.pinned_messages == next.pinned_messages
            && previous.latest_reply_keyboard_message == next.latest_reply_keyboard_message
    }

    fn presented_pins_changed(&mut self, chat_id: &str, cx: &mut App) -> bool {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        let messages = presentation.pinned_messages(chat_id);
        let has_more = presentation.pinned_messages_has_more(chat_id);
        let has_resolved_initial = presentation.pinned_messages_has_resolved_initial(chat_id);
        let error = presentation.pinned_messages_error(chat_id);
        if let Some(shown) = &self.presented_pins
            && shown.chat_id == chat_id
            && shown.messages.as_slice() == messages
            && shown.has_more == has_more
            && shown.has_resolved_initial == has_resolved_initial
            && shown.error.as_deref() == error
        {
            return false;
        }
        self.presented_pins = Some(ChatScreenPresentedPins {
            chat_id: chat_id.to_owned(),
            messages: messages.to_vec(),
            has_more,
            has_resolved_initial,
            error: error.map(str::to_owned),
        });
        true
    }

    fn presented_chat_entry_changed(&mut self, cx: &mut App) -> bool {
        let raw = self.chat_id.clone().unwrap_or_default();
        if raw.is_empty() {
            return true;
        }
        let resolved = self.resolved_chat_id(cx);
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        let entry = (
            presentation.chat(&resolved).or_else(|| presentation.chat(&raw)).cloned(),
            presentation.is_chat_archived(&resolved),
        );
        if self.presented_chat_entry.as_ref() == Some(&entry) {
            return false;
        }
        self.presented_chat_entry = Some(entry);
        true
    }

    /// open and not closing, the flyback into the bubble needs the cell playing again
    fn media_viewer_covers_page(state: &MediaViewerPresentationState) -> bool {
        state.covers_page()
    }

    /// a reply action from a toast without a reply field, the composer takes the keyboard once
    pub(super) fn consume_push_composer_focus(&mut self, cx: &mut Context<Self>) {
        if !self.is_on_screen || self.is_thread_screen() || self.is_context_preview(cx) {
            return;
        }
        let resolved_chat_id = self.resolved_chat_id(cx);
        let raw_chat_id = self.chat_id.clone().unwrap_or_default();
        let topic_id = self.topic_id.clone();
        let shows_all_topics = self.initial_all_view;
        let taken = PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
            store.take_composer_focus(
                &[raw_chat_id.as_str(), resolved_chat_id.as_str()],
                topic_id.as_deref(),
                shows_all_topics,
                cx,
            )
        });
        if !taken {
            return;
        }
        self.close_picker(cx);
        self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn run_chat_metadata_task(&mut self, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if !self.is_context_preview(cx) {
            self.exit_floating_video_note(&chat_id, cx);
        }
        if chat_id.is_empty() || self.is_pending_chat(cx) {
            return;
        }
        let known_type = ChatServicesStore::shared(cx).read(cx).chat_info(&chat_id, cx).map(|chat| chat.r#type);
        let Some(chat_type) = known_type else {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(&chat_id, false, cx));
            return;
        };
        if chat_type == ChatType::Direct
            && !self.is_context_preview(cx)
            && self.direct_peer_refreshed_chat_id != chat_id
        {
            self.direct_peer_refreshed_chat_id = chat_id.clone();
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(&chat_id, true, cx));
        }
        if self.is_community_outsider(cx) {
            if self.outsider_card_refreshed_chat_id != chat_id {
                self.outsider_card_refreshed_chat_id = chat_id.clone();
                ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(&chat_id, true, cx));
            }
            return;
        }
        chat_interactions(cx, |store, cx| store.load_chat_settings(&chat_id, cx));
        if self.resolved_thread_root_id().is_none() && self.is_topics_mode(cx) {
            chat_interactions(cx, |store, cx| {
                store.load_topics(&chat_id, cx);
                store.load_roles(&chat_id, cx);
            });
            if !self.is_context_preview(cx)
                && let Some(topic_id) = self.topic_id.clone().filter(|topic_id| !topic_id.is_empty())
            {
                chat_interactions(cx, |store, cx| store.verify_opened_topic(&chat_id, &topic_id, false, cx));
            }
        }
    }

    /// pins made while the socket was down reach this chat only as rows
    /// so the bar is read again on every reconnect
    pub(super) fn handle_connection_ready(&mut self, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        let scope = self.message_scope(cx);
        if chat_id.is_empty()
            || self.is_pending_chat(cx)
            || self.viewer_removed_from_community(cx)
            || !(self.is_topics_mode(cx) || scope == chat_id)
        {
            return;
        }
        message_interactions(cx, |store, cx| store.load_initial_pinned_messages(&chat_id, cx));
    }

    pub(super) fn run_message_load_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task_id_at_start = self.message_load_task_id(cx);
        if let Some(context_preview) = self.context_preview.clone() {
            let scope = self.message_scope(cx);
            let loaded = self.completed_message_load_task_id.as_ref() == Some(&task_id_at_start)
                || context_preview.read(cx).opening_loaded_scope.as_deref() == Some(scope.as_str());
            if loaded {
                self.completed_message_load_task_id = Some(task_id_at_start);
                return;
            }
        }
        let redelivered_jump_message_id = self.delivered_jump_message_id.take();
        self.state.is_message_selection_active = false;
        self.state.selected_messages_by_server_id.clear();
        self.pending_latest_jump_scope = None;
        self.pending_pinned_jump_id = None;
        let presentation = self.viewport_presentation(cx);
        self.message_viewport_chrome.update(cx, |chrome, cx| chrome.reset_for_presentation(&presentation, cx));
        self.message_list_scroll_request = None;
        self.show_chat_calendar = false;
        self.pending_chat_calendar_day_start_ms = None;
        self.pending_chat_calendar_day_installed = false;
        cx.notify();
        let chat_id = self.resolved_chat_id(cx);
        let viewer_removed = self.viewer_removed_from_community(cx);
        let trace_pending = self.is_pending_chat(cx);
        let trace_unread = self.active_unread_count(cx);
        let trace_scope = self.message_scope(cx);
        let trace_rows = message_services(cx, |store, cx| store.in_memory_message_count(&trace_scope, cx));
        RiseColdOpenTrace::mark(|| {
            format!(
                "chat load task chatEmpty={} removed={viewer_removed} pending={trace_pending} unread={trace_unread} rows={trace_rows}",
                chat_id.is_empty()
            )
        });
        if chat_id.is_empty() || viewer_removed {
            return;
        }
        let owner = self.rise_message_observation_owner_id;
        if self.is_pending_chat(cx) {
            message_interactions(cx, |store, cx| store.observe_draft_chat_messages(&chat_id, Some(owner), cx));
            return;
        }
        let scope = self.message_scope(cx);
        if self.is_topics_mode(cx) || scope == chat_id {
            message_interactions(cx, |store, cx| store.load_initial_pinned_messages(&chat_id, cx));
        }
        if self.first_frame_ready_scope.as_deref() != Some(scope.as_str()) {
            self.first_frame_ready_scope = None;
        }
        self.initial_network_load_scope = Some(scope.clone());
        let opening_jump_message_id = match redelivered_jump_message_id {
            Some(message_id) => Some(message_id),
            None => self.consume_pending_message_jump_target(window, cx),
        };
        self.delivered_jump_message_id = opening_jump_message_id.clone();
        if self.message_load_task_id(cx) != task_id_at_start {
            return;
        }
        if self.unread_divider.scope.as_deref() != Some(scope.as_str()) {
            let count = if self.is_context_preview(cx) || opening_jump_message_id.is_some() {
                0
            } else {
                self.active_unread_count(cx).max(0)
            };
            self.unread_divider = UnreadDivider { scope: Some(scope.clone()), count, message_id: None };
            if count > 0 {
                let key = self.scroll_position_key(cx);
                message_interactions(cx, |store, cx| store.forget_reading_position(&key, cx));
            }
        }
        self.consume_pending_message_search(window, cx);
        let jump_token = opening_jump_message_id.as_ref().map(|_| self.begin_message_jump(window, cx));
        let awaits_first_sync = opening_jump_message_id.is_none()
            && !self.is_context_preview(cx)
            && ChatOpeningSyncedScopes::awaits_first_sync(&scope, cx);
        let opening = OpeningLoad { scope: scope.clone(), id: Uuid::new_v4(), awaits_first_sync };
        self.opening_load = Some(opening.clone());
        let ceiling_opening = opening.clone();
        let ceiling = if awaits_first_sync { OPENING_FIRST_SYNC_WAIT_CEILING } else { OPENING_DIVIDER_WAIT_CEILING };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ceiling).await;
            let _ = this.update(cx, |screen, cx| {
                RiseColdOpenTrace::mark(|| {
                    format!("chat opening ceiling stillPending={}", screen.opening_load.as_ref() == Some(&ceiling_opening))
                });
                if screen.opening_load.as_ref() == Some(&ceiling_opening) {
                    screen.opening_load = None;
                    cx.notify();
                }
            });
        })
        .detach();
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let scroll_position_key = self.scroll_position_key(cx);
        let this = cx.weak_entity();
        let handle = window.window_handle();
        let completion_jump_id = opening_jump_message_id.clone();
        let jump_completion: MessageJumpCompletion = Box::new(move |outcome, cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |screen, cx| {
                    if outcome != RiseMessageJumpOutcome::Superseded {
                        ChatOpeningSyncedScopes::mark_synced(&opening.scope, cx);
                    }
                    if screen.opening_load.as_ref() == Some(&opening) {
                        screen.opening_load = None;
                        cx.notify();
                    }
                    let (Some(message_id), Some(token)) = (completion_jump_id, jump_token) else {
                        return;
                    };
                    let claims_screen = screen.pending_pinned_jump_id == Some(token.jump_id);
                    screen.finish_message_jump(&token, &message_id, outcome, window, cx);
                    if outcome != RiseMessageJumpOutcome::Installed && claims_screen {
                        screen.reopen_messages_after_abandoned_jump(&token.scope, window, cx);
                    }
                });
            });
        });
        message_interactions(cx, |store, cx| {
            store.load_initial_messages(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                Some(&scroll_position_key),
                Some(owner),
                opening_jump_message_id.as_deref(),
                Some(jump_completion),
                cx,
            );
        });
        self.completed_message_load_task_id = Some(self.message_load_task_id(cx));
        if let Some(context_preview) = self.context_preview.clone() {
            context_preview.update(cx, |session, _| session.opening_loaded_scope = Some(scope.clone()));
        }
    }

    pub(super) fn release_observed_unread_marks(&mut self, cx: &mut App) {
        if let Some((chat_id, topic_id)) = self.observed_unread_marks.take() {
            message_interactions(cx, |store, cx| store.release_unread_marks(&chat_id, topic_id.as_deref(), cx));
        }
    }

    pub(super) fn apply_sync_lifecycle(&mut self, frame: &ChatScreenFrame, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let chat = frame.chat.clone();
        let chat_id = self.resolved_chat_id(cx);
        Self::watch(&this, "chat-first-frame-task", self.first_frame_ready_scope.clone(), true, window, cx, |screen, _, _, _, cx| {
            screen.run_firstframe_task(cx);
        });
        let presence_owner = self.presence_observation_owner(cx);
        let is_direct = chat.r#type == ChatType::Direct;
        Self::watch(&this, "chat-participant", chat.participant.id.clone(), true, window, cx, move |_, _, peer_id, _, cx| {
            if !is_direct || peer_id.is_empty() {
                return;
            }
            let peers = [peer_id.clone()];
            PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.observe(&peers, &presence_owner, cx));
        });
        let block_peer = self.block_state_peer_user_id(cx);
        Self::watch(&this, "chat-block-peer", block_peer, true, window, cx, |_, _, peer_id, _, cx| {
            if peer_id.is_empty() {
                return;
            }
            let users = UserInteractionsStore::shared(cx);
            users.update(cx, |store, cx| store.refresh_block_state(peer_id, cx));
            let owned_by_guide = GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_user_id(peer_id));
            if !owned_by_guide {
                users.update(cx, |store, cx| store.refresh_foreign_profile_user_id(peer_id.clone(), cx));
            }
        });
        let on_removed = Rc::new(Self::act(this.clone(), |screen, window, cx| {
            screen.close_authoritatively_removed_chat(window, cx);
        }));
        ChatRemovalObserverModifier::observe("chat-removal", &chat_id, on_removed, window, cx);
        let notice = ChatServicesStore::shared(cx).read(cx).removed_topic_notice.clone();
        Self::watch(&this, "chat-removed-topic", notice, false, window, cx, |screen, _, notice, window, cx| {
            screen.leave_removed_topic_tab(notice.as_ref(), window, cx);
        });
        let bot_load_key = self.bot_chat_load_key(cx);
        Self::watch(&this, "chat-bot-load", bot_load_key, true, window, cx, |screen, _, _, _, cx| {
            screen.load_bot_chat_metadata(cx);
        });
        let current_track_id = ChatMusicHooks::current_track(cx).map(|track| track.id);
        Self::watch(&this, "chat-current-track", current_track_id, false, window, cx, |_screen, _, track_id, _, cx| {
            if track_id.is_none() {
                ChatMusicPlayerSheetLayer::dismiss(cx);
            }
        });
        let enter_key = format!(
            "{}|{}|{}|{}",
            chat_id,
            self.has_resolved_chat_metadata(cx),
            self.is_message_request(cx),
            self.is_context_preview(cx)
        );
        Self::watch(&this, "chat-enter", enter_key, true, window, cx, |screen, _, _, _, cx| {
            let chat_id = screen.resolved_chat_id(cx);
            if screen.is_context_preview(cx)
                || screen.resolved_chat(cx).r#type != ChatType::Direct
                || chat_id.is_empty()
                || screen.is_pending_chat(cx)
                || !screen.has_resolved_chat_metadata(cx)
                || screen.is_message_request(cx)
            {
                return;
            }
            PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.enter_chat(&chat_id, cx));
        });
        let gate = self.join_requests_gate_chat_id(cx);
        Self::watch(&this, "chat-join-gate", gate, true, window, cx, |_, _, gate, _, cx| {
            if !gate.is_empty() {
                chat_interactions(cx, |store, cx| store.load_join_requests(gate, cx));
            }
        });
        let mask_chat = self.permission_mask_load_chat_id(cx);
        Self::watch(&this, "chat-permission-mask", mask_chat, true, window, cx, |_, _, mask_chat, _, cx| {
            if !mask_chat.is_empty() {
                chat_interactions(cx, |store, cx| store.load_effective_permissions(mask_chat, None, false, cx));
            }
        });
        let topic_mask = self.composer_topic_mask_load_key(cx);
        Self::watch(&this, "chat-topic-mask", topic_mask, true, window, cx, |screen, _, key, _, cx| {
            let Some(topic_id) = screen.post_topic_id(cx).filter(|_| !key.is_empty()) else {
                return;
            };
            let chat_id = screen.resolved_chat_id(cx);
            chat_interactions(cx, |store, cx| store.load_effective_permissions(&chat_id, Some(&topic_id), false, cx));
        });
        let voice_chat = self.group_voice_observation_chat_id(cx);
        Self::watch(&this, "chat-group-voice", voice_chat, true, window, cx, |_, previous, current, _, cx| {
            if !previous.is_empty() && previous != current {
                chat_interactions(cx, |store, cx| store.end_observing_group_voice(previous, cx));
            }
            if !current.is_empty() {
                chat_interactions(cx, |store, cx| store.begin_observing_group_voice(current, cx));
            }
        });
        let account = account_id(cx);
        let trigger = Self::read_trigger(frame.latest_message.as_ref(), &frame.messages, &account);
        Self::watch(&this, "chat-read-trigger", trigger, false, window, cx, |screen, old, new, _, cx| {
            screen.handle_read_trigger(old, new, cx);
        });
        let backgrounded = RiseAppLifecycle::scene_phase(cx) == ScenePhase::Background;
        Self::watch(&this, "chat-return-from-background", backgrounded, false, window, cx, |screen, old, new, _, cx| {
            if *old
                && !*new
                && !screen.is_context_preview(cx)
                && !screen.is_message_request(cx)
                && !screen.is_community_outsider(cx)
                && !screen.is_hidden_under_sibling_topic(cx)
                && screen.message_viewport_chrome.read(cx).is_at_latest()
                && screen.open_scope_has_unread_incoming(cx)
            {
                screen.mark_open_chat_as_read(cx);
            }
        });
        let hidden_under_sibling = self.is_hidden_under_sibling_topic(cx);
        Self::watch(&this, "chat-sibling-topic-cover", hidden_under_sibling, false, window, cx, |screen, old, new, _, cx| {
            if *old && !*new && screen.message_viewport_chrome.read(cx).is_at_latest() && screen.active_unread_count(cx) > 0 {
                screen.mark_open_chat_as_read(cx);
            }
        });
        let active_unread = self.active_unread_count(cx);
        Self::watch(&this, "chat-unread-resend", active_unread, false, window, cx, |screen, old, new, _, cx| {
            let (old, new) = (*old, *new);
            if new <= 0 {
                screen.resent_bottom_read_count = None;
                return;
            }
            if new <= old
                || screen.resent_bottom_read_count == Some(new)
                || !screen.message_viewport_chrome.read(cx).is_at_latest()
                || screen.is_context_preview(cx)
                || screen.resolved_chat_id(cx).is_empty()
                || screen.is_message_request(cx)
                || screen.is_community_outsider(cx)
            {
                return;
            }
            let now = RiseClock::monotonic();
            if screen
                .last_bottom_read_resend
                .is_some_and(|(count, at)| count == new && now.saturating_sub(at) < Duration::from_secs(5))
            {
                return;
            }
            screen.resent_bottom_read_count = Some(new);
            screen.last_bottom_read_resend = Some((new, now));
            let chat_id = screen.resolved_chat_id(cx);
            let read_topic_id = screen.read_topic_id(cx);
            if let Some(task) = message_interactions(cx, |store, cx| {
                store.mark_chat_as_read(&chat_id, read_topic_id.as_deref(), false, cx)
            }) {
                task.detach();
            }
        });
        let divider_id = self.unread_divider_message_id(&frame.messages, cx);
        Self::watch(&this, "chat-divider", divider_id, true, window, cx, |screen, _, message_id, _, cx| {
            let scope = screen.message_scope(cx);
            if let Some(message_id) = message_id
                && screen.unread_divider.scope.as_deref() == Some(scope.as_str())
                && screen.unread_divider.message_id.is_none()
            {
                screen.unread_divider.message_id = Some(message_id.clone());
                cx.notify();
            }
        });
        let is_preview = self.is_context_preview(cx);
        let own_send = if is_preview { None } else { Self::latest_own_taken_send(&frame.messages, &account) };
        Self::watch(&this, "chat-own-send", own_send, true, window, cx, |screen, _, own_send, _, cx| {
            let chat_id = screen.resolved_chat_id(cx);
            if let Some(own_send) = own_send
                && !chat_id.is_empty()
            {
                let sent_at = own_send.sent_at;
                chat_interactions(cx, |store, cx| store.prime_slow_mode(&chat_id, sent_at, cx));
            }
        });
        let force_reply = if is_preview { None } else { frame.force_reply_message_id.clone() };
        let latest = frame.latest_message.clone();
        Self::watch(&this, "chat-force-reply", force_reply, true, window, cx, move |screen, _, message_id, window, cx| {
            let Some(message_id) = message_id else {
                return;
            };
            let Some(message) = latest.filter(|message| &message.id == message_id) else {
                return;
            };
            if screen.handled_bot_force_reply_message_id.as_ref() == Some(message_id) {
                return;
            }
            screen.handled_bot_force_reply_message_id = Some(message_id.clone());
            let chat = screen.resolved_chat(cx);
            let perms = MessagePermissions { message: &message, chat: &chat, is_own: false };
            screen.begin_reply(&message, perms, window, cx);
            screen.composer_focus_token += 1;
            cx.notify();
        });
        Self::watch(
            &this,
            "chat-picker-mode",
            self.state.media_picker_mode,
            false,
            window,
            cx,
            |screen, previous, mode, _, cx| {
                if screen.is_context_preview(cx) {
                    return;
                }
                let is_open = *mode == ChatMediaPickerMode::Attachments;
                GuideFacts::set(RiseGuideSignal::AttachmentPickerOpened, is_open, cx);
                let chat_id = screen.resolved_chat_id(cx);
                if (*previous == ChatMediaPickerMode::Attachments) != is_open
                    && !chat_id.is_empty()
                    && !screen.is_pending_chat(cx)
                {
                    message_interactions(cx, |store, cx| store.set_media_picker_typing(&chat_id, is_open, cx));
                }
            },
        );
        let topics_mode = self.is_topics_mode(cx);
        Self::watch(&this, "chat-topics-mode", topics_mode, false, window, cx, |screen, _, on, _, cx| {
            if *on && !screen.is_community_outsider(cx) {
                let chat_id = screen.resolved_chat_id(cx);
                chat_interactions(cx, |store, cx| {
                    store.load_topics(&chat_id, cx);
                    store.load_roles(&chat_id, cx);
                });
            }
        });
    }

    pub(super) fn run_firstframe_task(&mut self, cx: &mut Context<Self>) {
        let scope = self.message_scope(cx);
        if self.first_frame_ready_scope.as_deref() != Some(scope.as_str()) {
            return;
        }
        let uses_preview_layout = self.uses_context_preview_layout(cx);
        if !uses_preview_layout {
            ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| store.load(false, cx));
        }
        if self.initial_network_load_scope.as_deref() != Some(scope.as_str()) && !self.is_pending_chat(cx) {
            let chat_id = self.resolved_chat_id(cx);
            let topic_id = self.resolved_topic_id(cx);
            let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
            message_interactions(cx, |store, cx| {
                store.refresh_messages(&chat_id, topic_id.as_deref(), thread_root_id.as_deref(), cx);
            });
        }
        let chat = self.resolved_chat(cx);
        if !uses_preview_layout && chat.r#type == ChatType::Direct && !chat.is_bot_chat() {
            let peer_id = chat.participant.id.clone();
            let is_me = AuthServicesStore::shared(cx).read(cx).is_current_user(&peer_id);
            if !peer_id.is_empty() && !is_me {
                cx.spawn(async move |_, cx| {
                    GiftsActionsStore::load_tab(&peer_id, None, false, cx).await;
                })
                .detach();
            }
        }
    }

    pub(super) fn handle_read_trigger(&mut self, old: &ReadTrigger, new: &ReadTrigger, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if self.is_context_preview(cx) || chat_id.is_empty() || self.is_message_request(cx) || self.is_community_outsider(cx)
            || self.is_hidden_under_sibling_topic(cx)
        {
            return;
        }
        if new.latest_is_own && new.latest_id != old.latest_id {
            let account = account_id(cx);
            let messages = self.visible_messages(None, cx);
            if self.active_unread_count(cx) <= 0 && !Self::newest_incoming_is_unread(&messages, &account) {
                return;
            }
        } else if new.newest_incoming_id.is_some() && new.newest_incoming_id != old.newest_incoming_id {
            if !self.message_viewport_chrome.read(cx).is_at_latest() {
                return;
            }
        } else {
            return;
        }
        self.mark_open_chat_as_read(cx);
    }

    pub(super) fn mark_open_chat_as_read(&self, cx: &mut App) {
        let chat_id = self.resolved_chat_id(cx);
        let read_topic_id = self.read_topic_id(cx);
        if let Some(task) = message_interactions(cx, |store, cx| {
            store.mark_chat_as_read(&chat_id, read_topic_id.as_deref(), false, cx)
        }) {
            task.detach();
        }
    }

    pub(super) fn handle_disappear(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.is_on_screen = false;
        StickerPackChatSendTargets::unregister(cx.entity_id(), cx);
        if let Some(window) = window.as_deref_mut() {
            PageFocus::will_disappear(&self.page_focus, window, cx);
        }
        let is_preview = self.is_context_preview(cx);
        let is_transferring = self.context_preview_is_transferring(cx);
        let chat_id = self.resolved_chat_id(cx);
        if let (Some(root_id), Some(post)) =
            (self.resolved_thread_root_id().map(str::to_owned), self.thread_channel_post.clone())
        {
            let scope = self.message_scope(cx);
            message_services(cx, |store, cx| store.raise_channel_post_comments(&post, &root_id, &scope, cx));
        }
        if !is_preview {
            GuideFacts::set(RiseGuideSignal::ComposerRoleSuggested, false, cx);
            GuideFacts::set(RiseGuideSignal::ComposerEditingPhoto, false, cx);
            GuideFacts::set(RiseGuideSignal::ComposerPhotoPicked, false, cx);
        }
        if is_transferring {
            return;
        }
        let observed_voice = self.group_voice_observation_chat_id(cx);
        if !observed_voice.is_empty() {
            chat_interactions(cx, |store, cx| store.end_observing_group_voice(&observed_voice, cx));
        }
        self.release_observed_unread_marks(cx);
        if let Some(context_preview) = self.context_preview.clone()
            && context_preview.read(cx).is_preview()
        {
            self.completed_message_load_task_id = None;
            context_preview.update(cx, |session, _| session.opening_loaded_scope = None);
        }
        self.is_message_media_playback_active = false;
        let scope = self.message_scope(cx);
        if !is_preview {
            GuideFacts::set(RiseGuideSignal::ChatOpened, false, cx);
            GuideFacts::set(RiseGuideSignal::AttachmentPickerOpened, false, cx);
            if self.state.media_picker_mode == ChatMediaPickerMode::Attachments && !chat_id.is_empty() {
                message_interactions(cx, |store, cx| store.set_media_picker_typing(&chat_id, false, cx));
            }
            ChatDraftStore::shared(cx).update(cx, |store, cx| {
                store.publish_pending_previews(cx);
                store.flush_pending_to_disk(cx);
            });
            ChatPerformanceTrace::shared().stop_if_recording(&chat_id, "chat_disappear");
            let inline_scope = self.inline_composer_scope(cx);
            BotInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_inline_composer(&inline_scope, cx));
            self.reset_composer_input_surface(cx);
            message_interactions(cx, |store, cx| store.cancel_pending_latest_messages_jump(&scope, cx));
            ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.abandon(cx));
            chat_services(cx, |store, cx| store.restore_join_banner(&chat_id, cx));
            message_interactions(cx, |store, cx| store.stop_typing_if_active(&chat_id, cx));
        }
        self.pending_latest_jump_scope = None;
        self.pending_pinned_jump_id = None;
        self.message_list_scroll_request = None;
        self.show_chat_calendar = false;
        self.pending_chat_calendar_day_start_ms = None;
        self.pending_chat_calendar_day_installed = false;
        self.delivered_jump_message_id = None;
        let owner = self.presence_observation_owner(cx);
        PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.stop_observing(&owner, cx));
        if !is_preview
            && self.resolved_chat(cx).r#type == ChatType::Direct
            && !chat_id.is_empty()
            && !self.is_pending_chat(cx)
            && !self.is_message_request(cx)
        {
            PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.leave_chat(&chat_id, cx));
        }
        message_interactions(cx, |store, cx| store.checkpoint_messages(&scope, cx));
        let observation_owner = self.rise_message_observation_owner_id;
        let navigation = if window.is_some() && !is_preview && self.is_covered_in_stack(cx) { self.nav_store() } else { None };
        if let Some(navigation) = navigation {
            let chat_ids = self.route_chat_ids(cx);
            ChatCoveredFeedHold::park(
                scope,
                observation_owner,
                navigation,
                Box::new(move |path| path.iter().any(|route| Self::opens_chat(route, &chat_ids))),
                cx,
            );
        } else {
            message_interactions(cx, |store, cx| store.release_messages(&scope, Some(observation_owner), cx));
        }
        self.state.is_message_selection_active = false;
        self.state.selected_messages_by_server_id.clear();
        self.voice_hold_start_task = None;
        self.video_hold_start_task = None;
        let (voice_active, voice_locked) = {
            let recorder = self.voice_recorder.read(cx);
            (recorder.is_active(), recorder.is_locked())
        };
        if voice_active {
            match window {
                Some(window) if voice_locked && !is_preview => self.park_locked_voice_recording(window, cx),
                _ => self.voice_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx)),
            }
        }
        if self.video_note_recorder.read(cx).is_busy() {
            self.video_note_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx));
        }
        if !is_preview {
            self.stop_voice_draft_playback(cx);
            let player = VideoNotePlayerStore::shared(cx);
            let active_here = {
                let player = player.read(cx);
                player.is_active() && player.active_chat_id() == chat_id
            };
            if active_here {
                player.update(cx, |player, cx| player.enter_floating(cx));
            }
        }
    }

    pub(super) fn apply_composer_lifecycle(&mut self, _frame: &ChatScreenFrame, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let is_preview = self.is_context_preview(cx);
        Self::watch(
            &this,
            "chat-selected-delete-dialog",
            self.state.show_selected_messages_delete_dialog,
            false,
            window,
            cx,
            |screen, _, shown, window, cx| {
                if *shown {
                    screen.present_selected_messages_delete_dialog(window, cx);
                }
            },
        );
        Self::watch(
            &this,
            "chat-failed-send-dialog",
            self.state.failed_send_target.clone(),
            false,
            window,
            cx,
            |screen, _, target, window, cx| {
                if let Some(target) = target {
                    screen.present_failed_send_dialog(target, window, cx);
                }
            },
        );
        let composer_autofocus = self.is_on_screen && !is_preview && self.composer_field_reachable(cx);
        Self::watch(&this, "chat-composer-autofocus", composer_autofocus, true, window, cx, |screen, _, reachable, window, cx| {
            if *reachable {
                screen.focus_composer_field(window, cx);
            }
        });
        Self::watch(&this, "chat-input-text", self.state.input_text.clone(), false, window, cx, |screen, _, new_value, window, cx| {
            screen.handle_input_text_change(new_value, window, cx);
        });
        let draft_key = if is_preview { String::new() } else { self.composer_draft_scope_key(cx) };
        Self::watch(&this, "chat-draft-scope", draft_key, true, window, cx, |screen, previous, current, window, cx| {
            if !current.is_empty() {
                let chat_id = screen.resolved_chat_id(cx);
                let topic_id = if screen.resolved_thread_root_id().is_none() { screen.resolved_topic_id(cx) } else { None };
                message_interactions(cx, |store, cx| store.set_typing_topic(&chat_id, topic_id.as_deref(), cx));
                screen.handle_composer_draft_scope_change(previous, current, window, cx);
            }
        });
        let key = self.scroll_position_key(cx);
        Self::watch(&this, "chat-scroll-key", key, true, window, cx, |screen, _, key, _, cx| {
            let scope = screen.message_scope(cx);
            message_services(cx, |store, _| store.set_scroll_position_key(key, &scope));
        });
        let preference = self.link_preview_draft.read(cx).preference();
        Self::watch(&this, "chat-link-preference", preference, false, window, cx, |screen, _, _, window, cx| {
            let text = screen.state.input_text.clone();
            screen.persist_composer_draft(&text, None, window, cx);
        });
        let switched_off = !is_preview && self.link_previews_switched_off(cx);
        Self::watch(&this, "chat-link-blocked", switched_off, true, window, cx, |screen, _, switched_off, _, cx| {
            let switched_off = *switched_off;
            screen.link_preview_draft.update(cx, |draft, cx| draft.set_blocked_by_chat(switched_off, cx));
        });
        Self::watch(&this, "chat-composer-selection", self.composer_selection, false, window, cx, |screen, _, _, window, cx| {
            let text = screen.state.input_text.clone();
            screen.refresh_inline_composer_context(&text, window, cx);
        });
        let offers_roles = self.composer_offers_roles(cx);
        Self::watch(&this, "chat-offers-roles", offers_roles, true, window, cx, |_, _, offered, _, cx| {
            GuideFacts::set(RiseGuideSignal::ComposerRoleSuggested, *offered, cx);
        });
        let editing = !is_preview && self.state.edit_target.as_ref().is_some_and(|target| target.can_reedit_media);
        Self::watch(&this, "chat-editing-photo", editing, true, window, cx, |_, _, editing, _, cx| {
            GuideFacts::set(RiseGuideSignal::ComposerEditingPhoto, *editing, cx);
        });
        let picked = !is_preview && self.pending_reedit_item.is_some();
        Self::watch(&this, "chat-photo-picked", picked, true, window, cx, |_, _, picked, _, cx| {
            GuideFacts::set(RiseGuideSignal::ComposerPhotoPicked, *picked, cx);
        });
    }

    pub(super) fn handle_input_text_change(&mut self, new_value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_context_preview(cx) {
            return;
        }
        if self.state.edit_target.is_none() {
            self.link_preview_draft.update(cx, |draft, cx| draft.update(new_value, cx));
        }
        self.inline_user_mentions = ChatComposerMentionEntityCodec::rebased(
            &self.inline_user_mentions,
            &self.inline_user_mentions_anchor_text,
            new_value,
        );
        self.inline_user_mentions_anchor_text = new_value.to_owned();
        self.refresh_inline_composer_context(new_value, window, cx);
        self.persist_composer_draft(new_value, None, window, cx);
        self.refresh_composer_overflow(new_value, window, cx);
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() || self.is_pending_chat(cx) || self.resolved_chat(cx).r#type == ChatType::Channel {
            return;
        }
        if self.restored_draft_text.as_deref() == Some(new_value) {
            self.restored_draft_text = None;
            return;
        }
        let is_typing = !new_value.is_empty();
        message_interactions(cx, |store, cx| store.set_typing(&chat_id, is_typing, ChatAction::Typing, cx));
    }

    pub(super) fn handle_did_enter_background(&mut self, cx: &mut Context<Self>) {
        let picker_settled = self.picker_visible && self.composer_input_surface == ChatComposerInputSurface::Picker;
        if !picker_settled {
            self.reset_composer_input_surface(cx);
        }
        self.voice_recorder.update(cx, |recorder, cx| recorder.pause_for_background(cx));
        self.video_note_recorder.update(cx, |recorder, cx| recorder.pause_for_background(cx));
        ChatDraftStore::shared(cx).update(cx, |store, cx| {
            store.publish_pending_previews(cx);
            store.flush_pending_to_disk(cx);
        });
    }

    pub(super) fn handle_call_took_audio_session(&mut self, cx: &mut Context<Self>) {
        self.voice_recorder.update(cx, |recorder, cx| recorder.pause_for_background(cx));
        self.video_note_recorder.update(cx, |recorder, cx| recorder.pause_for_interruption(cx));
    }
}

impl Render for ChatScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui::hear_entity_through_events_only(RiseConversationPresentation::shared(cx).entity_id());
        if let Some(nav_store) = env::<NavigationStoreKey>(cx) {
            self.navigation_store = Some(nav_store);
        }
        let frame = Rc::new(self.bodyframe(cx));
        self.apply_load_lifecycle(&frame, window, cx);
        self.apply_sync_lifecycle(&frame, window, cx);
        self.apply_composer_lifecycle(&frame, window, cx);
        let first_pass = !self.has_rendered;
        self.has_rendered = true;
        CHAT_SCREEN_FIRST_PASS.set(first_pass);
        let shell = self.shell(&frame, window, cx);
        let overlays = self.presented_overlays(&frame, window, cx);
        CHAT_SCREEN_FIRST_PASS.set(false);
        let this = cx.weak_entity();
        let appearing = cx.entity();
        let disappearing = cx.entity();
        let accepts_dropped_files = self.composer_accepts_dropped_files(&frame, cx);
        let drop_target = cx.weak_entity();
        let click_focus = self.focus_handle.clone();
        let press_focus = self.focus_handle.clone();
        let press_composer = cx.weak_entity();
        let hears_window_presses = self.is_on_screen;
        div()
            .id("chat-screen")
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |screen, cx| screen.handle_key_down(event, window, cx));
            })
            .capture_any_mouse_down(move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                if !click_focus.contains_focused(window, cx) || ScreenFocus::text_field_is_focused(window, cx) {
                    ScreenFocus::claim(&click_focus, window, cx);
                }
            })
            .when(accepts_dropped_files, move |root| {
                root.on_drop(move |paths: &ExternalPaths, window: &mut Window, cx: &mut App| {
                    let paths = paths.paths().to_vec();
                    let _ = drop_target.update(cx, |screen, cx| screen.handle_dropped_files(paths, window, cx));
                })
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        if !hears_window_presses {
                            return;
                        }
                        let press_composer = press_composer.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture || !bounds.contains(&event.position) {
                                return;
                            }
                            if ScreenFocus::text_field_is_focused(window, cx)
                                || ScreenFocus::is_idle(window, cx)
                                || press_focus.contains_focused(window, cx)
                            {
                                ScreenFocus::claim(&press_focus, window, cx);
                            }
                            let settle_focus = press_focus.clone();
                            let composer = press_composer.clone();
                            let primary_press = event.button == MouseButton::Left;
                            window.defer(cx, move |window, cx| {
                                let orphaned = ScreenFocus::is_idle(window, cx)
                                    || window.focused(cx).is_some_and(|focused| !focused.is_in_rendered_frame(window));
                                if orphaned && ScreenFocus::top_sheet(window, cx).is_none() {
                                    ScreenFocus::claim(&settle_focus, window, cx);
                                }
                                if primary_press && settle_focus.is_focused(window) {
                                    let _ = composer.update(cx, |screen, cx| screen.focus_composer_field(window, cx));
                                }
                            });
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(shell)
            .children(overlays)
            .on_appear("chat-screen-lifecycle", move |window, cx| {
                appearing.update(cx, |screen, cx| screen.handle_appear(window, cx));
            })
            .and_on_disappear(move |window, cx| {
                disappearing.update(cx, |screen, cx| screen.handle_disappear(window, cx));
            })
    }
}

/// keeps a covered chat feed observed while its route stays on the stack
/// releases it once the route leaves without the chat showing again
pub(super) struct ChatCoveredFeedHold {
    pub(super) scope: String,
    pub(super) owner_id: Uuid,
    pub(super) keeps_route: Box<dyn Fn(&[RootRoute]) -> bool>,
    pub(super) navigation: Entity<NavigationStore>,
    pub(super) _observation: Option<Subscription>,
}

thread_local! {
    static CHAT_SCREEN_FIRST_PASS: Cell<bool> = const { Cell::new(false) };
    static CHAT_COVERED_FEED_HOLDS: RefCell<HashMap<Uuid, Rc<RefCell<ChatCoveredFeedHold>>>> = RefCell::new(HashMap::new());
}

impl ChatCoveredFeedHold {
    pub(super) fn park(
        scope: String,
        owner_id: Uuid,
        navigation: Entity<NavigationStore>,
        keeps_route: Box<dyn Fn(&[RootRoute]) -> bool>,
        cx: &mut App,
    ) {
        let previous = CHAT_COVERED_FEED_HOLDS.with_borrow(|parked| parked.get(&owner_id).cloned());
        if let Some(previous) = previous
            && previous.borrow().scope != scope
        {
            Self::release(&previous, cx);
        }
        let hold = Rc::new(RefCell::new(Self {
            scope,
            owner_id,
            keeps_route,
            navigation: navigation.clone(),
            _observation: None,
        }));
        CHAT_COVERED_FEED_HOLDS.with_borrow_mut(|parked| parked.insert(owner_id, hold.clone()));
        let weak = Rc::downgrade(&hold);
        let observation = cx.observe(&navigation, move |_, cx| {
            if let Some(hold) = weak.upgrade() {
                Self::check(&hold, cx);
            }
        });
        hold.borrow_mut()._observation = Some(observation);
        Self::check(&hold, cx);
    }

    /// chat is on screen again, its own load and disappearance own the feed from here
    pub(super) fn resume(owner_id: Uuid) {
        CHAT_COVERED_FEED_HOLDS.with_borrow_mut(|parked| parked.remove(&owner_id));
    }

    pub(super) fn check(hold: &Rc<RefCell<Self>>, cx: &mut App) {
        let owner_id = hold.borrow().owner_id;
        let is_parked = CHAT_COVERED_FEED_HOLDS
            .with_borrow(|parked| parked.get(&owner_id).is_some_and(|parked| Rc::ptr_eq(parked, hold)));
        if !is_parked {
            return;
        }
        let keeps = {
            let hold = hold.borrow();
            (hold.keeps_route)(&hold.navigation.read(cx).path)
        };
        if !keeps {
            Self::release(hold, cx);
        }
    }

    pub(super) fn release(hold: &Rc<RefCell<Self>>, cx: &mut App) {
        let (scope, owner_id) = {
            let hold = hold.borrow();
            (hold.scope.clone(), hold.owner_id)
        };
        CHAT_COVERED_FEED_HOLDS.with_borrow_mut(|parked| {
            if parked.get(&owner_id).is_some_and(|parked| Rc::ptr_eq(parked, hold)) {
                parked.remove(&owner_id);
            }
        });
        message_interactions(cx, |store, cx| store.release_messages(&scope, Some(owner_id), cx));
    }
}

impl ChatScreen {
    /// store completions may fire inside the store own call while this screen is updating
    /// so they land on the next turn of the loop
    pub(super) fn deliver(
        this: &WeakEntity<Self>,
        cx: &mut App,
        action: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let this = this.clone();
        cx.defer(move |cx| {
            let _ = this.update(cx, action);
        });
    }

    pub(super) fn deliver_in_window(
        this: &WeakEntity<Self>,
        handle: AnyWindowHandle,
        cx: &mut App,
        action: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let this = this.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |screen, cx| action(screen, window, cx));
            });
        });
    }

    /// transition of a view only built while shown
    /// the slot starts empty so the first frame plays the insertion
    /// unless it is there on the screen first frame
    pub(super) fn appearing<E: Styled + IntoElement>(
        id: &'static str,
        transition: Transition,
        spec: MotionSpec,
        element: E,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let now = RiseClock::monotonic();
        let born = CHAT_SCREEN_FIRST_PASS.get().then_some(());
        let presence = window.use_keyed_state(id, cx, move |_, _| Presence::new(born).with_transition(transition));
        if !presence.read(cx).is_present() {
            tracing::warn!(target: "riseonly::X1267", id, "chat chrome slot appears after the first pass");
            presence.update(cx, |presence, cx| presence.set_in(Some(()), spec, now, cx));
        }
        presence
            .read(cx)
            .render_transitioned(now, window, move |_| element)
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| Empty.into_any_element())
    }

    pub(super) fn load_initial_messages(&self, cx: &mut App) {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let scroll_position_key = self.scroll_position_key(cx);
        let owner_id = self.rise_message_observation_owner_id;
        message_interactions(cx, |store, cx| {
            store.load_initial_messages(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                Some(scroll_position_key.as_str()),
                Some(owner_id),
                None,
                None,
                cx,
            );
        });
    }

    pub(super) fn media_send_completion() -> MessageActionCompletion {
        Box::new(|result, cx: &mut App| Self::handle_media_send_completion(result, cx))
    }
}

#[derive(IntoElement)]
pub(super) struct ChatMessagesLoadErrorState {
    pub(super) text: String,
    pub(super) theme: AppTheme,
    pub(super) on_retry: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl RenderOnce for ChatMessagesLoadErrorState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let primary = self.theme.text.primary;
        let retry = self.on_retry.map(|on_retry| {
            LiquidGlassButton::new("chat-messages-load-retry", LiquidGlassButtonShape::Capsule)
                .accessibility_label(Some(tr("common_retry")))
                .on_press(move |window, cx| on_retry(window, cx))
                .child(MainText::new(tr("common_retry")).px(14.0).color(primary))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.0))
            .px(px(24.0))
            .child(MainText::new(self.text).px(16.0).tac(TextAlignT::Center).color(primary))
            .children(retry)
    }
}

/// what the chat header subtitle shows, resolved outside render
#[derive(Clone, PartialEq)]
pub(super) enum ChatHeaderSubtitleText {
    Status { text: String, accent: bool },
    Typing { text: String, trailing: Option<String> },
}

#[derive(Clone, PartialEq)]
pub(super) struct ChatHeaderSubtitleModel {
    pub(super) text: ChatHeaderSubtitleText,
    pub(super) refresh_timestamp: Option<TimeInterval>,
}

pub(super) struct ChatHeaderScopedContent {
    chat_id: String,
    screen: Option<WeakEntity<ChatScreen>>,
    model: Option<Rc<dyn Fn(&mut App) -> Option<ChatHeaderSubtitleModel>>>,
    shown: Option<ChatHeaderSubtitleModel>,
    resolve_scheduled: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChatHeaderScopedContent {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let messages = MessageServicesStore::shared(cx);
        let presence = PresenceServicesStore::shared(cx);
        let socket = RiseTransportStatus::shown_store(cx);
        let maintenance = MaintenanceGate::shared(cx);
        Self {
            chat_id: String::new(),
            screen: None,
            model: None,
            shown: None,
            resolve_scheduled: false,
            _subscriptions: vec![
                cx.observe(&messages, |scope, _, cx| scope.wake(cx)),
                cx.observe(&presence, |scope, _, cx| scope.wake(cx)),
                cx.observe(&socket, |scope, _, cx| scope.wake(cx)),
                cx.observe(&maintenance, |scope, _, cx| scope.wake(cx)),
            ],
        }
    }

    pub(super) fn configure(
        &mut self,
        chat_id: &str,
        screen: WeakEntity<ChatScreen>,
        model: Rc<dyn Fn(&mut App) -> Option<ChatHeaderSubtitleModel>>,
        cx: &mut Context<Self>,
    ) {
        if self.chat_id != chat_id {
            self.chat_id = chat_id.to_owned();
        }
        self.screen = Some(screen);
        self.model = Some(model);
        if self.resolve_scheduled {
            return;
        }
        self.resolve_scheduled = true;
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |scope, cx| {
                scope.resolve_scheduled = false;
                scope.wake(cx);
            });
        });
    }

    fn wake(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let Some(next) = model(cx) else {
            return;
        };
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        let text_unchanged = self.shown.as_ref().is_some_and(|shown| shown.text == next.text);
        let refresh_timestamp = next.refresh_timestamp;
        self.shown = Some(next);
        if text_unchanged {
            if let Some(screen) = self.screen.as_ref() {
                let _ = screen.update(cx, |screen, cx| screen.wait_for_header_refresh(refresh_timestamp, cx));
            }
            return;
        }
        cx.notify();
    }
}

impl Render for ChatHeaderScopedContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.shown.is_none() {
            self.shown = self.model.clone().and_then(|model| model(cx));
        }
        let Some(shown) = self.shown.clone() else {
            return Empty.into_any_element();
        };
        if let Some(screen) = self.screen.as_ref() {
            ChatScreen::watch(screen, "chat-header-refresh", shown.refresh_timestamp, true, window, cx, |screen, _, timestamp, _, cx| {
                screen.wait_for_header_refresh(*timestamp, cx);
            });
        }
        ChatScreen::header_subtitle_element(&shown, &self.chat_id, cx)
    }
}

pub(super) fn bot_interactions<R>(
    cx: &mut App,
    run: impl FnOnce(&mut BotInteractionsStore, &mut Context<BotInteractionsStore>) -> R,
) -> R {
    BotInteractionsStore::shared(cx).update(cx, run)
}

pub(super) fn draft_store<R>(cx: &mut App, run: impl FnOnce(&mut ChatDraftStore, &mut Context<ChatDraftStore>) -> R) -> R {
    ChatDraftStore::shared(cx).update(cx, run)
}

pub(super) fn send_failed_notice(title_key: &str, content: Option<String>, cx: &mut App) {
    Notifier::send(
        tr(title_key),
        NotifierSendOptions {
            content,
            r#type: NotifierType::Error,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        },
        cx,
    );
}

pub(super) const COMPOSER_CUSTOM_EMOJI_PLACEHOLDER: &str = "\u{FFFC}";
pub(super) const DEFAULT_TEXT_FILE_NAME: &str = "Text.txt";


impl ChatScreen {
    pub(super) fn delete_message_ids(&self, ids: &[String], for_everyone: bool, cx: &mut App) {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        message_interactions(cx, |store, cx| {
            store.delete_messages(ids, &chat_id, topic_id.as_deref(), thread_root_id.as_deref(), for_everyone, cx);
        });
    }

    pub(super) fn cancel_composer_reply_or_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.edit_target.is_some() {
            self.cancel_edit(window, cx);
        } else if self.state.reply_target.is_some() {
            self.cancel_reply(cx);
        }
    }

    pub(super) fn member_attachments_locked(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        chat_services(cx, |store, cx| store.member_attachments_switched_off(&chat_id, post_topic_id.as_deref(), cx))
    }

    pub(super) fn composer_accepts_dropped_files(&self, frame: &ChatScreenFrame, cx: &mut App) -> bool {
        self.state.media_picker_mode == ChatMediaPickerMode::None
            && self.state.edit_target.is_none()
            && !self.state.is_message_search_active
            && !self.state.is_message_selection_active
            && !frame.peer_blocked
            && !self.resolved_chat_id(cx).is_empty()
            && !self.is_message_request(cx)
            && !self.chat_unavailable(cx)
            && !self.chat_metadata_loading(cx)
            && !self.viewer_is_outside(&frame.chat, cx)
            && !self.channel_posting_locked(cx)
            && !self.topic_posting_locked(cx)
    }

    pub(super) fn handle_dropped_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        match file_drop::evaluate(&paths, |path| path.is_file()) {
            DropOutcome::Accepted(paths) => {
                if self.member_attachments_locked(cx) {
                    chat_interactions(cx, |store, cx| store.member_send_refused(ErrorCode::ChatSendRestricted.wire(), cx));
                    return;
                }
                *self.dropped_files.borrow_mut() = paths;
                resign_chat_keyboard(window, cx);
                self.state.media_picker_mode = ChatMediaPickerMode::Attachments;
                cx.notify();
            }
            DropOutcome::Rejected(DropRejection::Empty) => {}
            DropOutcome::Rejected(DropRejection::TooMany { limit, .. }) => {
                let title = tr_args("share_error_too_many_items", &[FormatArg::Int(limit as i64)]);
                Notifier::send(title, NotifierSendOptions::default(), cx);
            }
            DropOutcome::Rejected(DropRejection::NotAFile(_)) => {
                Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx);
            }
        }
    }

    pub(super) fn pasted_media_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        let presentable = !self.pasted_media.is_empty();
        let (target, duration) = if presentable {
            (1.0, ModalSheetMetrics::PRESENT_DURATION)
        } else {
            (0.0, ModalSheetMetrics::DISMISS_DURATION)
        };
        if *self.pasted_media_presented.target() != target {
            self.pasted_media_presented.set_in(target, ease_out(duration), now, cx);
        }
        let fraction = self.pasted_media_presented.frame_value(now, window);
        if !presentable && !self.pasted_media_presented.is_running(now) {
            self.pasted_media_shown.clear();
            return None;
        }
        let sheet = self.pasted_media_sheet(cx).appearance(fraction, presentable);
        Some(div().absolute().inset_0().child(sheet).into_any_element())
    }

    pub(super) fn pasted_media_sheet(&mut self, cx: &mut Context<Self>) -> ChatPasteMediaSheet {
        let entity = cx.entity();
        let close_this = cx.weak_entity();
        let send_this = cx.weak_entity();
        ChatPasteMediaSheet::new(
            "chat-paste-media",
            self.pasted_media_shown.clone(),
            ChatDecorativeBackdrop::new().preview_preset(Some(ChatWallpaperPreset::default_preset().clone())),
            Binding::entity(
                entity.clone(),
                |screen: &Self| screen.state.input_text.clone(),
                |screen, value, cx| {
                    screen.state.input_text = value;
                    cx.notify();
                },
            ),
            move |_, cx| {
                let _ = close_this.update(cx, |screen, cx| screen.close_pasted_media(cx));
            },
            move |as_file, spoiler, window, cx| {
                let _ = send_this.update(cx, |screen, cx| screen.send_pasted_media(as_file, spoiler, window, cx));
            },
        )
        .caption_custom_emojis(Some(Binding::entity(
            entity,
            |screen: &Self| screen.draft_custom_emojis.clone(),
            |screen, value, cx| {
                screen.draft_custom_emojis = value;
                cx.notify();
            },
        )))
    }
}

impl ChatScreen {
    /// row below a plate that replaces the composer, same insets as the composer
    /// so the feed doesnt move when one takes the other place
    pub(super) fn composer_plate_row(&self, plate: AnyElement, safe_bottom_inset: f32, cx: &App) -> AnyElement {
        div()
            .w_full()
            .flex_none()
            .flex()
            .justify_center()
            .px(px(ChatBottomBarMetrics::H_PADDING))
            .pt(px(ChatBottomBarMetrics::TOP_PADDING))
            .pb(px(self.composer_bottom_padding(safe_bottom_inset, cx)))
            .child(plate)
            .into_any_element()
    }

    pub(super) fn locked_pill(icon: &'static str, title: String, theme: &AppTheme) -> AnyElement {
        let secondary = theme.text.secondary;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.0))
            .px(px(16.0))
            .h(px(ChatBottomBarMetrics::LOCKED_PILL_HEIGHT))
            .children(IconUi::sized(icon, px(12.0), secondary, false))
            .child(MainText::new(title).px(14.0).font_weight(FontWeight::MEDIUM).color(secondary).number_of_lines(1))
            .liquid_chrome_capsule_with(0.12, true, None, false)
    }
}
