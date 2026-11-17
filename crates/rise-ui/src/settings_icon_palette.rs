use std::sync::LazyLock;

use gpui::Hsla;
use rise_canon::rise_theme_tokens::SettingsIcon;
use rise_theme::color;

/// settings icon backgrounds come from canon and not local literals, the same 36 colors Android draws
/// parsed once per launch, not on every redraw
pub struct SettingsIconPalette {
    profile: Hsla,
    achievements: Hsla,
    privacy: Hsla,
    sessions: Hsla,
    blocks: Hsla,
    goals_plans: Hsla,
    notifications: Hsla,
    system_notifications: Hsla,
    quick_reaction: Hsla,
    subscription: Hsla,
    customization: Hsla,
    memory: Hsla,
    moderation: Hsla,
    language: Hsla,
    reset: Hsla,
    stickers: Hsla,
    chat_folders: Hsla,
    logout: Hsla,
    sparks: Hsla,
    delete_account: Hsla,
    legal: Hsla,
    privacy_phone: Hsla,
    privacy_avatar: Hsla,
    privacy_description: Hsla,
    privacy_birthday: Hsla,
    privacy_friends: Hsla,
    privacy_plans: Hsla,
    privacy_goals: Hsla,
    privacy_achievements: Hsla,
    privacy_messages: Hsla,
    privacy_last_seen: Hsla,
    privacy_online_status: Hsla,
    privacy_in_chat: Hsla,
    privacy_sparks: Hsla,
    privacy_gifts: Hsla,
    moderation_become: Hsla,
    moderation_appeals: Hsla,
    memory_videos: Hsla,
    memory_images: Hsla,
    memory_photos: Hsla,
    memory_files: Hsla,
    memory_stories: Hsla,
    memory_audio: Hsla,
    memory_other: Hsla,
}

static PALETTE: LazyLock<SettingsIconPalette> = LazyLock::new(|| SettingsIconPalette {
    profile: color(SettingsIcon::Profile.hex()),
    achievements: color("#F5A623"),
    privacy: color(SettingsIcon::Privacy.hex()),
    sessions: color(SettingsIcon::Sessions.hex()),
    blocks: color(SettingsIcon::Blocks.hex()),
    goals_plans: color(SettingsIcon::GoalsPlans.hex()),
    notifications: color(SettingsIcon::Notifications.hex()),
    system_notifications: color("#8E8E93"),
    quick_reaction: color(SettingsIcon::QuickReaction.hex()),
    subscription: color(SettingsIcon::Subscription.hex()),
    customization: color(SettingsIcon::Customization.hex()),
    memory: color(SettingsIcon::Memory.hex()),
    moderation: color(SettingsIcon::Moderation.hex()),
    language: color(SettingsIcon::Language.hex()),
    reset: color(SettingsIcon::Reset.hex()),
    stickers: color(SettingsIcon::Stickers.hex()),
    chat_folders: color(SettingsIcon::ChatFolders.hex()),
    logout: color(SettingsIcon::Logout.hex()),
    sparks: color(SettingsIcon::Sparks.hex()),
    delete_account: color("#FF3B30"),
    legal: color("#8E8E93"),
    privacy_phone: color(SettingsIcon::PrivacyPhone.hex()),
    privacy_avatar: color(SettingsIcon::PrivacyAvatar.hex()),
    privacy_description: color(SettingsIcon::PrivacyDescription.hex()),
    privacy_birthday: color(SettingsIcon::PrivacyBirthday.hex()),
    privacy_friends: color(SettingsIcon::PrivacyFriends.hex()),
    privacy_plans: color(SettingsIcon::PrivacyPlans.hex()),
    privacy_goals: color(SettingsIcon::PrivacyGoals.hex()),
    privacy_achievements: color("#E9C300"),
    privacy_messages: color(SettingsIcon::PrivacyMessages.hex()),
    privacy_last_seen: color(SettingsIcon::PrivacyLastSeen.hex()),
    privacy_online_status: color(SettingsIcon::PrivacyOnlineStatus.hex()),
    privacy_in_chat: color(SettingsIcon::PrivacyInChat.hex()),
    privacy_sparks: color(SettingsIcon::PrivacySparks.hex()),
    privacy_gifts: color("#FF4F79"),
    moderation_become: color(SettingsIcon::ModerationBecome.hex()),
    moderation_appeals: color(SettingsIcon::ModerationAppeals.hex()),
    memory_videos: color(SettingsIcon::MemoryVideos.hex()),
    memory_images: color(SettingsIcon::MemoryImages.hex()),
    memory_photos: color(SettingsIcon::MemoryPhotos.hex()),
    memory_files: color(SettingsIcon::MemoryFiles.hex()),
    memory_stories: color(SettingsIcon::MemoryStories.hex()),
    memory_audio: color(SettingsIcon::MemoryAudio.hex()),
    memory_other: color(SettingsIcon::MemoryOther.hex()),
});

impl SettingsIconPalette {
    pub fn profile() -> Hsla {
        PALETTE.profile
    }

    pub fn achievements() -> Hsla {
        PALETTE.achievements
    }

    pub fn privacy() -> Hsla {
        PALETTE.privacy
    }

    pub fn sessions() -> Hsla {
        PALETTE.sessions
    }

    pub fn blocks() -> Hsla {
        PALETTE.blocks
    }

    pub fn goals_plans() -> Hsla {
        PALETTE.goals_plans
    }

    pub fn notifications() -> Hsla {
        PALETTE.notifications
    }

    pub fn system_notifications() -> Hsla {
        PALETTE.system_notifications
    }

    pub fn quick_reaction() -> Hsla {
        PALETTE.quick_reaction
    }

    pub fn subscription() -> Hsla {
        PALETTE.subscription
    }

    pub fn customization() -> Hsla {
        PALETTE.customization
    }

    pub fn memory() -> Hsla {
        PALETTE.memory
    }

    pub fn moderation() -> Hsla {
        PALETTE.moderation
    }

    pub fn language() -> Hsla {
        PALETTE.language
    }

    pub fn reset() -> Hsla {
        PALETTE.reset
    }

    pub fn stickers() -> Hsla {
        PALETTE.stickers
    }

    pub fn chat_folders() -> Hsla {
        PALETTE.chat_folders
    }

    pub fn logout() -> Hsla {
        PALETTE.logout
    }

    pub fn sparks() -> Hsla {
        PALETTE.sparks
    }

    pub fn delete_account() -> Hsla {
        PALETTE.delete_account
    }

    pub fn legal() -> Hsla {
        PALETTE.legal
    }

    pub fn privacy_phone() -> Hsla {
        PALETTE.privacy_phone
    }

    pub fn privacy_avatar() -> Hsla {
        PALETTE.privacy_avatar
    }

    pub fn privacy_description() -> Hsla {
        PALETTE.privacy_description
    }

    pub fn privacy_birthday() -> Hsla {
        PALETTE.privacy_birthday
    }

    pub fn privacy_friends() -> Hsla {
        PALETTE.privacy_friends
    }

    pub fn privacy_plans() -> Hsla {
        PALETTE.privacy_plans
    }

    pub fn privacy_goals() -> Hsla {
        PALETTE.privacy_goals
    }

    pub fn privacy_achievements() -> Hsla {
        PALETTE.privacy_achievements
    }

    pub fn privacy_messages() -> Hsla {
        PALETTE.privacy_messages
    }

    pub fn privacy_last_seen() -> Hsla {
        PALETTE.privacy_last_seen
    }

    pub fn privacy_online_status() -> Hsla {
        PALETTE.privacy_online_status
    }

    pub fn privacy_in_chat() -> Hsla {
        PALETTE.privacy_in_chat
    }

    pub fn privacy_sparks() -> Hsla {
        PALETTE.privacy_sparks
    }

    pub fn privacy_gifts() -> Hsla {
        PALETTE.privacy_gifts
    }

    pub fn moderation_become() -> Hsla {
        PALETTE.moderation_become
    }

    pub fn moderation_appeals() -> Hsla {
        PALETTE.moderation_appeals
    }

    pub fn memory_videos() -> Hsla {
        PALETTE.memory_videos
    }

    pub fn memory_images() -> Hsla {
        PALETTE.memory_images
    }

    pub fn memory_photos() -> Hsla {
        PALETTE.memory_photos
    }

    pub fn memory_files() -> Hsla {
        PALETTE.memory_files
    }

    pub fn memory_stories() -> Hsla {
        PALETTE.memory_stories
    }

    pub fn memory_audio() -> Hsla {
        PALETTE.memory_audio
    }

    pub fn memory_other() -> Hsla {
        PALETTE.memory_other
    }
}
