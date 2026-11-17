use futures::FutureExt;

use crate::modules::chat::stores::chats::chat_actions::forum_voice_roster_hooks::{
    ForumVoiceRosterHooks, ForumVoiceRosterSource, set_forum_voice_roster_hooks,
};
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCActionExecutor;

pub fn install_forum_voice_roster_hooks() {
    set_forum_voice_roster_hooks(ForumVoiceRosterHooks {
        require_source: |cx| {
            let domain = RiseRTCActionExecutor::require_domain(cx);
            async move {
                let rtc = domain.await.ok()?;
                let account_id = rtc.account_id.clone();
                Some(ForumVoiceRosterSource {
                    account_id,
                    list_channel_participants: Box::new(
                        move |room_id: String, cursor: Option<String>, page_limit: i64| {
                            let rtc = rtc.clone();
                            async move {
                                rtc.repository.list_channel_participants(&room_id, cursor, page_limit).await
                            }
                            .boxed()
                        },
                    ),
                })
            }
            .boxed_local()
        },
    });
}
