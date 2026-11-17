use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Empty, Entity, FontWeight, ListAlignment, Pixels, Point, SharedString,
    StyleRefinement, Subscription, Task, WeakEntity, Window, };
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore};
use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileTabErrorState;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_members_tab::members_end_probe;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::{
    ForumMemberGrouping, ForumMemberGroupingViewerRights, MemberRoleGroup,
};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_members::member_rights_sheet::{
    MemberRightsSheetPresentation, MemberRightsTarget,
};
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::MemberRolePill;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMember, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const PREFETCH_THRESHOLD: usize = 8;
const LOGO_SIZE: f32 = 40.0;
const ONLINE_DOT_SIZE: f32 = 10.0;
const ONLINE_DOT_STROKE: f32 = 1.5;
const ONLINE_DOT_OFFSET: f32 = 14.0;

fn page_id(chat_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("chat-settings-members-{chat_id}-{name}")))
}

fn load_members(chat_id: &str, force_refresh: bool, cx: &mut App) {
    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_members(chat_id, None, None, force_refresh, cx));
}

fn refresh_members(chat_id: &str, cx: &mut App) {
    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_members(chat_id, None, cx));
}

enum ChatSettingsMembersRow {
    AddMembers(ChatInfo),
    GroupHeader(MemberRoleGroup),
    Member { member: ChatMember, index: usize },
}

impl ChatSettingsMembersRow {
    fn key(&self) -> String {
        match self {
            Self::AddMembers(_) => "add-members".to_owned(),
            Self::GroupHeader(group) => format!("group-{}", group.id),
            Self::Member { member, .. } => format!("member-{}", member.user_id),
        }
    }
}

/// members settings view, not an element since the stack keeps it while a member profile is on top
/// the initial load flag and list position must outlive that push
pub struct ChatSettingsMembers {
    chat_id: String,
    content: Entity<ChatSettingsMembersContent>,
    did_initial_load: bool,
    list: ListUi,
    row_keys: Vec<String>,
    _subscriptions: [Subscription; 4],
}

impl ChatSettingsMembers {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let presence = PresenceServicesStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let page = cx.entity();
        Self {
            chat_id: chat_id.unwrap_or_default(),
            content: cx.new(|cx| ChatSettingsMembersContent::new(&page, cx)),
            did_initial_load: false,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            row_keys: Vec::new(),
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
                cx.observe(&presence, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn initial_load(&mut self, cx: &mut Context<Self>) {
        let is_return = self.did_initial_load;
        self.did_initial_load = true;
        load_members(&self.chat_id, !is_return, cx);
        if !is_return {
            let chat_id = self.chat_id.clone();
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_roles(&chat_id, cx));
        }
    }

    fn sync_list(&mut self, rows: &[ChatSettingsMembersRow]) {
        let keys: Vec<String> = rows.iter().map(ChatSettingsMembersRow::key).collect();
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end = keys
            .iter()
            .rev()
            .zip(self.row_keys.iter().rev())
            .take(tail_limit)
            .take_while(|(new, old)| new == old)
            .count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }
}

impl Render for ChatSettingsMembers {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(AnyView::from(self.content.clone()).cached(StyleRefinement::default().size_full()))
    }
}

/// screen body under the rights sheet, redraws when the page publishes and not when the sheet moves
struct ChatSettingsMembersContent {
    page: WeakEntity<ChatSettingsMembers>,
    _page_observer: Subscription,
}

impl ChatSettingsMembersContent {
    fn new(page: &Entity<ChatSettingsMembers>, cx: &mut Context<Self>) -> Self {
        Self { page: page.downgrade(), _page_observer: cx.observe(page, |_, _, cx| cx.notify()) }
    }
}

impl Render for ChatSettingsMembersContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(this) = self.page.upgrade() else {
            return Empty.into_any_element();
        };
        let chat_id = this.read(cx).chat_id.clone();
        let nav_store = env::<NavigationStoreKey>(cx);

        let task_this = this.downgrade();
        use_task(page_id(&chat_id, "task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_this.update(cx, |page, cx| page.initial_load(cx)));
        });

        let chat = ChatServicesStore::shared(cx).update(cx, |services, cx| services.chat_info(&chat_id, cx));
        let title = tr(if chat.as_ref().is_some_and(|chat| chat.r#type == ChatType::Channel) {
            "chat_profile_subscribers"
        } else {
            "chat_settings_members"
        });

        let back_store = nav_store.clone();
        let keys_chat_id = chat_id.clone();
        let shell = SettingsWrapperUi::new(title, move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(ChatSettingsMembersBody {
            parts: ChatSettingsMembersParts { chat_id: chat_id.clone(), page: this.clone(), nav_store },
        });

        div()
            .relative()
            .size_full()
            .child(PageRefreshKeys::new(
                page_id(&chat_id, "keys"),
                move |_, cx| refresh_members(&keys_chat_id, cx),
                shell,
            ))
            .into_any_element()
    }
}

/// what the rows and menus of the screen need, each holds its own copy
#[derive(Clone)]
struct ChatSettingsMembersParts {
    chat_id: String,
    page: Entity<ChatSettingsMembers>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl ChatSettingsMembersParts {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn direct_conversation_id(member: &ChatMember, cx: &mut App) -> String {
        let user = member.to_user();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.direct_conversation_id(&user, cx))
    }

    fn open_chat_profile(&self, member: &ChatMember, cx: &mut App) {
        let chat_id = Self::direct_conversation_id(member, cx);
        if chat_id.is_empty() {
            self.push(RootRoute::UserPage { user_id: Some(member.user_id.clone()), tag: Some(member.tag.clone()) }, cx);
            return;
        }
        self.push(RootRoute::ChatProfile { chat_id: Some(chat_id) }, cx);
    }

    fn present_rights(&self, member: &ChatMember, restrictions_only: bool, window: &mut Window, cx: &mut App) {
        let target = MemberRightsTarget::new(member.clone(), restrictions_only);
        let chat_id = self.chat_id.clone();
        MemberRightsSheetPresentation::shared(cx)
            .update(cx, |sheet, cx| sheet.present(chat_id, target, |_: &mut App| {}, window, cx));
    }

    fn member_action(&self, member: &ChatMember, id: &str, window: &mut Window, cx: &mut App) {
        match id {
            "manage" => self.present_rights(member, false, window, cx),
            "profile" => self.open_chat_profile(member, cx),
            "message" => {
                let chat_id = Self::direct_conversation_id(member, cx);
                if !chat_id.is_empty() {
                    self.push(RootRoute::Chat { chat_id: Some(chat_id) }, cx);
                }
            }
            "copy" => {
                let value = if member.tag.is_empty() { member.name.clone() } else { format!("@{}", member.tag) };
                UIPasteboard::general().set_string(&value, cx);
            }
            "mention" => {
                if !member.tag.is_empty() {
                    UIPasteboard::general().set_string(&format!("@{}", member.tag), cx);
                }
            }
            "restrict" => self.present_rights(member, true, window, cx),
            "remove" => {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.confirm_remove_member(&self.chat_id, member, cx));
            }
            "report" => {
                let me = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
                if !member.user_id.is_empty() && member.user_id != me {
                    let subject = ReportSubject {
                        name: member.name.clone(),
                        tag: member.tag.clone(),
                        avatar_url: member.avatar_url.clone(),
                        user_id: Some(member.user_id.clone()),
                    };
                    let target = ReportTarget::User(member.user_id.clone());
                    ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, Some(subject), None, cx));
                }
            }
            _ => {}
        }
    }

    fn open_add_members(&self, chat: &ChatInfo, cx: &mut App) {
        let chat_id = chat.id.clone();
        let members = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members(&self.chat_id, cx));
        let mut seated: HashSet<String> = members.into_iter().map(|member| member.user_id).collect();
        if let Some(me) = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()) {
            seated.insert(me);
        }
        PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            let _ = store.present(
                PeoplePickerMode::SearchAll,
                0,
                Vec::new(),
                seated,
                tr("chat_members_add"),
                None,
                PeoplePickerHostContext::Root,
                move |picked, cx| {
                    if picked.is_empty() {
                        return;
                    }
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.add_members(&chat_id, &picked, cx));
                },
                cx,
            );
        });
    }

    fn rows(&self, members: &[ChatMember], chat: Option<&ChatInfo>, cx: &mut App) -> Vec<ChatSettingsMembersRow> {
        let mut rows = Vec::with_capacity(members.len() + 1);
        if let Some(chat) = chat
            && ChatProfilePermissions::distributes_invites(chat, cx)
        {
            rows.push(ChatSettingsMembersRow::AddMembers(chat.clone()));
        }
        let mut seen: HashSet<String> = HashSet::with_capacity(members.len());
        let mut display_index = 0usize;
        let mut push_member = |rows: &mut Vec<ChatSettingsMembersRow>, member: &ChatMember| {
            if seen.insert(member.user_id.clone()) {
                rows.push(ChatSettingsMembersRow::Member { member: member.clone(), index: display_index });
                display_index += 1;
            }
        };
        if ForumMemberGrouping::should_group(&self.chat_id, None, chat, cx) {
            let has_more = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members_has_more(&self.chat_id, cx));
            let unloaded_total = if has_more { chat.map(|chat| chat.member_count) } else { None };
            for group in ForumMemberGrouping::groups(members, &self.chat_id, unloaded_total, cx) {
                let group_members = group.members.clone();
                rows.push(ChatSettingsMembersRow::GroupHeader(group));
                for member in &group_members {
                    push_member(&mut rows, member);
                }
            }
        } else {
            for member in &ForumMemberGrouping::presence_ordered(members, cx) {
                push_member(&mut rows, member);
            }
        }
        rows
    }

    fn group_header(group: &MemberRoleGroup, theme: &AppTheme) -> Div {
        hstack(6.0)
            .w_full()
            .px(px(16.0))
            .pt(px(14.0))
            .pb(px(4.0))
            .child(
                MainText::new(group.title.to_uppercase())
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(group.color.map(|color| theme.legible(color)).unwrap_or(theme.text.secondary))
                    .line_limit(1),
            )
            .child(div().flex_none().child(MainText::new(group.count().to_string()).px(12.0).color(theme.text.secondary)))
            .child(spacer_min_w(0.0))
    }

    fn member_row(
        &self,
        member: &ChatMember,
        rights: ForumMemberGroupingViewerRights,
        first_other_member_id: Option<&str>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Div {
        let actions = ForumMemberGrouping::member_actions(member, &self.chat_id, rights, cx);
        let is_first_other = first_other_member_id == Some(member.user_id.as_str());
        let content_member = member.clone();
        let content_parts = self.clone();
        let action_member = member.clone();
        let action_parts = self.clone();
        let tap_member = member.clone();
        let tap_parts = self.clone();
        vstack(1.0)
            .w_full()
            .child(
                div().w_full().child(
                    HoldContextMenuContainer::new(
                        SharedString::from(format!("chat-member-hold-{}", member.user_id)),
                        HoldContextMenuAlignment::Leading,
                        Rc::new(move |_: &App| actions.clone()),
                        Rc::new(move |_: &mut Window, _: &mut App| {
                            let parts = content_parts.clone();
                            let profile_member = content_member.clone();
                            MemberRow {
                                author: content_member.to_author_info(),
                                member: content_member.clone(),
                                on_open_avatar_profile: Rc::new(move |_: &mut Window, cx: &mut App| {
                                    parts.open_chat_profile(&profile_member, cx);
                                }),
                            }
                            .guide_anchor(is_first_other.then_some(RiseGuideAnchor::ChatMembersFirstOther))
                            .into_any_element()
                        }),
                    )
                    .show_emoji_list(false)
                    .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                        action_parts.member_action(&action_member, &id, window, cx);
                    }))
                    .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                        tap_parts.open_chat_profile(&tap_member, cx);
                    })),
                ),
            )
            .child(div().w_full().h(px(0.5)).bg(theme.border._100))
    }

    fn add_members_row(&self, chat: &ChatInfo) -> Div {
        let parts = self.clone();
        let target = chat.clone();
        div().w_full().py(px(12.0)).child(GroupedBtns::new(
            page_id(&self.chat_id, "add-members"),
            vec![
                GroupedBtn::new("addMembers", tr("chat_members_add"))
                    .icon("person.badge.plus")
                    .icon_background(ChatTileColor::blue())
                    .show_chevron(false)
                    .callback(move |_, cx| parts.open_add_members(&target, cx)),
            ],
        ))
    }
}

#[derive(IntoElement)]
struct ChatSettingsMembersBody {
    parts: ChatSettingsMembersParts,
}

impl RenderOnce for ChatSettingsMembersBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parts = self.parts;
        let chat_id = parts.chat_id.clone();
        let services = ChatServicesStore::shared(cx);
        let members = services.update(cx, |services, cx| services.members(&chat_id, cx));
        let members = ForumMemberGrouping::viewer_visible_members(members, &chat_id, cx);
        let status = services.read(cx).member_list_render_status(&chat_id, !members.is_empty());
        let is_empty = members.is_empty();
        let loading_id = page_id(&chat_id, "skeleton");
        let retry_chat_id = chat_id.clone();
        let render_id = page_id(&chat_id, "render");

        SaiDataRender::new(status, move |_, cx| {
            let theme = rise_ui::theme(cx).clone();
            let chat = ChatServicesStore::shared(cx).update(cx, |services, cx| services.chat_info(&chat_id, cx));
            let rights = ForumMemberGrouping::viewer_rights(&chat_id, cx);
            let me = ForumMemberGrouping::my_user_id(cx);
            let first_other_member_id = members
                .iter()
                .find(|member| !member.user_id.is_empty() && member.user_id != me)
                .map(|member| member.user_id.clone());
            let has_more = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members_has_more(&chat_id, cx));
            let relative_id = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members_last_id(&chat_id, cx));
            let cache_key = ChatServicesStore::members_cache_key(&chat_id);
            let rows = Rc::new(parts.rows(&members, chat.as_ref(), cx));
            let item_count = rows.iter().filter(|row| matches!(row, ChatSettingsMembersRow::Member { .. })).count();
            parts.page.update(cx, |page, _| page.sync_list(&rows));
            let top_inset = list_shell_floating_top_inset(cx);
            let refresh_chat_id = chat_id.clone();
            let screen = parts.page.clone();
            let row_count = rows.len();
            let probe_chat_id = chat_id.clone();
            let list = screen.read(cx).list.refreshable(
                page_id(&chat_id, "list"),
                move |_, cx| {
                    refresh_members(&refresh_chat_id, cx);
                    Task::ready(())
                },
                move |index, _, cx| {
                    let theme = rise_ui::theme(cx).clone();
                    let Some(row) = rows.get(index) else {
                        return Empty.into_any_element();
                    };
                    let content = match row {
                        ChatSettingsMembersRow::AddMembers(chat) => parts.add_members_row(chat).into_any_element(),
                        ChatSettingsMembersRow::GroupHeader(group) => {
                            ChatSettingsMembersParts::group_header(group, &theme).into_any_element()
                        }
                        ChatSettingsMembersRow::Member { member, index: member_index } => {
                            let load_chat_id = parts.chat_id.clone();
                            parts
                                .member_row(member, rights, first_other_member_id.as_deref(), &theme, cx)
                                .sai_pagination_prefetch(
                                    SharedString::from(format!("chat-member-prefetch-{}", member.user_id)),
                                    SaiPaginationPrefetchOptions::new(
                                        *member_index,
                                        item_count,
                                        cache_key.clone(),
                                        relative_id.clone(),
                                        has_more,
                                    )
                                    .threshold(PREFETCH_THRESHOLD),
                                    move |_, cx| {
                                        ChatInteractionsStore::shared(cx)
                                            .update(cx, |store, cx| store.load_more_members(&load_chat_id, None, cx));
                                    },
                                )
                                .into_any_element()
                        }
                    };
                    let top_padding = if index == 0 { top_inset } else { 1.0 };
                    let row = div().w_full().pt(px(top_padding)).child(content);
                    if index + 1 == row_count {
                        row.child(members_end_probe(&probe_chat_id, None, &probe_chat_id)).into_any_element()
                    } else {
                        row.into_any_element()
                    }
                },
            );
            div().size_full().bg(theme.bg._100).child(list)
        })
        .id(render_id)
        .is_empty(is_empty)
        .no_data_text(tr("chat_members_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(move |_, cx| {
            let theme = rise_ui::theme(cx).clone();
            scroll_view(loading_id.clone())
                .size_full()
                .scroll_disabled(true)
                .shows_indicators(false)
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .child(UserListSkeleton::new())
        })
        .sai_data_render_error(move |_, _| {
            let retry_chat_id = retry_chat_id.clone();
            ChatProfileTabErrorState::new(tr("error_fetching_data"), move |_, cx| load_members(&retry_chat_id, true, cx))
        })
    }
}

#[derive(IntoElement)]
struct MemberRow {
    author: AuthorInfo,
    member: ChatMember,
    on_open_avatar_profile: ButtonAction,
}

impl MemberRow {
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

impl RenderOnce for MemberRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let member = self.member;
        let author = self.author;
        let is_online = PresenceServicesStore::shared(cx).read(cx).is_online(&member.user_id, member.is_online);
        let name_color = member
            .name_color()
            .map(|color| theme.legible(color))
            .or_else(|| PremiumNameColor::color_for_user(author.id.as_deref(), Some(&author.more), cx));
        let on_press = self.on_open_avatar_profile;
        let user_id = member.user_id.clone();
        let dot_outer = ONLINE_DOT_SIZE + ONLINE_DOT_STROKE;
        let dot_origin = (LOGO_SIZE - dot_outer) / 2.0 + ONLINE_DOT_OFFSET;

        div().w_full().bg(theme.bg._100).child(
            hstack(10.0)
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .child(
                    div()
                        .id(SharedString::from(format!("chat-member-logo-frame-{user_id}")))
                        .occlude()
                        .relative()
                        .size(px(LOGO_SIZE))
                        .flex_none()
                        .child(
                            UserLogoUi::new(SharedString::from(format!("chat-member-logo-{user_id}")))
                                .source(SharedString::from(author.more.logo.clone()))
                                .name(SharedString::from(author.name.clone()))
                                .user_id(author.id.clone().map(SharedString::from))
                                .size(LOGO_SIZE)
                                .is_button(true)
                                .on_press(move |window, cx| on_press(window, cx))
                                .gift_worn_frame(
                                    SharedString::from(format!("chat-member-worn-frame-{user_id}")),
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
                            hstack(4.0)
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
                        .when(!author.tag.is_empty(), |stack| {
                            stack.child(
                                MainText::new(format!("@{}", author.tag))
                                    .px(12.0)
                                    .color(theme.text.secondary)
                                    .number_of_lines(1),
                            )
                        }),
                )
                .child(div().flex_none().w(px(6.0)))
                .children(Self::rank_chip(&member))
                .gift_worn_nameplate(
                    SharedString::from(format!("chat-member-nameplate-{}", member.user_id)),
                    Some(member.user_id.clone()),
                ),
        )
    }
}
