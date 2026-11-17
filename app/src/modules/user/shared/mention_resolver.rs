use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWsOnError;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::stores::tag::tag_actions::{GlobalTagSearchGatewayDTO, TagActionsStore};
use crate::modules::story::stores::story::story_actions::story_failure_text::StoryFailureText;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MentionTapDestination {
    User { tag: String },
    Chat { id: String },
}

pub struct MentionTapResolver {
    generation: i64,
    loader_task: Option<Task<()>>,
    safety_task: Option<Task<()>>,
    reconnect_watch: Option<RiseTransportDropWatch>,
}

type MentionTapNavigate = Rc<dyn Fn(MentionTapDestination, &mut App)>;

enum MentionTapOutcome {
    Navigate(MentionTapDestination),
    NotFound,
}

impl MentionTapResolver {
    const LOADER_DELAY: f64 = BlockingLoaderStore::APPEARANCE_DELAY;
    const SAFETY_TIMEOUT: f64 = 15.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { generation: 0, loader_task: None, safety_task: None, reconnect_watch: None }
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.invalidate();
        Self::hide_loader(cx);
    }

    pub fn resolve(
        &mut self,
        tag: &str,
        navigate: impl Fn(MentionTapDestination, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.resolve_with(tag, Rc::new(navigate), true, false, cx);
    }

    fn resolve_with(
        &mut self,
        tag: &str,
        navigate: MentionTapNavigate,
        retries_on_connect: bool,
        page_open: bool,
        cx: &mut Context<Self>,
    ) {
        let canonical = Self::normalize(tag);
        if canonical.is_empty() {
            return;
        }

        self.invalidate();
        Self::hide_loader(cx);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;

        if retries_on_connect && RiseTransportStatus::is_waiting_for_network(cx) {
            self.open_page_until_connect(&canonical, navigate, cx);
            return;
        }

        if !page_open {
            self.loader_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs_f64(Self::LOADER_DELAY)).await;
                let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    if generation != this.generation {
                        return;
                    }
                    let weak = cx.weak_entity();
                    BlockingLoaderStore::shared(cx).update(cx, |store, cx| {
                        store.show(
                            move |cx: &mut App| {
                                let _ = weak.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                                    this.cancel(generation, cx);
                                });
                            },
                            cx,
                        );
                    });
                });
            }));
        }

        let safety_tag = canonical.clone();
        let safety_navigate = navigate.clone();
        self.safety_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::SAFETY_TIMEOUT)).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if generation != this.generation {
                    return;
                }
                if let Some(task) = this.safety_task.take() {
                    task.detach();
                }
                this.finish(generation, cx);
                if RiseTransportStatus::is_waiting_for_network(cx) {
                    if retries_on_connect {
                        this.open_page_until_connect(&safety_tag, safety_navigate.clone(), cx);
                    } else if !page_open {
                        Self::notify_connection_lost(&safety_tag, cx);
                    }
                } else if !page_open {
                    Self::notify_not_found(&safety_tag, cx);
                }
            });
        }));

        let error_navigate = navigate.clone();
        let weak_success = cx.weak_entity();
        let weak_error = cx.weak_entity();
        let success_tag = canonical.clone();
        let error_tag = canonical.clone();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let _ = weak_error.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if generation != this.generation {
                    return;
                }
                this.finish(generation, cx);
                if StoryFailureText::is_transport_failure(error, cx) {
                    if retries_on_connect {
                        this.open_page_until_connect(&error_tag, error_navigate.clone(), cx);
                    } else if !page_open {
                        Self::notify_connection_lost(&error_tag, cx);
                    }
                } else if !page_open {
                    Self::notify_not_found(&error_tag, cx);
                }
            });
        });
        let tag_actions = TagActionsStore::shared(cx);
        tag_actions.update(cx, |store, cx| {
            store.resolve_public_tag_action(
                &canonical,
                move |response: GlobalTagSearchGatewayDTO, cx: &mut App| {
                    let outcome = weak_success.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                        if generation != this.generation {
                            return None;
                        }
                        let outcome = Self::outcome(&response, &success_tag);
                        this.finish(generation, cx);
                        Some(outcome)
                    });
                    match outcome {
                        Ok(Some(MentionTapOutcome::Navigate(destination))) => navigate(destination, cx),
                        Ok(Some(MentionTapOutcome::NotFound)) if !page_open => {
                            Self::notify_not_found(&success_tag, cx)
                        }
                        Ok(Some(MentionTapOutcome::NotFound)) => {}
                        Ok(None) | Err(_) => {}
                    }
                },
                Some(on_error),
                cx,
            );
        });
    }

    /// opens the profile page for the tag at once and resolves it again on the next connect,
    /// retry only navigates if the tag turns out different
    fn open_page_until_connect(&mut self, tag: &str, navigate: MentionTapNavigate, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let opened_tag = tag.to_owned();
        navigate(MentionTapDestination::User { tag: opened_tag.clone() }, &mut **cx);
        let retry_navigate: MentionTapNavigate = Rc::new(move |destination: MentionTapDestination, cx: &mut App| {
            if let MentionTapDestination::User { tag } = &destination
                && compare_case_insensitive(&Self::normalize(tag), &opened_tag) == std::cmp::Ordering::Equal
            {
                return;
            }
            navigate(destination, cx);
        });
        self.retry_on_connect(generation, tag, retry_navigate, cx);
    }

    fn retry_on_connect(&mut self, generation: i64, tag: &str, navigate: MentionTapNavigate, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let retry_tag = tag.to_owned();
        let retry_navigate = navigate.clone();
        self.reconnect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let _ = weak.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    if generation != this.generation {
                        return;
                    }
                    this.reconnect_watch = None;
                    this.resolve_with(&retry_tag, retry_navigate, false, true, cx);
                });
            },
            cx,
        ));
        RiseTransportStatus::wake_if_offline(cx);
        if RiseTransportStatus::is_connected(cx) {
            self.reconnect_watch = None;
            self.resolve_with(tag, navigate, false, true, cx);
        }
    }

    fn outcome(response: &GlobalTagSearchGatewayDTO, canonical: &str) -> MentionTapOutcome {
        let exact_chat = response
            .results
            .iter()
            .filter_map(|item| item.chat.as_ref())
            .find(|chat| {
                chat.is_public
                    && compare_case_insensitive(
                        &Self::normalize(chat.tag.as_deref().unwrap_or_default()),
                        canonical,
                    ) == std::cmp::Ordering::Equal
            });
        let exact_user = response.results.iter().filter_map(|item| item.user.as_ref()).find(|user| {
            std::iter::once(&user.tag).chain(user.alias_tags.iter()).any(|handle| {
                compare_case_insensitive(&Self::normalize(handle), canonical) == std::cmp::Ordering::Equal
            })
        });
        if let Some(exact_chat) = exact_chat
            && !exact_chat.id.is_empty()
        {
            return MentionTapOutcome::Navigate(MentionTapDestination::Chat { id: exact_chat.id.clone() });
        }
        if let Some(exact_user) = exact_user {
            let primary = Self::normalize(&exact_user.tag);
            return MentionTapOutcome::Navigate(MentionTapDestination::User {
                tag: if primary.is_empty() { canonical.to_owned() } else { primary },
            });
        }
        MentionTapOutcome::NotFound
    }

    fn finish(&mut self, generation: i64, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.loader_task = None;
        self.safety_task = None;
        Self::hide_loader(cx);
    }

    fn cancel(&mut self, generation: i64, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.invalidate();
        Self::hide_loader(cx);
    }

    fn invalidate(&mut self) {
        self.loader_task = None;
        self.safety_task = None;
        if let Some(watch) = self.reconnect_watch.take() {
            watch.stop();
        }
    }

    fn hide_loader(cx: &mut App) {
        BlockingLoaderStore::shared(cx).update(cx, |store, cx| store.hide(cx));
    }

    fn notify_not_found(tag: &str, cx: &mut App) {
        Notifier::send(
            tr("mention_not_found_title"),
            NotifierSendOptions {
                content: Some(format!("@{tag}")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("person.fill.questionmark".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn notify_connection_lost(tag: &str, cx: &mut App) {
        Notifier::send(
            tr("error_connection_lost"),
            NotifierSendOptions {
                content: Some(format!("@{tag}")),
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("wifi.slash".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn normalize(raw: &str) -> String {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        let trimmed = trimmed.strip_prefix('@').unwrap_or(trimmed);
        trimming_whitespaces_and_newlines(trimmed).to_owned()
    }
}
