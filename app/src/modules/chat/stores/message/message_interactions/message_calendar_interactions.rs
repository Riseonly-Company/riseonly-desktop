use std::rc::Rc;

use gpui::{App, Context};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::characters;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::{Locale, tr};

use super::message_interactions::MessageInteractionsStore;
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::engine::messages::rise_chat_calendar::{
    RiseChatCalendarDay, RiseChatCalendarMonth, RiseChatCalendarPage, RiseChatCalendarWindow,
};
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn uppercased_first_character(raw: &str) -> String {
    match characters(raw).next() {
        Some(first) => format!("{}{}", first.to_uppercase(), &raw[first.len()..]),
        None => String::new(),
    }
}

impl MessageInteractionsStore {
    #[allow(clippy::too_many_arguments)]
    pub fn open_chat_calendar(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        calendar: &Calendar,
        locale: &Locale,
        now: &Date,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let weekday_symbols = RiseChatCalendarMonth::weekday_symbols(calendar, locale.identifier());
        MessageServicesStore::shared(cx).update(cx, |services, cx| {
            services.open_chat_calendar(chat_id, topic_id, thread_root_id, weekday_symbols, cx);
        });
        self.load_chat_calendar_month(now, calendar, locale, false, cx);
    }

    pub fn load_older_chat_calendar_month(&self, calendar: &Calendar, locale: &Locale, cx: &mut Context<Self>) {
        let services = MessageServicesStore::shared(cx);
        let Some(oldest) = services.read(cx).chat_calendar_oldest_loaded_month_start() else {
            return;
        };
        let Some(previous) =
            calendar.date_by_adding(CalendarComponent::Month, -1, &RiseChatCalendarWindow::date(oldest))
        else {
            return;
        };
        self.load_chat_calendar_month(&previous, calendar, locale, true, cx);
    }

    pub fn select_chat_calendar_day(
        &self,
        day: &RiseChatCalendarDay,
        completion: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let services = MessageServicesStore::shared(cx);
        let presentation = &services.read(cx).chat_calendar;
        let chat_id = presentation.chat_id.clone();
        let topic_id = presentation.topic_id.clone();
        let thread_root_id = presentation.thread_root_id.clone();
        if chat_id.is_empty() {
            if let Some(completion) = completion {
                cx.defer(move |cx| completion(false, cx));
            }
            return;
        }
        let day_start_ms = day.day_start_ms;
        MessageActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.jump_to_date_action(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                day_start_ms,
                Self::jump_completion(completion),
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn jump_to_chat_calendar_day(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        day_start_ms: i64,
        is_loaded_locally: bool,
        completion: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || is_loaded_locally {
            if let Some(completion) = completion {
                let installed = !chat_id.is_empty();
                cx.defer(move |cx| completion(installed, cx));
            }
            return;
        }
        MessageActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.jump_to_date_action(
                chat_id,
                topic_id,
                thread_root_id,
                day_start_ms,
                Self::jump_completion(completion),
                cx,
            );
        });
    }

    fn jump_completion(completion: Option<Rc<dyn Fn(bool, &mut App)>>) -> Rc<dyn Fn(bool, &mut App)> {
        Rc::new(move |installed: bool, cx: &mut App| {
            if let Some(completion) = &completion {
                completion(installed, cx);
            }
        })
    }

    fn load_chat_calendar_month(
        &self,
        date: &Date,
        calendar: &Calendar,
        locale: &Locale,
        is_older: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(window) = RiseChatCalendarWindow::month(date, calendar) else {
            return;
        };
        let services = MessageServicesStore::shared(cx);
        let presentation = &services.read(cx).chat_calendar;
        let scope_key = presentation.scope_key.clone();
        let chat_id = presentation.chat_id.clone();
        let topic_id = presentation.topic_id.clone();
        let thread_root_id = presentation.thread_root_id.clone();
        if !services.update(cx, |services, cx| services.begin_chat_calendar_load(&scope_key, is_older, cx)) {
            return;
        }

        let raw_title =
            RiseFormat::date_formatter(&tr("date_month_year_format"), locale.identifier(), calendar, None).string_from(date);
        let title = uppercased_first_character(&raw_title);

        let revalidated: Option<Rc<dyn Fn(RiseChatCalendarPage, &mut App)>> = if is_older {
            None
        } else {
            let window = window.clone();
            let calendar = calendar.clone();
            let title = title.clone();
            let scope_key = scope_key.clone();
            Some(Rc::new(move |page: RiseChatCalendarPage, cx: &mut App| {
                Self::publish_chat_calendar_month(
                    page,
                    window.clone(),
                    calendar.clone(),
                    title.clone(),
                    scope_key.clone(),
                    false,
                    cx,
                );
            }))
        };

        let request_window = window.clone();
        let calendar = calendar.clone();
        let completion: Rc<dyn Fn(Option<RiseChatCalendarPage>, &mut App)> =
            Rc::new(move |page: Option<RiseChatCalendarPage>, cx: &mut App| {
                let Some(page) = page else {
                    MessageServicesStore::shared(cx).update(cx, |services, cx| {
                        services.fail_chat_calendar_load(&scope_key, is_older, cx);
                    });
                    return;
                };
                Self::publish_chat_calendar_month(
                    page,
                    window.clone(),
                    calendar.clone(),
                    title.clone(),
                    scope_key.clone(),
                    is_older,
                    cx,
                );
            });

        MessageActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.load_chat_calendar_month_action(
                &chat_id,
                topic_id.as_deref(),
                thread_root_id.as_deref(),
                request_window,
                true,
                false,
                revalidated,
                completion,
                cx,
            );
        });
    }

    fn publish_chat_calendar_month(
        page: RiseChatCalendarPage,
        window: RiseChatCalendarWindow,
        calendar: Calendar,
        title: String,
        scope_key: String,
        is_older: bool,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| {
            let has_older = page.has_older;
            let building = RiseAccountRuntime::current().spawn(async move {
                RiseChatCalendarMonth::make(&window, &page, &calendar, title, &Date::now())
            });
            let Ok(month) = building.await else {
                return;
            };
            cx.update(|cx| {
                MessageServicesStore::shared(cx).update(cx, |services, cx| {
                    services.apply_chat_calendar_month(month, has_older, &scope_key, is_older, cx);
                });
            });
        })
        .detach();
    }
}
