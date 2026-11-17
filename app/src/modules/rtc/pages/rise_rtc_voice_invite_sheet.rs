use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, IntoElement, RenderOnce, SharedString, WeakEntity, Window, white};
use rise_canon::rise_proto::chat::{ProtoInviteLink, ProtoInviteVoiceTarget};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_i18n::date_format::date_formatter::Style;
use rise_i18n::{AppDateFormatter, FormatArg, tr, tr_args, tr_plural};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::system_share::ShareItem;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFractionModifiers;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::{IconUi, theme};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::rtc::shared::rise_rtc_screen_safe_area::RiseRTCScreenSafeArea;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const SUBTITLE_SEPARATOR: &str = " \u{00B7} ";
const AUTHOR_RECHECK_DELAY: Duration = Duration::from_millis(1200);

fn scope_key(chat_id: &str, topic_id: Option<&str>) -> String {
    format!("rtc-voice-invite-{chat_id}-{}", topic_id.unwrap_or_default())
}

fn scoped_id(key: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{key}-{name}")))
}

/// store completions may run while the store still holds its lease on the
/// callers stack so the sheet state is written on the next effect cycle
fn deliver<S: 'static>(
    weak: WeakEntity<S>,
    cx: &mut App,
    update: impl FnOnce(&mut S, &mut Context<S>) + 'static,
) {
    cx.defer(move |cx| {
        let _ = weak.update(cx, update);
    });
}

fn copy_link(url: &str, cx: &mut App) {
    UIPasteboard::general().set_string(url, cx);
    let _ = ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
    let _ = Notifier::send(
        tr("copied"),
        NotifierSendOptions {
            r#type: NotifierType::Success,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        },
        cx,
    );
}

fn formatted_date_time(stamp_ms: i64) -> String {
    let mut formatter = AppDateFormatter::make(None, None, None);
    formatter.set_date_style(Style::Medium);
    formatter.set_time_style(Style::Short);
    formatter.string_from(&Date::from_time_interval_since_1970(stamp_ms as TimeInterval / 1000.0))
}

struct RiseRTCVoiceInviteBottomSheetState {
    fraction: f32,
}

/// invite sheet in the apps own shell like the other room sheets
/// the system one brought its glass material and nav bar into the voice chat
/// a forum text topic shares the sheet and its link opens the topic
#[derive(IntoElement)]
pub struct RiseRTCVoiceInviteBottomSheet {
    is_presented: Binding<bool>,
    chat_id: String,
    topic_id: Option<String>,
    text: bool,
}

impl RiseRTCVoiceInviteBottomSheet {
    pub fn new(is_presented: Binding<bool>, chat_id: String, topic_id: Option<String>) -> Self {
        Self { is_presented, chat_id, topic_id, text: false }
    }

    pub fn text(mut self, text: bool) -> Self {
        self.text = text;
        self
    }
}

impl RenderOnce for RiseRTCVoiceInviteBottomSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = theme(cx).bg._100;
        let key = scope_key(&self.chat_id, self.topic_id.as_deref());
        let fraction = window.use_keyed_state(scoped_id(&key, "fraction"), cx, |_, _| {
            RiseRTCVoiceInviteBottomSheetState { fraction: 0.4 }
        });
        let current_fraction = fraction.read(cx).fraction;
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let text = self.text;
        let content_key = key.clone();
        BottomSheetUi::new(
            scoped_id(&key, "bottom-sheet"),
            self.is_presented,
            move |_, _| {
                let writer = fraction.clone();
                scroll_view(scoped_id(&content_key, "scroll")).shows_indicators(false).child(
                    div()
                        .w_full()
                        .pb(px(12.0 + RiseRTCScreenSafeArea::insets().bottom))
                        .child(RiseRTCVoiceInviteSheet::new(chat_id.clone(), topic_id.clone()).text(text))
                        .measured_sheet_fraction(scoped_id(&content_key, "measure"), 0.3, move |value, _, cx| {
                            writer.update(cx, |state, cx| {
                                if state.fraction != value {
                                    state.fraction = value;
                                    cx.notify();
                                }
                            });
                        }),
                )
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![current_fraction])
        .dismisses_on_backdrop_tap(true)
        .full_page_shows_drag_indicator(true)
        .full_page_hides_footer(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(background))
        .footer_background(Some(background))
        .surface_shadow_enabled(false)
    }
}

struct RiseRTCVoiceInviteSheetState {
    chat_id: String,
    topic_id: Option<String>,
    text: bool,
    link: Option<ProtoInviteLink>,
    load_failed: bool,
    customizing: bool,
    links: Vec<ProtoInviteLink>,
    author_names: HashMap<String, String>,
    revoking: HashSet<String>,
    page: Presence<bool>,
}

impl RiseRTCVoiceInviteSheetState {
    fn new(chat_id: String, topic_id: Option<String>, text: bool) -> Self {
        Self {
            chat_id,
            topic_id,
            text,
            link: None,
            load_failed: false,
            customizing: false,
            links: Vec::new(),
            author_names: HashMap::new(),
            revoking: HashSet::new(),
            page: Presence::new(Some(false)).with_transition(Transition::opacity()),
        }
    }

    fn on_appear(&mut self, cx: &mut Context<Self>) {
        if self.link.is_none() {
            self.load(cx);
        }
        self.load_links(cx);
    }

    fn set_customizing(&mut self, customizing: bool, cx: &mut Context<Self>) {
        if self.customizing == customizing {
            return;
        }
        self.customizing = customizing;
        self.page.set_keyed_in(Some(customizing), |value| *value, ease_in_out(0.2), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn created(&mut self, created: ProtoInviteLink, cx: &mut Context<Self>) {
        self.link = Some(created.clone());
        self.links.retain(|item| item.id != created.id);
        self.links.insert(0, created);
        self.resolve_authors(cx);
        self.set_customizing(false, cx);
        cx.notify();
    }

    fn load_links(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let chat_id = self.chat_id.clone();
        let topic_id = self.topic_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_voice_invite_links(
                &chat_id,
                topic_id.as_deref(),
                Box::new(move |loaded: Option<Vec<ProtoInviteLink>>, cx: &mut App| {
                    let Some(loaded) = loaded else {
                        return;
                    };
                    deliver(weak, cx, move |this: &mut Self, cx: &mut Context<Self>| {
                        this.links = loaded;
                        this.resolve_authors(cx);
                        cx.notify();
                    });
                }),
                cx,
            );
        });
    }

    fn author_name(creator_id: &str, cx: &mut App) -> Option<String> {
        GiftPeople::person(creator_id, cx).map(|person| person.name).filter(|name| !name.is_empty())
    }

    fn resolve_authors(&mut self, cx: &mut Context<Self>) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut missing: Vec<String> = Vec::new();
        let creator_ids: Vec<String> = self
            .links
            .iter()
            .map(|item| item.creator_id.clone())
            .filter(|creator_id| seen.insert(creator_id.clone()))
            .collect();
        for creator_id in creator_ids {
            if creator_id.is_empty() || self.author_names.contains_key(&creator_id) {
                continue;
            }
            match Self::author_name(&creator_id, cx) {
                Some(name) => {
                    self.author_names.insert(creator_id, name);
                }
                None => missing.push(creator_id),
            }
        }
        if missing.is_empty() {
            return;
        }
        for creator_id in &missing {
            let creator_id = creator_id.clone();
            UserInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.refresh_foreign_profile_user_id(creator_id, cx));
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTHOR_RECHECK_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                for creator_id in missing {
                    if let Some(name) = Self::author_name(&creator_id, cx) {
                        this.author_names.insert(creator_id, name);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke(&mut self, link_id: String, cx: &mut Context<Self>) {
        if self.revoking.contains(&link_id) {
            return;
        }
        self.revoking.insert(link_id.clone());
        cx.notify();
        let weak = cx.weak_entity();
        let request_id = link_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.revoke_voice_invite_link(
                &request_id,
                Box::new(move |ok: bool, cx: &mut App| {
                    deliver(weak, cx, move |this: &mut Self, cx: &mut Context<Self>| {
                        this.finish_revoke(link_id, ok, cx);
                    });
                }),
                cx,
            );
        });
    }

    fn finish_revoke(&mut self, link_id: String, ok: bool, cx: &mut Context<Self>) {
        self.revoking.remove(&link_id);
        cx.notify();
        if !ok {
            return;
        }
        self.links.retain(|item| item.id != link_id);
        if self.link.as_ref().is_some_and(|link| link.id == link_id) {
            self.link = None;
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.load_failed = false;
        cx.notify();
        let weak = cx.weak_entity();
        let chat_id = self.chat_id.clone();
        let topic_id = self.topic_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_voice_invite_link(
                &chat_id,
                topic_id.as_deref(),
                Box::new(move |loaded: Option<ProtoInviteLink>, cx: &mut App| {
                    deliver(weak, cx, move |this: &mut Self, cx: &mut Context<Self>| {
                        match loaded {
                            Some(loaded) => this.link = Some(loaded),
                            None => this.load_failed = true,
                        }
                        cx.notify();
                    });
                }),
                cx,
            );
        });
    }

    fn row_subtitle(&self, item: &ProtoInviteLink) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(name) = self.author_names.get(&item.creator_id)
            && !name.is_empty()
        {
            parts.push(name.clone());
        }
        parts.push(if item.used_count > 0 {
            tr_plural("chat_links_joined_count", item.used_count)
        } else {
            tr("chat_links_no_joins")
        });
        if let Some(limit) = item.usage_limit
            && limit > 0
        {
            parts.push(tr_args("chat_links_remaining", &[FormatArg::Int((limit - item.used_count).max(0))]));
        }
        match item.expires_at {
            Some(expires_at) if expires_at > 0 => parts.push(formatted_date_time(expires_at)),
            _ => parts.push(tr("chat_links_expiry_none")),
        }
        parts.join(SUBTITLE_SEPARATOR)
    }
}

#[derive(IntoElement)]
pub struct RiseRTCVoiceInviteSheet {
    chat_id: String,
    topic_id: Option<String>,
    text: bool,
}

impl RiseRTCVoiceInviteSheet {
    pub fn new(chat_id: String, topic_id: Option<String>) -> Self {
        Self { chat_id, topic_id, text: false }
    }

    pub fn text(mut self, text: bool) -> Self {
        self.text = text;
        self
    }

    /// without the right to add people the link only brings members in whatever the place
    fn hint(chat_id: &str, text: bool, cx: &mut App) -> String {
        let chat = ChatServicesStore::shared(cx).read(cx).chat_info(chat_id, cx);
        let opens_for_newcomers = chat.is_some_and(|chat| ChatProfilePermissions::distributes_invites(&chat, cx));
        if !opens_for_newcomers {
            return tr("invite_members_only_hint");
        }
        tr(if text { "topic_invite_sheet_hint" } else { "voice_invite_sheet_hint" })
    }

    pub fn share_url(link: &ProtoInviteLink) -> String {
        RiseonlyPublicLink::join_url(&link.link, AppEnvironment::current())
            .map(|url| url.absolute_string().to_owned())
            .unwrap_or_else(|| link.link.clone())
    }

    fn limits_subtitle(link: &ProtoInviteLink) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(expires_at) = link.expires_at
            && expires_at > 0
        {
            parts.push(formatted_date_time(expires_at));
        }
        if let Some(limit) = link.usage_limit
            && limit > 0
        {
            parts.push(tr_plural("chat_links_usage_count", limit));
        }
        if link.used_count > 0 {
            parts.push(tr_plural("chat_links_joined_count", link.used_count));
        }
        if parts.is_empty() { None } else { Some(parts.join(SUBTITLE_SEPARATOR)) }
    }

    fn main_page(
        key: &str,
        state: &Entity<RiseRTCVoiceInviteSheetState>,
        reader: &RiseRTCVoiceInviteSheetState,
        hint: &str,
        theme: &AppTheme,
    ) -> Div {
        let status: AnyElement = if let Some(link) = reader.link.as_ref() {
            Self::link_card(key, link, theme).into_any_element()
        } else if reader.load_failed {
            let retry = state.clone();
            plain(scoped_id(key, "retry"))
                .w_full()
                .min_h(px(52.0))
                .flex()
                .items_center()
                .justify_center()
                .on_press(move |_, cx| retry.update(cx, |this, cx| this.load(cx)))
                .child(
                    MainText::new(tr("join_preview_load_failed"))
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.primary._100),
                )
                .into_any_element()
        } else {
            div()
                .w_full()
                .min_h(px(104.0))
                .flex()
                .items_center()
                .justify_center()
                .child(progress_view())
                .into_any_element()
        };

        let mut group = GroupedBtns::new(scoped_id(key, "links"), Self::invite_group_items(key, state, reader, theme))
            .horizontal_padding(0.0)
            .uses_lazy_rows(false);
        if !reader.links.is_empty() {
            group = group
                .title(tr(if reader.text { "chat_links_topic_section" } else { "chat_links_voice_section" }).to_uppercase());
        }

        vstack(18.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .child(
                div().w_full().pt(px(10.0)).flex().justify_center().child(
                    MainText::new(tr("voice_invite_tile"))
                        .px(20.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD),
                ),
            )
            .child(status)
            .child(
                div()
                    .px(px(4.0))
                    .child(MainText::new(hint.to_owned()).px(13.0).color(theme.text.secondary)),
            )
            .child(group)
    }

    fn invite_group_items(
        key: &str,
        state: &Entity<RiseRTCVoiceInviteSheetState>,
        reader: &RiseRTCVoiceInviteSheetState,
        theme: &AppTheme,
    ) -> Vec<GroupedBtn> {
        let customize = state.clone();
        let mut items = vec![
            GroupedBtn::new("voice_invite_customize", tr("voice_invite_customize"))
                .icon("slider.horizontal.3")
                .icon_background(theme.primary._100)
                .icon_foreground(white())
                .callback(move |_, cx| customize.update(cx, |this, cx| this.set_customizing(true, cx))),
        ];
        for item in &reader.links {
            let url = Self::share_url(item);
            let title = item.name.as_deref().filter(|name| !name.is_empty()).map(str::to_owned).unwrap_or_else(|| url.clone());
            items.push(
                GroupedBtn::new(item.id.clone(), title)
                    .icon(if reader.text { "number" } else { "waveform" })
                    .icon_background(ChatTileColor::blue())
                    .icon_foreground(white())
                    .subtitle(reader.row_subtitle(item))
                    .text_line_limit(1)
                    .subtitle_line_limit(2)
                    .show_chevron(false)
                    .trailing_slot(Self::revoke_button(key, item, state, reader, theme))
                    .callback(move |_, cx| copy_link(&url, cx)),
            );
        }
        items
    }

    fn revoke_button(
        key: &str,
        item: &ProtoInviteLink,
        state: &Entity<RiseRTCVoiceInviteSheetState>,
        reader: &RiseRTCVoiceInviteSheetState,
        theme: &AppTheme,
    ) -> Div {
        let is_revoking = reader.revoking.contains(&item.id);
        let link_id = item.id.clone();
        let revoke = state.clone();
        let label: AnyElement = if is_revoking {
            div()
                .w(px(70.0))
                .h(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .child(progress_view())
                .into_any_element()
        } else {
            div()
                .h(px(32.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .child(
                    MainText::new(tr("chat_links_revoke"))
                        .px(14.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.semantic.error_100)
                        .number_of_lines(1),
                )
                .into_any_element()
        };
        div().occlude().child(
            plain(scoped_id(key, &format!("revoke-{}", item.id)))
                .disabled(is_revoking)
                .on_press(move |_, cx| {
                    let link_id = link_id.clone();
                    revoke.update(cx, |this, cx| this.revoke(link_id, cx));
                })
                .child(label),
        )
    }

    fn link_card(key: &str, link: &ProtoInviteLink, theme: &AppTheme) -> Div {
        let url = Self::share_url(link);
        let copy_url = url.clone();
        let share_url = url.clone();
        vstack(12.0)
            .w_full()
            .child(
                div().w_full().py(px(13.0)).px(px(12.0)).bg(theme.bg._200).rounded_rect(14.0).child(
                    MainText::new(url)
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.primary)
                        .number_of_lines(1),
                ),
            )
            .child(
                hstack(10.0)
                    .w_full()
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(scoped_id(key, "copy"))
                                .w_full()
                                .on_press(move |_, cx| copy_link(&copy_url, cx))
                                .child(Self::button_label("doc.on.doc", tr("chat_links_copy"), theme)),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(scoped_id(key, "share"))
                                .w_full()
                                .on_press(move |_, cx| {
                                    let _ = SystemShare::present(vec![ShareItem::Link(share_url.clone())], None, cx);
                                })
                                .child(Self::button_label("square.and.arrow.up", tr("share_action"), theme)),
                        ),
                    ),
            )
            .children(Self::limits_subtitle(link).map(|subtitle| {
                MainText::new(subtitle).px(13.0).tac(TextAlignT::Center).color(theme.text.secondary)
            }))
    }

    fn button_label(icon: &'static str, title: String, theme: &AppTheme) -> Div {
        hstack(8.0)
            .w_full()
            .h(px(46.0))
            .items_center()
            .justify_center()
            .bg(theme.bg._200)
            .rounded_rect(14.0)
            .children(IconUi::sized(icon, px(16.0), theme.primary._100, false))
            .child(
                MainText::new(title)
                    .px(15.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.primary._100)
                    .number_of_lines(1),
            )
    }

    fn page(
        customizing: bool,
        key: &str,
        state: &Entity<RiseRTCVoiceInviteSheetState>,
        reader: &RiseRTCVoiceInviteSheetState,
        hint: &str,
        theme: &AppTheme,
    ) -> Div {
        if !customizing {
            return Self::main_page(key, state, reader, hint, theme);
        }
        let back = state.clone();
        let weak = state.downgrade();
        div().w_full().child(RiseRTCVoiceInviteCustomizeForm {
            key: format!("{key}-customize"),
            chat_id: reader.chat_id.clone(),
            topic_id: reader.topic_id.clone(),
            text: reader.text,
            on_back: Rc::new(move |_: &mut Window, cx: &mut App| {
                back.update(cx, |this, cx| this.set_customizing(false, cx));
            }),
            on_created: Rc::new(move |created: ProtoInviteLink, cx: &mut App| {
                deliver(weak.clone(), cx, move |this: &mut RiseRTCVoiceInviteSheetState, cx| {
                    this.created(created, cx);
                });
            }),
        })
    }
}

impl RenderOnce for RiseRTCVoiceInviteSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = scope_key(&self.chat_id, self.topic_id.as_deref());
        let text = self.text;
        let hint = Self::hint(&self.chat_id, text, cx);
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let state = window.use_keyed_state(scoped_id(&key, "state"), cx, move |_, _| {
            RiseRTCVoiceInviteSheetState::new(chat_id, topic_id, text)
        });
        let theme = theme(cx).clone();
        let now = RiseClock::monotonic();
        let reader = state.read(cx);
        let departing = reader.page.render_departing_transitioned(now, window, |customizing| {
            Self::page(*customizing, &key, &state, reader, &hint, &theme)
        });
        let current = reader
            .page
            .render_transitioned(now, window, |customizing| Self::page(*customizing, &key, &state, reader, &hint, &theme));
        let appear = state.clone();
        zstack(Alignment::Top)
            .w_full()
            .children(departing)
            .children(current)
            .on_appear(scoped_id(&key, "appear"), move |_, cx| {
                appear.update(cx, |this, cx| this.on_appear(cx));
            })
    }
}

#[derive(IntoElement)]
struct RiseRTCVoiceInviteCustomizeForm {
    key: String,
    chat_id: String,
    topic_id: Option<String>,
    text: bool,
    on_back: Rc<dyn Fn(&mut Window, &mut App)>,
    on_created: Rc<dyn Fn(ProtoInviteLink, &mut App)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpiryPreset {
    HalfHour,
    Hour,
    SixHours,
    TwelveHours,
    Day,
    Week,
    None,
}

impl ExpiryPreset {
    const ALL_CASES: [Self; 7] =
        [Self::HalfHour, Self::Hour, Self::SixHours, Self::TwelveHours, Self::Day, Self::Week, Self::None];

    fn title_key(self) -> &'static str {
        match self {
            Self::HalfHour => "chat_links_expiry_30m",
            Self::Hour => "chat_links_expiry_hour",
            Self::SixHours => "chat_links_expiry_6h",
            Self::TwelveHours => "chat_links_expiry_12h",
            Self::Day => "chat_links_expiry_day",
            Self::Week => "chat_links_expiry_week",
            Self::None => "chat_links_expiry_none",
        }
    }

    fn seconds(self) -> Option<TimeInterval> {
        match self {
            Self::HalfHour => Some(1_800.0),
            Self::Hour => Some(3_600.0),
            Self::SixHours => Some(21_600.0),
            Self::TwelveHours => Some(43_200.0),
            Self::Day => Some(86_400.0),
            Self::Week => Some(604_800.0),
            Self::None => None,
        }
    }
}

struct RiseRTCVoiceInviteCustomizeFormState {
    expiry: ExpiryPreset,
    usage: i64,
    submitting: bool,
}

impl RiseRTCVoiceInviteCustomizeFormState {
    fn create(
        &mut self,
        chat_id: &str,
        topic_id: Option<String>,
        on_created: Rc<dyn Fn(ProtoInviteLink, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if self.submitting {
            return;
        }
        self.submitting = true;
        cx.notify();
        let expires_at_ms: Option<i64> = self
            .expiry
            .seconds()
            .map(|seconds| ((RiseClock::now().time_interval_since_1970() + seconds) * 1000.0) as i64);
        let usage_limit = (self.usage > 0).then_some(self.usage);
        let weak = cx.weak_entity();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_invite_link(
                chat_id,
                None,
                expires_at_ms,
                usage_limit,
                false,
                Some(ProtoInviteVoiceTarget { topic_id, text: false }),
                None,
                Some(on_created),
                Box::new(move |_: bool, cx: &mut App| {
                    deliver(weak, cx, |this: &mut Self, cx: &mut Context<Self>| {
                        this.submitting = false;
                        cx.notify();
                    });
                }),
                cx,
            );
        });
    }
}

impl RiseRTCVoiceInviteCustomizeForm {
    const USAGE_PRESETS: [i64; 7] = [0, 1, 5, 10, 25, 50, 100];

    fn usage_title(count: i64) -> String {
        if count == 0 { tr("chat_links_usage_unlimited") } else { tr_plural("chat_links_usage_count", count) }
    }

    fn header(&self, theme: &AppTheme) -> Div {
        let on_back = self.on_back.clone();
        hstack(8.0)
            .px(px(10.0))
            .pt(px(6.0))
            .child(
                div().id(scoped_id(&self.key, "back-label")).tooltip_label(tr("common_back")).flex_none().child(
                    plain(scoped_id(&self.key, "back"))
                        .size(px(36.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_press(move |window, cx| on_back(window, cx))
                        .children(IconUi::sized("chevron.left", px(18.0), theme.primary._100, false)),
                ),
            )
            .child(
                MainText::new(tr(if self.text { "topic_invite_customize_title" } else { "voice_invite_customize_title" }))
                    .px(20.0)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1),
            )
    }

    fn create_button(&self, state: &Entity<RiseRTCVoiceInviteCustomizeFormState>, submitting: bool, theme: &AppTheme) -> Div {
        let create = state.clone();
        let chat_id = self.chat_id.clone();
        let topic_id = self.topic_id.clone();
        let on_created = self.on_created.clone();
        let label: AnyElement = if submitting {
            progress_view().tint(white()).into_any_element()
        } else {
            MainText::new(tr("chat_links_create"))
                .px(16.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::BOLD)
                .color(white())
                .into_any_element()
        };
        div().w_full().px(px(16.0)).child(
            plain(scoped_id(&self.key, "create"))
                .w_full()
                .disabled(submitting)
                .on_press(move |_, cx| {
                    let topic_id = topic_id.clone();
                    let on_created = on_created.clone();
                    create.update(cx, |this, cx| this.create(&chat_id, topic_id, on_created, cx));
                })
                .child(
                    div()
                        .w_full()
                        .h(px(50.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.primary._100)
                        .capsule()
                        .child(label),
                ),
        )
    }

    fn choice_row(
        id: String,
        title: String,
        selected: bool,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> GroupedBtn {
        let row = GroupedBtn::new(id, title)
            .text_line_limit(1)
            .accessibility_selected(selected)
            .show_chevron(false)
            .callback(action);
        if selected {
            row.trailing_slot(div().children(IconUi::sized("checkmark", px(14.0), theme.primary._100, false)))
        } else {
            row
        }
    }
}

impl RenderOnce for RiseRTCVoiceInviteCustomizeForm {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx).clone();
        let state = window.use_keyed_state(scoped_id(&self.key, "state"), cx, |_, _| RiseRTCVoiceInviteCustomizeFormState {
            expiry: ExpiryPreset::None,
            usage: 0,
            submitting: false,
        });
        let (expiry, usage, submitting) = {
            let reader = state.read(cx);
            (reader.expiry, reader.usage, reader.submitting)
        };

        let expiry_rows: Vec<GroupedBtn> = ExpiryPreset::ALL_CASES
            .into_iter()
            .map(|preset| {
                let select = state.clone();
                Self::choice_row(
                    format!("voice-invite-expiry-{}", preset.title_key()),
                    tr(preset.title_key()),
                    expiry == preset,
                    &theme,
                    move |_, cx| {
                        select.update(cx, |this, cx| {
                            this.expiry = preset;
                            cx.notify();
                        });
                    },
                )
            })
            .collect();
        let usage_rows: Vec<GroupedBtn> = Self::USAGE_PRESETS
            .into_iter()
            .map(|count| {
                let select = state.clone();
                Self::choice_row(
                    format!("voice-invite-usage-{count}"),
                    Self::usage_title(count),
                    usage == count,
                    &theme,
                    move |_, cx| {
                        select.update(cx, |this, cx| {
                            this.usage = count;
                            cx.notify();
                        });
                    },
                )
            })
            .collect();

        vstack(16.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(self.header(&theme))
            .child(GroupedBtns::new(scoped_id(&self.key, "expiry"), expiry_rows).title(tr("chat_links_expiry_title")))
            .child(GroupedBtns::new(scoped_id(&self.key, "usage"), usage_rows).title(tr("chat_links_usage_title")))
            .child(self.create_button(&state, submitting, &theme))
    }
}
