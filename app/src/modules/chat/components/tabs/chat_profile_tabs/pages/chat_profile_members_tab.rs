use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Entity, FontWeight, Pixels, Point, SharedString, Task, Window, canvas};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileTabBottomSpacer, ChatProfileTabErrorState, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::{
    ChatProfileTabHost, ChatProfileWindowedRow, ChatProfileWindowedRows,
};
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen_support::ChatHeaderRefreshSchedule;
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::{
    ForumMemberGrouping, ForumMemberGroupingViewerRights, MemberRoleGroup,
};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_members::member_rights_sheet::{
    MemberRightsSheetPresentation, MemberRightsTarget,
};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::MemberRolePill;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMember;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::presence::stores::presence::presence_interactions::presence_interactions::PresenceInteractionsStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const PREFETCH_THRESHOLD: usize = 8;
const LOGO_SIZE: f32 = 40.0;
const ONLINE_DOT_SIZE: f32 = 10.0;
const ONLINE_DOT_STROKE: f32 = 1.5;
const ONLINE_DOT_OFFSET: f32 = 14.0;

const END_PROBE_REACH: f32 = 480.0;

/// asks for the next members page while the list end is on screen
/// row prefetch claims a cursor once, a skipped page leaves the roster short
/// shared by profile tab and members page
pub(crate) fn members_end_probe(chat_id: &str, topic_id: Option<&str>, member_scope: &str) -> AnyElement {
    let chat_id = chat_id.to_string();
    let topic_id = topic_id.map(str::to_string);
    let member_scope = member_scope.to_string();
    div()
        .w_full()
        .h(px(1.0))
        .child(
            canvas(
                move |bounds, window, cx| {
                    if bounds.top() > window.viewport_size().height + px(END_PROBE_REACH) {
                        return;
                    }
                    let services = ChatServicesStore::shared(cx);
                    if !services.update(cx, |services, cx| services.members_has_more(&member_scope, cx)) {
                        return;
                    }
                    let cache_key = ChatServicesStore::members_cache_key(&member_scope);
                    if !SaiWs::pagination_state(&cache_key, cx)
                        .read(cx)
                        .can_request_automatically(SaiWsPaginationEdge::Trailing)
                    {
                        return;
                    }
                    let chat_id = chat_id.clone();
                    let topic_id = topic_id.clone();
                    cx.defer(move |cx| {
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.load_more_members(&chat_id, topic_id.as_deref(), cx);
                        });
                    });
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
        .into_any_element()
}

#[derive(Clone)]
struct ChatProfileMembersTabParts {
    chat_id: String,
    topic_id: Option<String>,
    member_scope: String,
    nav_store: Option<Entity<NavigationStore>>,
}

/// presence the shown rows read, online dot and status line
#[derive(Default)]
struct MembersPresenceShown {
    members: Vec<(String, bool, Option<i64>)>,
    presence: Vec<(bool, Option<i64>)>,
}

impl MembersPresenceShown {
    fn read(members: &[(String, bool, Option<i64>)], presence: &PresenceServicesStore) -> Vec<(bool, Option<i64>)> {
        members
            .iter()
            .map(|(user_id, is_online, last_seen)| {
                (presence.is_online(user_id, *is_online), presence.last_seen(user_id, *last_seen))
            })
            .collect()
    }
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileMembersTab {
    chat_id: String,
    topic_id: Option<String>,
    group_by_role: bool,
}

impl ChatProfileMembersTab {
    const ESTIMATED_ROW_HEIGHT: f32 = 58.5;

    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), topic_id: None, group_by_role: false }
    }

    pub(crate) fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    pub(crate) fn group_by_role(mut self, group_by_role: bool) -> Self {
        self.group_by_role = group_by_role;
        self
    }

    fn empty_text(is_preview: bool, member_scope: &str, cx: &mut App) -> String {
        if is_preview {
            return tr("chat_members_after_join");
        }
        if ChatServicesStore::shared(cx).read(cx).is_member_list_refused(member_scope)
            && let Some(text) = CanonErrors::text(Some(ErrorCode::ChatPermissionDenied.wire()), &HashMap::new())
        {
            return text;
        }
        tr("chat_members_empty")
    }

    fn load_members(chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_members(chat_id, topic_id, None, true, cx));
    }

    fn body(
        chat_id: &str,
        topic_id: Option<&str>,
        group_by_role: bool,
        presence_shown: &RefCell<MembersPresenceShown>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let member_scope = ChatServicesStore::member_scope_key(chat_id, topic_id);
        let id_base = format!("chat-profile-members-{member_scope}");
        let services = ChatServicesStore::shared(cx);
        let chat_info = services.update(cx, |services, cx| services.chat_info(chat_id, cx));
        let is_preview = chat_info.as_ref().is_some_and(|chat| chat.is_community_preview());
        let members = if is_preview {
            Vec::new()
        } else {
            let members = services.update(cx, |services, cx| services.members(&member_scope, cx));
            ForumMemberGrouping::viewer_visible_members(members, chat_id, cx)
        };
        let rights = ForumMemberGrouping::viewer_rights(chat_id, cx);
        let parts = ChatProfileMembersTabParts {
            chat_id: chat_id.to_owned(),
            topic_id: topic_id.map(str::to_owned),
            member_scope: member_scope.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        let shown_members: Vec<(String, bool, Option<i64>)> =
            members.iter().map(|member| (member.user_id.clone(), member.is_online, member.last_seen)).collect();
        let presence = MembersPresenceShown::read(&shown_members, PresenceServicesStore::shared(cx).read(cx));
        *presence_shown.borrow_mut() = MembersPresenceShown { members: shown_members, presence };
        Self::wire_presence(&parts, &members, is_preview, &id_base, window, cx);
        Self::wait_for_presence_refresh(&members, &id_base, window, cx);

        let has_members = !members.is_empty();
        let status = services.read(cx).member_list_render_status(&member_scope, has_members);
        let empty_text = Self::empty_text(is_preview, &member_scope, cx);
        let unloaded_total = (topic_id.is_none()
            && services.update(cx, |services, cx| services.members_has_more(&member_scope, cx)))
        .then(|| chat_info.as_ref().map(|chat| chat.member_count))
        .flatten();
        let rows = Self::rows(&parts, &members, rights, group_by_role, unloaded_total, &id_base, cx);

        let refresh_parts = parts.clone();
        let error_parts = parts.clone();
        let end_parts = parts.clone();
        let stop_owner = Self::presence_owner(&member_scope);
        let scroll_id = SharedString::from(id_base.clone());
        let rows_id = SharedString::from(format!("{id_base}-rows"));
        let loading_id = SharedString::from(format!("{id_base}-loading"));

        div()
            .w_full()
            .child(
                SaiDataRender::new(status, move |_, _| {
                    ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                        div()
                            .w_full()
                            .child(ChatProfileWindowedRows::new(rows_id, Self::ESTIMATED_ROW_HEIGHT, rows))
                            .child(members_end_probe(
                                &end_parts.chat_id,
                                end_parts.topic_id.as_deref(),
                                &end_parts.member_scope,
                            ))
                            .child(ChatProfileTabBottomSpacer)
                    })
                    .on_refresh(move |_, cx| {
                        let parts = refresh_parts.clone();
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.refresh_members(&parts.chat_id, parts.topic_id.as_deref(), cx);
                        });
                        Task::ready(())
                    })
                })
                .id(SharedString::from(format!("{id_base}-render")))
                .is_empty(!has_members)
                .no_data_text(empty_text)
                .need_pending(!has_members && !is_preview)
                .sai_data_render_loading(move |_, _| {
                    ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| UserListSkeleton::new())
                        .scroll_enabled(false)
                        .shows_indicators(false)
                })
                .sai_data_render_error(move |_, _| {
                    let parts = error_parts.clone();
                    ChatProfileTabErrorState::new(tr("error_fetching_data"), move |_, cx| {
                        Self::load_members(&parts.chat_id, parts.topic_id.as_deref(), cx);
                    })
                }),
            )
            .on_disappear(SharedString::from(format!("{id_base}-disappear")), move |_, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.stop_observing(&stop_owner, cx));
            })
            .into_any_element()
    }

    fn presence_owner(member_scope: &str) -> String {
        format!("members-{member_scope}")
    }

    fn wire_presence(
        parts: &ChatProfileMembersTabParts,
        members: &[ChatMember],
        is_preview: bool,
        id_base: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let member_ids: Vec<String> = members.iter().map(|member| member.user_id.clone()).collect();
        let owner = Self::presence_owner(&parts.member_scope);
        on_change(
            SharedString::from(format!("{id_base}-member-ids")),
            member_ids,
            true,
            window,
            cx,
            move |_, ids, _, cx| {
                PresenceInteractionsStore::shared(cx).update(cx, |store, cx| store.observe(ids, &owner, cx));
            },
        );
        let chat_id = parts.chat_id.clone();
        let topic_id = parts.topic_id.clone();
        on_change(
            SharedString::from(format!("{id_base}-preview")),
            is_preview,
            false,
            window,
            cx,
            move |was_preview, now_preview, _, cx| {
                if *was_preview && !*now_preview {
                    Self::load_members(&chat_id, topic_id.as_deref(), cx);
                }
            },
        );
    }

    fn next_presence_refresh_timestamp(members: &[ChatMember], cx: &mut App) -> Option<TimeInterval> {
        let now = RiseClock::now().time_interval_since_1970();
        let gate = MaintenanceGate::shared(cx);
        let presence_now = gate.read(cx).server_now().time_interval_since_1970();
        let presence = PresenceServicesStore::shared(cx);
        let presence = presence.read(cx);
        let mut earliest: Option<TimeInterval> = None;
        for member in members {
            let next = ChatHeaderRefreshSchedule::next_refresh_timestamp(
                &[],
                0.0,
                presence.is_online(&member.user_id, member.is_online),
                presence.last_seen(&member.user_id, member.last_seen),
                now,
                Some(presence_now),
            );
            if let Some(next) = next
                && next < earliest.unwrap_or(TimeInterval::INFINITY)
            {
                earliest = Some(next);
            }
        }
        earliest
    }

    fn wait_for_presence_refresh(members: &[ChatMember], id_base: &str, window: &mut Window, cx: &mut App) {
        let revision = window.use_keyed_state(SharedString::from(format!("{id_base}-presence-revision")), cx, |_, _| 0u64);
        let timestamp = Self::next_presence_refresh_timestamp(members, cx);
        use_task(
            SharedString::from(format!("{id_base}-presence-refresh")),
            timestamp,
            window,
            cx,
            async move |cx| {
                let Some(timestamp) = timestamp else { return };
                let delay = (timestamp - RiseClock::now().time_interval_since_1970()).max(0.0);
                if delay > 0.0 {
                    cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
                }
                let _ = cx.update(|_, cx| {
                    revision.update(cx, |revision, cx| {
                        *revision = revision.wrapping_add(1);
                        cx.notify();
                    });
                });
            },
        );
    }

    fn rows(
        parts: &ChatProfileMembersTabParts,
        members: &[ChatMember],
        rights: ForumMemberGroupingViewerRights,
        group_by_role: bool,
        unloaded_total: Option<i64>,
        id_base: &str,
        cx: &mut App,
    ) -> Vec<ChatProfileWindowedRow> {
        let mut seen: HashSet<&str> = HashSet::with_capacity(members.len());
        let members: Vec<ChatMember> =
            members.iter().filter(|member| seen.insert(member.user_id.as_str())).cloned().collect();
        let members = members.as_slice();
        let item_count = members.len();
        let mut display_index = 0usize;
        let mut rows = Vec::with_capacity(item_count);
        if group_by_role {
            for group in ForumMemberGrouping::groups(members, &parts.chat_id, unloaded_total, cx) {
                let header = group.clone();
                rows.push(ChatProfileWindowedRow::new(
                    format!("{id_base}-group-{}", group.id),
                    move |_, cx| Self::group_header(&header, cx),
                ));
                for member in group.members {
                    rows.push(Self::windowed_member_row(parts, member, rights, display_index, item_count, id_base));
                    display_index += 1;
                }
            }
        } else {
            let ordered = ForumMemberGrouping::presence_ordered(members, cx);
            for (index, member) in ordered.into_iter().enumerate() {
                rows.push(Self::windowed_member_row(parts, member, rights, index, item_count, id_base));
            }
        }
        rows
    }

    fn windowed_member_row(
        parts: &ChatProfileMembersTabParts,
        member: ChatMember,
        rights: ForumMemberGroupingViewerRights,
        index: usize,
        item_count: usize,
        id_base: &str,
    ) -> ChatProfileWindowedRow {
        let parts = parts.clone();
        let member = Rc::new(member);
        ChatProfileWindowedRow::new(format!("{id_base}-member-{}", member.user_id), move |_, cx| {
            Self::member_row(&parts, member.clone(), rights, index, item_count, cx)
        })
    }

    fn group_header(group: &MemberRoleGroup, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        hstack(6.0)
            .w_full()
            .px(px(16.0))
            .pt(px(14.0))
            .pb(px(5.0))
            .child(
                MainText::new(group.title.to_uppercase())
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(group.color.map(|color| theme.legible(color)).unwrap_or(theme.text.secondary))
                    .line_limit(1),
            )
            .child(div().flex_none().child(MainText::new(group.count().to_string()).px(12.0).color(theme.text.secondary)))
            .child(spacer())
            .into_any_element()
    }

    fn member_row(
        parts: &ChatProfileMembersTabParts,
        member: Rc<ChatMember>,
        rights: ForumMemberGroupingViewerRights,
        index: usize,
        item_count: usize,
        cx: &mut App,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let actions = ForumMemberGrouping::member_actions(&member, &parts.chat_id, rights, cx);
        let services = ChatServicesStore::shared(cx);
        let cache_key = ChatServicesStore::members_cache_key(&parts.member_scope);
        let relative_id = services.update(cx, |services, cx| services.members_last_id(&parts.member_scope, cx));
        let has_more = services.update(cx, |services, cx| services.members_has_more(&parts.member_scope, cx));

        let content_member = member.clone();
        let content_parts = parts.clone();
        let action_member = member.clone();
        let action_parts = parts.clone();
        let tap_member = member.clone();
        let tap_parts = parts.clone();
        let load_parts = parts.clone();
        vstack(1.0)
            .w_full()
            .pb(px(1.0))
            .child(
                div().w_full().child(
                    HoldContextMenuContainer::new(
                        SharedString::from(format!("chat-profile-member-hold-{}", member.user_id)),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(move |_: &App| actions.clone()),
                        Rc::new(move |_: &mut Window, _: &mut App| {
                            let parts = content_parts.clone();
                            let profile_member = content_member.clone();
                            ProfileMemberRow {
                                author: content_member.to_author_info(),
                                member: content_member.clone(),
                                on_open_avatar_profile: Rc::new(move |_: &mut Window, cx: &mut App| {
                                    Self::open_chat_profile(&parts, &profile_member, cx);
                                }),
                            }
                            .into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                        Self::member_hold_action(&action_parts, &action_member, &id, window, cx);
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                        Self::open_chat_profile(&tap_parts, &tap_member, cx);
                    }))
                    .sai_pagination_prefetch(
                        SharedString::from(format!("chat-profile-member-prefetch-{}", member.user_id)),
                        SaiPaginationPrefetchOptions::new(index, item_count, cache_key, relative_id, has_more)
                            .threshold(PREFETCH_THRESHOLD),
                        move |_, cx| {
                            let parts = load_parts.clone();
                            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.load_more_members(&parts.chat_id, parts.topic_id.as_deref(), cx);
                            });
                        },
                    ),
                ),
            )
            .child(div().w_full().h(px(0.5)).bg(theme.border._100))
            .into_any_element()
    }

    fn present_rights(
        parts: &ChatProfileMembersTabParts,
        member: &ChatMember,
        restrictions_only: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let target = MemberRightsTarget::new(member.clone(), restrictions_only);
        let chat_id = parts.chat_id.clone();
        MemberRightsSheetPresentation::shared(cx)
            .update(cx, |sheet, cx| sheet.present(chat_id, target, |_: &mut App| {}, window, cx));
    }

    fn member_hold_action(
        parts: &ChatProfileMembersTabParts,
        member: &ChatMember,
        id: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        match id {
            "manage" => Self::present_rights(parts, member, false, window, cx),
            "copy" => {
                let value = if member.tag.is_empty() { member.name.clone() } else { format!("@{}", member.tag) };
                UIPasteboard::general().set_string(&value, cx);
            }
            "mention" => {
                if !member.tag.is_empty() {
                    UIPasteboard::general().set_string(&format!("@{}", member.tag), cx);
                }
            }
            "restrict" => Self::present_rights(parts, member, true, window, cx),
            "remove" => {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.confirm_remove_member(&parts.chat_id, member, cx));
            }
            "report" => {
                let me = AuthServicesStore::shared(cx)
                    .read(cx)
                    .profile()
                    .map(|profile| profile.id.clone())
                    .unwrap_or_default();
                if !member.user_id.is_empty() && member.user_id != me {
                    let target = ReportTarget::User(member.user_id.clone());
                    let subject = ReportSubject {
                        name: member.name.clone(),
                        tag: member.tag.clone(),
                        avatar_url: member.avatar_url.clone(),
                        user_id: Some(member.user_id.clone()),
                    };
                    ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, Some(subject), None, cx));
                }
            }
            _ => {}
        }
    }

    fn open_chat_profile(parts: &ChatProfileMembersTabParts, member: &ChatMember, cx: &mut App) {
        let user = member.to_user();
        let chat_id =
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.direct_conversation_id(&user, cx));
        let route = if chat_id.is_empty() {
            RootRoute::UserPage { user_id: Some(member.user_id.clone()), tag: Some(member.tag.clone()) }
        } else {
            RootRoute::ChatProfile { chat_id: Some(chat_id) }
        };
        if let Some(nav_store) = parts.nav_store.as_ref() {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

impl RenderOnce for ChatProfileMembersTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let group_by_role = self.group_by_role;
        let presence_shown = Rc::new(RefCell::new(MembersPresenceShown::default()));
        let body_presence_shown = presence_shown.clone();
        ChatProfileTabHost::hosted(
            SharedString::from(format!(
                "chat-profile-members-host-{chat_id}-{}-{group_by_role}",
                topic_id.as_deref().unwrap_or_default()
            )),
            window,
            cx,
            {
                let members_key =
                    ChatServicesStore::members_cache_key(&ChatServicesStore::member_scope_key(&chat_id, topic_id.as_deref()));
                move |cx| {
                    let presence = PresenceServicesStore::shared(cx);
                    let revisions = SaiCacheRevisionTracker::shared(cx);
                    let seen = Cell::new(revisions.read(cx).revision(&members_key));
                    vec![
                        cx.observe(&presence, move |_, presence, cx| {
                            let mut shown = presence_shown.borrow_mut();
                            let next = MembersPresenceShown::read(&shown.members, presence.read(cx));
                            if next != shown.presence {
                                shown.presence = next;
                                drop(shown);
                                cx.notify();
                            }
                        }),
                        cx.observe(&revisions, move |_, revisions, cx| {
                            let revision = revisions.read(cx).revision(&members_key);
                            if revision != seen.get() {
                                seen.set(revision);
                                cx.notify();
                            }
                        }),
                    ]
                }
            },
            move |window, cx| {
                Self::body(&chat_id, topic_id.as_deref(), group_by_role, &body_presence_shown, window, cx)
            },
        )
    }
}

#[derive(IntoElement)]
struct ProfileMemberRow {
    author: AuthorInfo,
    member: Rc<ChatMember>,
    on_open_avatar_profile: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ProfileMemberRow {
    fn rank_chip(member: &ChatMember) -> Option<MemberRolePill> {
        if !member.custom_title.is_empty() {
            Some(MemberRolePill::new(member.custom_title.clone()).color_hex(member.custom_color.clone()))
        } else if member.is_owner() {
            Some(MemberRolePill::new(tr("chat_member_role_owner")))
        } else if member.is_admin() {
            Some(MemberRolePill::new(tr("chat_member_role_admin")))
        } else {
            None
        }
    }
}

impl RenderOnce for ProfileMemberRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let member = self.member;
        let author = self.author;
        let presence = PresenceServicesStore::shared(cx);
        let is_online = presence.read(cx).is_online(&member.user_id, member.is_online);
        let status_text = presence
            .update(cx, |presence, cx| presence.status_text(&member.user_id, member.is_online, member.last_seen, cx));
        let name_color = member
            .name_color()
            .map(|color| theme.legible(color))
            .or_else(|| PremiumNameColor::color_for_user(author.id.as_deref(), Some(&author.more), cx));
        let on_press = self.on_open_avatar_profile;
        let user_id = member.user_id.clone();
        let dot_outer = ONLINE_DOT_SIZE + ONLINE_DOT_STROKE;
        let dot_origin = (LOGO_SIZE - dot_outer) / 2.0 + ONLINE_DOT_OFFSET;

        div().w_full().bg(theme.bg._100).hover(|style| style.bg(theme.bg._200)).child(
            hstack(10.0)
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .child(
                    div()
                        .id(SharedString::from(format!("chat-profile-member-logo-frame-{user_id}")))
                        .occlude()
                        .relative()
                        .size(px(LOGO_SIZE))
                        .flex_none()
                        .child(
                            UserLogoUi::new(SharedString::from(format!("chat-profile-member-logo-{user_id}")))
                                .source(SharedString::from(author.more.logo.clone()))
                                .name(SharedString::from(author.name.clone()))
                                .user_id(author.id.clone().map(SharedString::from))
                                .size(LOGO_SIZE)
                                .is_button(true)
                                .on_press(move |window, cx| on_press(window, cx))
                                .gift_worn_frame(
                                    SharedString::from(format!("chat-profile-member-worn-frame-{user_id}")),
                                    Some(user_id.clone()),
                                    LOGO_SIZE,
                                ),
                        )
                        .when(is_online, |frame| {
                            frame.child(
                                div()
                                    .absolute()
                                    .left(px(dot_origin))
                                    .top(px(dot_origin))
                                    .size(px(dot_outer))
                                    .rounded_full()
                                    .border(px(ONLINE_DOT_STROKE))
                                    .border_color(theme.bg._100)
                                    .bg(theme.semantic.success_100),
                            )
                        }),
                )
                .child(
                    vstack(2.0)
                        .flex_1()
                        .min_w_0()
                        .align(HorizontalAlignment::Leading)
                        .child(
                            hstack(5.0)
                                .min_w_0()
                                .max_w_full()
                                .child(
                                    MainText::new(author.name.clone())
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(name_color)
                                        .number_of_lines(1),
                                )
                                .when(author.more.is_official, |line| line.child(OfficialBadge::new().size(px(14.0))))
                                .when(author.more.is_premium, |line| {
                                    line.child(
                                        PremiumBadge::new()
                                            .user_id(author.id.clone().map(SharedString::from))
                                            .emoji(author.more.premium_badge_emoji.clone().map(SharedString::from))
                                            .size(14.0),
                                    )
                                }),
                        )
                        .child(
                            MainText::new(status_text)
                                .px(12.0)
                                .color(if is_online { theme.semantic.success_100 } else { theme.text.secondary })
                                .number_of_lines(1),
                        ),
                )
                .child(div().flex_none().w(px(6.0)))
                .children(Self::rank_chip(&member))
                .gift_worn_nameplate(
                    SharedString::from(format!("chat-profile-member-nameplate-{}", member.user_id)),
                    Some(member.user_id.clone()),
                ),
        )
    }
}
