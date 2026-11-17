use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer_hooks::{
    TopicMentionComposerHooks, set_topic_mention_composer_hooks,
};
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;

pub fn install_topic_mention_composer_hooks() {
    set_topic_mention_composer_hooks(TopicMentionComposerHooks {
        present_room: |target, cx| {
            RTCInteractionsStore::shared(cx).update(cx, |store, cx| store.present_room(target, cx));
        },
    });
}
