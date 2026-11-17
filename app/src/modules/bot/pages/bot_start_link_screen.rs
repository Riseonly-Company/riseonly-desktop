use std::rc::Rc;

use gpui::{App, Context, ElementId, SharedString, Task, Window};
use rise_i18n::tr;
use rise_ui::swiftui::environment_values::IsEnabledKey;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::bot::components::bot_ui_components::BotStateView;
use crate::modules::bot::shared::bot_start_payload::BotStartPayload;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotModel;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatStartCard;
use crate::modules::chat::engine::rise_conversation_bot_links::RiseBotStartLinkError;

fn page_id(tag: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("bot-start-link-{tag}-{name}")))
}

/// bot start link screen, started and the resolved bot must outlive a push from above while
/// the deep link is on the stack
pub struct BotStartLinkScreen {
    tag: String,
    start_param: Option<String>,
    bot: Option<BotModel>,
    failed: bool,
    moderated: bool,
    started: bool,
    attempt: i64,
    account_id: Option<String>,
    account_epoch: Option<String>,
}

impl BotStartLinkScreen {
    pub fn new(tag: String, start_param: Option<String>, _cx: &mut Context<Self>) -> Self {
        Self {
            tag,
            start_param,
            bot: None,
            failed: false,
            moderated: false,
            started: false,
            attempt: 0,
            account_id: None,
            account_epoch: None,
        }
    }

    fn begin_resolve(&mut self, cx: &mut Context<Self>) -> Option<Task<anyhow::Result<BotModel>>> {
        self.failed = false;
        self.moderated = false;
        self.bot = None;
        self.account_id = SaiAccountCacheScope::active_user_id();
        self.account_epoch = SaiAccountCacheScope::active_epoch();
        cx.notify();
        if self.start_param.as_deref().is_some_and(|start_param| !BotStartPayload::is_valid(start_param)) {
            self.failed = true;
            return None;
        }
        let tag = self.tag.clone();
        Some(BotInteractionsStore::shared(cx).update(cx, |store, cx| store.resolve_start_link(&tag, cx)))
    }

    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.account_id.as_deref(), self.account_epoch.as_deref())
    }

    fn finish_resolve(&mut self, result: anyhow::Result<BotModel>, cx: &mut Context<Self>) {
        match result {
            Ok(bot) => {
                if !self.is_current_account() {
                    return;
                }
                self.bot = Some(bot);
            }
            Err(error) if matches!(error.downcast_ref::<RiseBotStartLinkError>(), Some(RiseBotStartLinkError::ModerationDisabled)) => {
                if !self.is_current_account() {
                    return;
                }
                self.moderated = true;
            }
            Err(_) => self.failed = true,
        }
        cx.notify();
    }

    fn start(&mut self, nav_store: Option<&gpui::Entity<NavigationStore>>, cx: &mut Context<Self>) {
        if self.started || !self.is_current_account() {
            return;
        }
        let Some(bot) = self.bot.clone() else {
            return;
        };
        self.started = true;
        cx.notify();
        let start_param = self.start_param.clone();
        let chat_id = BotInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.start_linked_bot(&bot, start_param.as_deref(), cx));
        let Some(chat_id) = chat_id else {
            self.started = false;
            self.bot = None;
            self.failed = true;
            return;
        };
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.replace_top(RootRoute::Chat { chat_id: Some(chat_id) }, cx);
            });
        }
    }
}

impl Render for BotStartLinkScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let tag = self.tag.clone();
        let nav_store = env::<NavigationStoreKey>(cx);

        let task_page = this.downgrade();
        use_task(page_id(&tag, "resolve"), self.attempt, window, cx, async move |cx| {
            let begin_page = task_page.clone();
            let Ok(Ok(Some(task))) = cx.update(|_, cx| begin_page.update(cx, |page, cx| page.begin_resolve(cx))) else {
                return;
            };
            let result = task.await;
            let _ = cx.update(|_, cx| task_page.update(cx, |page, cx| page.finish_resolve(result, cx)));
        });

        let state = if let Some(bot) = &self.bot {
            let start_page = this.clone();
            let start_store = nav_store.clone();
            env_scope::<IsEnabledKey>(!self.started)
                .child(BotChatStartCard {
                    id: page_id(&tag, "card"),
                    name: bot.name.clone(),
                    tag: bot.tag.clone(),
                    bot_user_id: if bot.bot_user_id.is_empty() { bot.id.clone() } else { bot.bot_user_id.clone() },
                    description: bot.description.clone(),
                    avatar_url: Some(bot.avatar_url.clone()),
                    can_manage: false,
                    on_start: Rc::new(move |_: &mut Window, cx: &mut App| {
                        start_page.update(cx, |page, cx| page.start(start_store.as_ref(), cx));
                    }),
                    on_manage: Rc::new(|_: &mut Window, _: &mut App| {}),
                })
                .into_any_element()
        } else if self.moderated {
            BotStateView::new(page_id(&tag, "moderated"), "nosign", tr("bot_start_link_moderated")).into_any_element()
        } else if self.failed {
            let retry_page = this.clone();
            BotStateView::new(page_id(&tag, "failed"), "exclamationmark.triangle", tr("error"))
                .message(tr("bot_load_error"))
                .action_title(tr("bot_retry"))
                .action(move |_: &mut Window, cx: &mut App| {
                    retry_page.update(cx, |page, cx| {
                        page.attempt += 1;
                        cx.notify();
                    });
                })
                .into_any_element()
        } else {
            progress_view().into_any_element()
        };

        let back_store = nav_store;
        SettingsWrapperUi::new(format!("@{tag}"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().flex().flex_col().items_center().justify_center().child(state))
    }
}
