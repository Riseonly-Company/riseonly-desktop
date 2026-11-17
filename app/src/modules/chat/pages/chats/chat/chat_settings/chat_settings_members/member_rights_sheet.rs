use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyWindowHandle, App, Context, Empty, Entity, FontWeight, Hsla, Render, RenderOnce, SharedString,
    Subscription, Window,
};
use rise_canon::rise_key::chat as key;
use rise_canon::{ChatPermissionBit, RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::button_style::DISABLED_OPACITY;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::{SwitchRowUi, SwitchUi};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_ui::InputUi;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_host::NotifierHost;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatMember, ChatPerm, ChatPermExt, ChatRole, ChatType,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsBoolResult, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_well::ColorPickerWell;

const RANK_RULE_ENDPOINT: &str = "chat_edit_member";
const RANK_RULE_POINTER: &str = "/custom_title";
const ROLE_ADMIN: &str = "ADMIN";
const ROLE_MEMBER: &str = "MEMBER";
const CARD_RADIUS: f32 = 16.0;

const RESTRICTION_RULES: [(&[&str], i64); 7] = [
    (&[key::CAN_SEND_MEDIA], ChatPerm::ATTACH_MEDIA),
    (&[key::CAN_SEND_STICKERS], ChatPerm::SEND_STICKERS),
    (&[key::CAN_SEND_POLLS], ChatPerm::SEND_POLLS),
    (&[key::CAN_SEND_WEB_PAGE_PREVIEWS], ChatPerm::EMBED_LINKS),
    (&[key::CAN_SEND_AUDIO, key::CAN_SEND_VOICE], ChatPerm::SEND_VOICE),
    (&[key::CAN_SEND_DOCUMENTS], ChatPerm::SEND_DOCUMENTS),
    (&[key::CAN_REACT], ChatPerm::ADD_REACTIONS),
];

const SHEET_HEADER_HEIGHT: f32 = 44.0;

/// the mode travels with the presented item
/// a separate flag could be read stale and open the full rights sheet for restrict
#[derive(Clone)]
pub struct MemberRightsTarget {
    pub member: ChatMember,
    pub restrictions_only: bool,
}

impl MemberRightsTarget {
    pub fn new(member: ChatMember, restrictions_only: bool) -> Self {
        Self { member, restrictions_only }
    }

    pub fn id(&self) -> String {
        format!("{}|{}", self.member.id(), self.restrictions_only)
    }
}

fn rank_rule() -> RiseFieldRule {
    RiseFieldRule::new(RANK_RULE_ENDPOINT, RANK_RULE_POINTER)
}

fn divider(theme: &AppTheme) -> impl IntoElement {
    div().flex_none().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
}

fn role_matches(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[derive(IntoElement)]
pub struct MemberRightsSheet {
    chat_id: String,
    member: ChatMember,
    restrictions_only: bool,
    on_saved: Rc<dyn Fn(&mut App)>,
}

impl MemberRightsSheet {
    pub fn new(
        chat_id: impl Into<String>,
        member: ChatMember,
        restrictions_only: bool,
        on_saved: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self { chat_id: chat_id.into(), member, restrictions_only, on_saved: Rc::new(on_saved) }
    }
}

impl RenderOnce for MemberRightsSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let action = dismiss(cx);
        let window_handle = window.window_handle();
        let id = SharedString::from(format!("member-rights-sheet-{}-{}", self.member.id(), self.restrictions_only));
        let state = window.use_keyed_state(id, cx, move |_, cx| {
            MemberRightsSheetState::new(self.chat_id, self.member, self.restrictions_only, self.on_saved, cx)
        });
        state.update(cx, |state, _| state.dismiss_action = Some((action, window_handle)));
        state
    }
}

struct MemberRightsSheetState {
    chat_id: String,
    member: ChatMember,
    restrictions_only: bool,
    on_saved: Rc<dyn Fn(&mut App)>,
    dismiss_action: Option<(DismissAction, AnyWindowHandle)>,
    notifier_host: Entity<NotifierHost>,

    is_admin: bool,
    title: HashMap<String, String>,
    can_change_info: bool,
    can_manage_chat: bool,
    can_manage_reactions: bool,
    can_delete_messages: bool,
    can_ban_members: bool,
    can_invite_users: bool,
    can_pin_messages: bool,
    can_promote_members: bool,
    can_manage_stories: bool,
    can_manage_video_chats: bool,
    is_anonymous: bool,
    badge_color: Hsla,
    has_badge_color: bool,
    color_touched: bool,
    saving: bool,
    assigned_role_ids: HashSet<String>,
    role_writes_in_flight: i64,
    perm_allow: i64,
    perm_deny: i64,
    perm_touched: bool,
    perm_touched_bits: i64,
    restriction_changes: HashMap<String, bool>,
    restriction_permissions: Option<i64>,
    restriction_load_failed: bool,
    restriction_generation: u64,
    restriction_domain_id: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl MemberRightsSheetState {
    fn new(
        chat_id: String,
        member: ChatMember,
        restrictions_only: bool,
        on_saved: Rc<dyn Fn(&mut App)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let seed = member.is_admin() || member.is_owner();
        tracing::warn!(
            target: "riseonly::X1766",
            chat = %chat_id,
            member = %member.user_id,
            role = %member.role,
            change_info = member.can_change_info,
            manage_chat = member.can_manage_chat,
            reactions = member.can_manage_reactions,
            delete = member.can_delete_messages,
            ban = member.can_ban_members,
            invite = member.can_invite_users,
            pin = member.can_pin_messages,
            promote = member.can_promote_members,
            stories = member.can_manage_stories,
            video = member.can_manage_video_chats,
            "rights sheet opened with member rights"
        );
        let has_color = !member.custom_color.is_empty();
        let blue = UIColor::system_blue().resolved_color(rise_ui::theme(cx).appearance);

        let services = ChatServicesStore::shared(cx);
        let presence = PresenceServicesStore::shared(cx);
        let registry = RiseConversationDomainRegistry::shared(cx);
        let mut subscriptions = vec![
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&presence, |_, _, cx| cx.notify()),
            cx.observe(&registry, |_, _, cx| cx.notify()),
        ];
        if let Some(sandbox) = GuideSandbox::existing(cx) {
            subscriptions.push(cx.observe(&sandbox, |_, _, cx| cx.notify()));
        }

        GuideFacts::set(RiseGuideSignal::MemberRightsOpen, true, cx);
        cx.on_release(|sheet, cx| {
            sheet.restriction_generation = sheet.restriction_generation.wrapping_add(1);
            GuideFacts::set(RiseGuideSignal::MemberRightsOpen, false, cx);
            GuideFacts::set(RiseGuideSignal::MemberTitleFilled, false, cx);
        })
        .detach();

        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |sheet, cx| sheet.run_task(cx));
        });

        Self {
            chat_id,
            is_admin: seed,
            title: HashMap::from([(key::TITLE.to_owned(), member.custom_title.clone())]),
            can_change_info: member.can_change_info || !seed,
            can_manage_chat: member.can_manage_chat || !seed,
            can_manage_reactions: member.can_manage_reactions || !seed,
            can_delete_messages: member.can_delete_messages || !seed,
            can_ban_members: member.can_ban_members || !seed,
            can_invite_users: member.can_invite_users || !seed,
            can_pin_messages: member.can_pin_messages || !seed,
            can_promote_members: member.can_promote_members,
            can_manage_stories: member.can_manage_stories || !seed,
            can_manage_video_chats: member.can_manage_video_chats || !seed,
            is_anonymous: member.is_anonymous,
            badge_color: if has_color { color(&member.custom_color) } else { blue },
            has_badge_color: has_color,
            color_touched: false,
            saving: false,
            assigned_role_ids: HashSet::new(),
            role_writes_in_flight: 0,
            perm_allow: 0,
            perm_deny: 0,
            perm_touched: false,
            perm_touched_bits: 0,
            restriction_changes: HashMap::new(),
            restriction_permissions: None,
            restriction_load_failed: false,
            restriction_generation: 0,
            restriction_domain_id: None,
            member,
            restrictions_only,
            on_saved,
            dismiss_action: None,
            notifier_host: cx.new(NotifierHost::new),
            _subscriptions: subscriptions,
        }
    }

    fn run_task(&mut self, cx: &mut Context<Self>) {
        if self.restrictions_only {
            self.load_restriction_permissions(cx);
            return;
        }
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let interactions = ChatInteractionsStore::shared(cx);
        interactions.update(cx, |store, cx| store.load_roles(&chat_id, cx));
        self.assigned_role_ids = self.member.role_ids.iter().cloned().collect();
        cx.notify();
        let this = cx.weak_entity();
        interactions.update(cx, |store, cx| {
            store.load_member_override(
                &chat_id,
                &participant_id,
                Box::new(move |allow: i64, deny: i64, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |sheet, cx| {
                            sheet.perm_allow = allow;
                            sheet.perm_deny = deny;
                            cx.notify();
                        });
                    });
                }),
                cx,
            );
        });
    }

    /// a completion the stores may call synchronously, so it waits for the end of the current update
    /// this sheet is usually the entity being updated then
    fn after(
        cx: &Context<Self>,
        completion: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> ChatInteractionsBoolResult {
        let this = cx.weak_entity();
        Box::new(move |ok: bool, cx: &mut App| {
            cx.defer(move |cx| {
                let _ = this.update(cx, |sheet, cx| completion(sheet, ok, cx));
            });
        })
    }

    fn dismiss(&self, cx: &mut App) {
        let Some((action, window_handle)) = self.dismiss_action.clone() else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| action.call_as_function(window, cx));
        });
    }

    fn on_saved(&self, cx: &mut App) {
        (self.on_saved)(cx);
    }

    fn title_text(&self) -> &str {
        self.title.get(key::TITLE).map_or("", String::as_str)
    }

    fn chat(&self, cx: &mut App) -> Option<ChatInfo> {
        let services = ChatServicesStore::shared(cx);
        services.read(cx).chat_info(&self.chat_id, cx)
    }

    fn custom_roles(&self, cx: &mut App) -> Vec<ChatRole> {
        let chat_id = self.chat_id.clone();
        ChatServicesStore::shared(cx).update(cx, |services, cx| services.custom_roles(&chat_id, cx))
    }

    fn is_channel(&self, cx: &mut App) -> bool {
        self.chat(cx).is_some_and(|chat| chat.r#type == ChatType::Channel)
    }

    fn is_sandbox(&self, cx: &App) -> bool {
        GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_roles(Some(self.chat_id.as_str())))
    }

    fn is_self(&self, cx: &mut App) -> bool {
        let me = ForumMemberGrouping::my_user_id(cx);
        !me.is_empty() && self.member.user_id == me
    }

    fn viewer_is_owner(&self, cx: &mut App) -> bool {
        ChatPerm::is_owner_role(self.chat(cx).and_then(|chat| chat.role).as_deref())
    }

    /// a mask without view channels counts as a failed read
    fn viewer_mask(&self, cx: &mut App) -> Option<i64> {
        let chat_id = self.chat_id.clone();
        ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.effective_permissions(&chat_id, None, cx))
            .map(|permissions| permissions.mask())
            .filter(|mask| mask & ChatPerm::VIEW_CHANNELS != 0)
    }

    /// until the mask loads the owner and admin roles stand in for it like on other screens
    fn viewer_holds(&self, permission: i64, cx: &mut App) -> bool {
        if self.is_sandbox(cx) || self.viewer_is_owner(cx) {
            return true;
        }
        if let Some(mask) = self.viewer_mask(cx) {
            return ChatPerm::has(mask, permission);
        }
        ChatPerm::is_owner_or_legacy_admin_role(self.chat(cx).and_then(|chat| chat.role).as_deref())
    }

    fn outranks_member(&self, cx: &mut App) -> bool {
        ForumMemberGrouping::can_moderate(&self.member, &self.chat_id, cx)
    }

    /// edit_member needs manage members and a rank above the target unless it is yourself
    fn can_edit_identity(&self, cx: &mut App) -> bool {
        if self.is_sandbox(cx) {
            return true;
        }
        if self.is_self(cx) {
            return self.viewer_is_owner(cx)
                || self.viewer_mask(cx).is_some_and(|mask| ChatPerm::has(mask, ChatPerm::MANAGE_MEMBERS));
        }
        self.viewer_holds(ChatPerm::MANAGE_MEMBERS, cx) && self.outranks_member(cx)
    }

    /// assign_role needs manage roles and nobody but the owner hands roles to themselves
    fn can_assign_roles(&self, cx: &mut App) -> bool {
        self.is_sandbox(cx)
            || (self.viewer_holds(ChatPerm::MANAGE_ROLES, cx) && (!self.is_self(cx) || self.viewer_is_owner(cx)))
    }

    /// promoting needs the promote right and an admin rank, nobody changes their own role
    fn can_change_admin(&self, cx: &mut App) -> bool {
        if self.is_sandbox(cx) {
            return true;
        }
        if self.is_self(cx)
            || self.member.is_owner()
            || !ChatPerm::is_owner_or_legacy_admin_role(self.chat(cx).and_then(|chat| chat.role).as_deref())
            || !self.viewer_holds(ChatPerm::MANAGE_ROLES, cx)
            || !self.viewer_holds(ChatPerm::MANAGE_MEMBERS, cx)
        {
            return false;
        }
        self.outranks_member(cx)
    }

    /// admins share one rank so only the owner outranks an admin and sets their rights
    /// an admin promotion hands out the promoter own rights
    fn can_set_admin_rights(&self, cx: &mut App) -> bool {
        self.is_sandbox(cx) || (self.viewer_is_owner(cx) && !self.is_self(cx) && !self.member.is_owner())
    }

    /// set_member_permissions needs manage members and a rank above the target
    fn can_edit_overrides(&self, cx: &mut App) -> bool {
        self.is_sandbox(cx)
            || (!self.is_self(cx) && self.viewer_holds(ChatPerm::MANAGE_MEMBERS, cx) && self.outranks_member(cx))
    }

    /// chat-service ignores a member own allow and deny for an admin or a role holder with administrator
    /// so an admin has no restrictions to set
    fn overrides_apply(&self, cx: &mut App) -> bool {
        if self.is_admin {
            return false;
        }
        if self.perm_deny & ChatPerm::ADMINISTRATOR != 0 {
            return true;
        }
        !self
            .custom_roles(cx)
            .iter()
            .any(|role| self.assigned_role_ids.contains(&role.id) && role.permissions & ChatPerm::ADMINISTRATOR != 0)
    }

    fn has_editable_fields(&self, cx: &mut App) -> bool {
        if self.restrictions_only {
            return self.can_edit_restrictions(cx);
        }
        self.can_change_admin(cx)
            || self.can_edit_identity(cx)
            || (self.can_edit_overrides(cx) && self.overrides_apply(cx) && !self.custom_roles(cx).is_empty())
    }

    fn active_domain_id(cx: &mut App) -> Option<usize> {
        let registry = RiseConversationDomainRegistry::shared(cx);
        let active = registry.read(cx).active();
        active.map(|domain| Arc::as_ptr(&domain) as usize)
    }

    fn can_edit_restrictions(&self, cx: &mut App) -> bool {
        self.can_manage_restrictions(cx)
            && self.restriction_permissions.is_some()
            && self.restriction_domain_id.is_some()
            && Self::active_domain_id(cx) == self.restriction_domain_id
    }

    fn can_manage_restrictions(&self, cx: &mut App) -> bool {
        !self.is_channel(cx)
            && !self.is_self(cx)
            && !self.is_admin
            && !self.member.is_owner()
            && self.outranks_member(cx)
            && self.viewer_holds(ChatPerm::MUTE_MEMBERS, cx)
    }

    /// only a loaded mask closes the sheet
    /// between an invalidation and its reload a role holder rights are unknown and not gone
    fn viewer_lost_management(&self, cx: &mut App) -> bool {
        if self.is_sandbox(cx) || self.viewer_is_owner(cx) {
            return false;
        }
        let Some(mask) = self.viewer_mask(cx) else {
            return false;
        };
        if self.restrictions_only {
            return !ChatPerm::has(mask, ChatPerm::MUTE_MEMBERS);
        }
        !ChatPerm::has(mask, ChatPerm::MANAGE_MEMBERS) && !ChatPerm::has(mask, ChatPerm::MANAGE_ROLES)
    }

    fn sheet_title_key(&self, is_channel: bool) -> &'static str {
        if self.restrictions_only {
            return "ctx_restrict";
        }
        if self.member.is_owner() {
            return "chat_member_role_owner";
        }
        if self.member.is_admin() {
            return "chat_member_rights_admin_title";
        }
        if is_channel { "chat_member_rights_subscriber_title" } else { "chat_member_rights_member_title" }
    }

    fn header_card(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let member = &self.member;
        let presence = PresenceServicesStore::shared(cx);
        let status = presence
            .update(cx, |presence, cx| presence.status_text(&member.user_id, member.is_online, member.last_seen, cx));
        let is_online = presence.read(cx).is_online(&member.user_id, member.is_online);
        div().w_full().px(px(12.0)).child(
            hstack(12.0)
                .w_full()
                .p(px(14.0))
                .bg(theme.bg._200)
                .rounded(px(CARD_RADIUS))
                .child(
                    UserLogoUi::new("member-rights-logo")
                        .source(SharedString::from(member.avatar_url.clone()))
                        .name(SharedString::from(member.name.clone()))
                        .user_id(SharedString::from(member.user_id.clone()))
                        .size(48.0),
                )
                .child(
                    vstack(2.0)
                        .flex_1()
                        .min_w_0()
                        .align(HorizontalAlignment::Leading)
                        .child(
                            hstack(4.0)
                                .child(
                                    MainText::new(member.name.clone())
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary),
                                )
                                .when(member.is_official, |line| line.child(OfficialBadge::new().size(px(15.0))))
                                .when(member.is_premium, |line| {
                                    line.child(
                                        PremiumBadge::new()
                                            .user_id(Some(SharedString::from(member.user_id.clone())))
                                            .emoji(member.premium_badge_emoji.clone().map(SharedString::from))
                                            .size(15.0),
                                    )
                                }),
                        )
                        .child(MainText::new(status).px(13.0).color(if is_online {
                            theme.semantic.success_100
                        } else {
                            theme.text.secondary
                        })),
                ),
        )
        .into_any_element()
    }

    fn set_role(&mut self, role: &ChatRole, on: bool, cx: &mut Context<Self>) {
        let chat_id = self.chat_id.clone();
        let services = ChatServicesStore::shared(cx);
        let locked =
            !self.can_assign_roles(cx) || !services.update(cx, |services, cx| services.viewer_can_grant(&chat_id, role, cx));
        if locked || self.role_writes_in_flight != 0 || self.saving {
            return;
        }
        let role_id = role.id.clone();
        if on {
            self.assigned_role_ids.insert(role_id.clone());
        } else {
            self.assigned_role_ids.remove(&role_id);
        }
        self.role_writes_in_flight += 1;
        cx.notify();
        let participant_id = self.member.user_id.clone();
        let this = cx.weak_entity();
        let answered_role_id = role_id.clone();
        let on_result: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |ok: bool, cx: &mut App| {
            let this = this.clone();
            let role_id = answered_role_id.clone();
            cx.defer(move |cx| {
                let _ = this.update(cx, |sheet, cx| {
                    sheet.role_writes_in_flight = (sheet.role_writes_in_flight - 1).max(0);
                    if ok {
                        sheet.on_saved(cx);
                    } else if on {
                        sheet.assigned_role_ids.remove(&role_id);
                    } else {
                        sheet.assigned_role_ids.insert(role_id);
                    }
                    cx.notify();
                });
            });
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.assign_role(&chat_id, &participant_id, &role_id, on, Some(on_result), cx);
        });
    }

    fn roles_card(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let roles = self.custom_roles(cx);
        let count = roles.len();
        let busy = self.role_writes_in_flight > 0 || self.saving;
        let chat_id = self.chat_id.clone();
        let services = ChatServicesStore::shared(cx);
        let this = cx.entity();
        let can_assign = self.can_assign_roles(cx);
        let rows: Vec<AnyElement> = roles
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                let locked =
                    !can_assign || !services.update(cx, |services, cx| services.viewer_can_grant(&chat_id, &role, cx));
                let swift_color = role.swift_color();
                let read_id = role.id.clone();
                let row_key = role.id.clone();
                let name = role.name.clone();
                let is_on = Binding::entity(
                    this.clone(),
                    move |sheet: &Self| sheet.assigned_role_ids.contains(&read_id),
                    move |sheet: &mut Self, on, cx| sheet.set_role(&role, on, cx),
                );
                let toggled = is_on.clone();
                let row = plain(SharedString::from(format!("member-rights-role-{row_key}")))
                    .w_full()
                    .disabled(busy || locked)
                    .on_press(move |window, cx| toggled.toggle(window, cx))
                    .child(
                        hstack(10.0)
                            .w_full()
                            .px(px(16.0))
                            .py(px(11.0))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(12.0))
                                    .rounded_full()
                                    .bg(swift_color.unwrap_or(theme.text.secondary.opacity(0.4))),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    MainText::new(name)
                                        .px(15.0)
                                        .color(swift_color.map_or(theme.text.primary, |tint| theme.legible(tint))),
                                ),
                            )
                            .when(locked, |line| {
                                line.children(IconUi::sized("lock.fill", px(11.0), theme.text.secondary.opacity(0.6), true))
                            })
                            .child(SwitchUi::new(SharedString::from(format!("member-rights-role-switch-{row_key}")), is_on)),
                    );
                let element = div().w_full().child(row);
                if index + 1 < count {
                    return element
                        .child(
                            div()
                                .flex_none()
                                .w_full()
                                .pl(px(16.0))
                                .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.5))),
                        )
                        .into_any_element();
                }
                element.into_any_element()
            })
            .collect();
        div().w_full().px(px(12.0)).child(
            vstack(0.0)
                .w_full()
                .bg(theme.bg._200)
                .rounded(px(CARD_RADIUS))
                .overflow_hidden()
                .child(
                    div()
                        .w_full()
                        .px(px(16.0))
                        .pt(px(12.0))
                        .pb(px(6.0))
                        .child(MainText::new(tr("roles_title")).px(13.0).color(theme.text.secondary)),
                )
                .children(rows)
                .guide_anchor(RiseGuideAnchor::MemberRightsRoles),
        )
    }

    fn override_rows(&self) -> Vec<&'static ChatPermissionBit> {
        ChatPerm::EDITOR_ROWS
            .iter()
            .filter(|row| row.mask != ChatPerm::ADMINISTRATOR || self.perm_state(row.mask) != 0)
            .collect()
    }

    fn permission_overrides_card(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.override_rows();
        let count = rows.len();
        let held: Vec<bool> = rows.iter().map(|row| self.viewer_holds(row.mask, cx)).collect();
        let this = cx.entity();
        let elements: Vec<AnyElement> = rows
            .into_iter()
            .zip(held)
            .enumerate()
            .map(|(index, (row, held))| {
                let mask = row.mask;
                let state = self.perm_state(mask);
                let item = |id: i64, state: i64, label: &'static str, item_key: &'static str| {
                    let this = this.clone();
                    ContextMenuItem::new(id, tr(label), None, item_key, move |_, cx| {
                        this.update(cx, |sheet, cx| sheet.set_perm_state(mask, state, cx));
                    })
                };
                let (label_key, label_color) = match state {
                    1 => ("member_perm_allow", theme.semantic.success_100),
                    2 => ("member_perm_deny", theme.semantic.error_100),
                    _ => ("member_perm_inherit", theme.text.secondary),
                };
                let line = hstack(None::<f32>)
                    .w_full()
                    .px(px(16.0))
                    .py(px(9.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pr(px(8.0))
                            .child(MainText::new(tr(row.i18n_key)).px(15.0).color(theme.text.primary).number_of_lines(2)),
                    )
                    .when(!held, |line| {
                        line.child(
                            hstack(4.0)
                                .opacity(DISABLED_OPACITY)
                                .children(IconUi::sized("lock.fill", px(11.0), theme.text.secondary.opacity(0.6), true))
                                .child(
                                    MainText::new(tr(label_key))
                                        .px(14.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(label_color),
                                ),
                        )
                    })
                    .when(held, |line| {
                        line.child(
                            ContextMenuUi::new(
                                SharedString::from(format!("member-rights-override-{mask}")),
                                vec![
                                    item(0, 0, "member_perm_inherit", "0"),
                                    item(1, 1, "member_perm_allow", "1"),
                                    item(2, 2, "member_perm_deny", "2"),
                                ],
                                MainText::new(tr(label_key)).px(14.0).font_weight(FontWeight::SEMIBOLD).color(label_color),
                            )
                            .selected(state.to_string()),
                        )
                    });
                let element = div().w_full().child(line);
                if index + 1 < count {
                    return element.child(divider(theme)).into_any_element();
                }
                element.into_any_element()
            })
            .collect();
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                div()
                    .px(px(16.0))
                    .child(MainText::new(tr("member_perm_overrides").to_uppercase()).px(12.0).color(theme.text.secondary)),
            )
            .child(
                div().w_full().px(px(12.0)).child(
                    vstack(0.0).w_full().bg(theme.bg._200).rounded(px(CARD_RADIUS)).child(div().w_full().children(elements)),
                ),
            )
    }

    fn perm_state(&self, bit: i64) -> i64 {
        if self.perm_allow & bit != 0 {
            return 1;
        }
        if self.perm_deny & bit != 0 {
            return 2;
        }
        0
    }

    fn set_perm_state(&mut self, bit: i64, state: i64, cx: &mut Context<Self>) {
        if !self.viewer_holds(bit, cx) {
            return;
        }
        self.perm_touched = true;
        self.perm_touched_bits |= bit;
        self.perm_allow &= !bit;
        self.perm_deny &= !bit;
        if state == 1 {
            self.perm_allow |= bit;
        } else if state == 2 {
            self.perm_deny |= bit;
        }
        cx.notify();
    }

    fn admin_toggle_card(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.can_change_admin(cx);
        let row = Self::toggle_row(
            "member-rights-make-admin",
            tr("chat_member_make_admin"),
            self.is_admin,
            Self::field(cx, |sheet| sheet.is_admin, |sheet, value| sheet.is_admin = value),
            enabled,
        );
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                div().w_full().px(px(12.0)).child(
                    div()
                        .w_full()
                        .bg(theme.bg._200)
                        .rounded(px(CARD_RADIUS))
                        .overflow_hidden()
                        .child(row)
                        .guide_anchor(RiseGuideAnchor::MemberRightsAdmin),
                ),
            )
    }

    /// read only for anyone but the owner
    /// an admin own rights or for a promoted one what chat-service gives them
    /// never the promote right
    fn cap_value(&self, value: bool, promoted_gets: impl FnOnce(&mut App) -> bool, cx: &mut App) -> bool {
        if self.can_set_admin_rights(cx) || self.member.is_admin() {
            return value;
        }
        promoted_gets(cx)
    }

    fn change_info_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_change_info, |cx| self.viewer_holds(ChatPerm::MANAGE_GROUP, cx), cx)
    }

    fn manage_chat_right(&self, cx: &mut App) -> bool {
        self.cap_value(
            self.can_manage_chat,
            |cx| self.viewer_holds(ChatPerm::MANAGE_GROUP | ChatPerm::MANAGE_TOPICS | ChatPerm::MANAGE_SECTIONS, cx),
            cx,
        )
    }

    fn manage_reactions_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_manage_reactions, |cx| self.viewer_holds(ChatPerm::MANAGE_REACTIONS, cx), cx)
    }

    fn delete_messages_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_delete_messages, |cx| self.viewer_holds(ChatPerm::MANAGE_MESSAGES, cx), cx)
    }

    fn ban_members_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_ban_members, |cx| self.viewer_holds(ChatPerm::BAN_MEMBERS, cx), cx)
    }

    fn invite_users_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_invite_users, |cx| self.viewer_holds(ChatPerm::CREATE_INVITES, cx), cx)
    }

    fn pin_messages_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_pin_messages, |cx| self.viewer_holds(ChatPerm::PIN_MESSAGES, cx), cx)
    }

    fn promote_members_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_promote_members, |_| false, cx)
    }

    fn manage_stories_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_manage_stories, |cx| self.viewer_holds(ChatPerm::MANAGE_GROUP, cx), cx)
    }

    fn manage_video_chats_right(&self, cx: &mut App) -> bool {
        self.cap_value(self.can_manage_video_chats, |cx| self.viewer_holds(ChatPerm::MANAGE_VOICE_CHANNEL, cx), cx)
    }

    /// rights row a role change leaves behind
    /// for a promotion what chat-service grants, for a demotion nothing since it clears the admin columns
    fn granted_rights(&self, to_admin: bool, cx: &mut App) -> HashMap<String, bool> {
        let ban = self.ban_members_right(cx);
        let rights = [
            (key::CAN_CHANGE_INFO, self.change_info_right(cx)),
            (key::CAN_MANAGE_CHAT, self.manage_chat_right(cx)),
            (key::CAN_MANAGE_REACTIONS, self.manage_reactions_right(cx)),
            (key::CAN_DELETE_MESSAGES, self.delete_messages_right(cx)),
            (key::CAN_BAN_MEMBERS, ban),
            (key::CAN_KICK_MEMBERS, ban),
            (key::CAN_RESTRICT_MEMBERS, ban),
            (key::CAN_INVITE_USERS, self.invite_users_right(cx)),
            (key::CAN_PIN_MESSAGES, self.pin_messages_right(cx)),
            (key::CAN_PROMOTE_MEMBERS, self.promote_members_right(cx)),
            (key::CAN_MANAGE_STORIES, self.manage_stories_right(cx)),
            (key::CAN_MANAGE_VIDEO_CHATS, self.manage_video_chats_right(cx)),
        ];
        rights.into_iter().map(|(name, value)| (name.to_owned(), to_admin && value)).collect()
    }

    fn admin_perms_card(&self, theme: &AppTheme, is_channel: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let locked = !self.can_set_admin_rights(cx);
        let enabled = !locked;
        let mut rows: Vec<AnyElement> = Vec::new();

        let shown = self.change_info_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-change-info",
            tr(if is_channel { "chat_admin_perm_change_info_channel" } else { "chat_admin_perm_change_info" }),
            shown,
            Self::field(cx, |sheet| sheet.can_change_info, |sheet, value| sheet.can_change_info = value),
            enabled,
        ));
        let shown = self.manage_chat_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-manage-chat",
            tr(if is_channel { "perm_manage_channel" } else { "perm_manage_group" }),
            shown,
            Self::field(cx, |sheet| sheet.can_manage_chat, |sheet, value| sheet.can_manage_chat = value),
            enabled,
        ));
        let shown = self.manage_reactions_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-manage-reactions",
            tr("perm_manage_reactions"),
            shown,
            Self::field(cx, |sheet| sheet.can_manage_reactions, |sheet, value| sheet.can_manage_reactions = value),
            enabled,
        ));
        let shown = self.delete_messages_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-delete-messages",
            tr("chat_admin_perm_delete_messages"),
            shown,
            Self::field(cx, |sheet| sheet.can_delete_messages, |sheet, value| sheet.can_delete_messages = value),
            enabled,
        ));
        let shown = self.ban_members_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-ban-members",
            tr("chat_admin_perm_ban_users"),
            shown,
            Self::field(cx, |sheet| sheet.can_ban_members, |sheet, value| sheet.can_ban_members = value),
            enabled,
        ));
        let shown = self.invite_users_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-invite-users",
            tr(if is_channel { "chat_admin_perm_add_subscribers" } else { "chat_admin_perm_add_members" }),
            shown,
            Self::field(cx, |sheet| sheet.can_invite_users, |sheet, value| sheet.can_invite_users = value),
            enabled,
        ));
        let shown = self.pin_messages_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-pin-messages",
            tr("chat_admin_perm_pin"),
            shown,
            Self::field(cx, |sheet| sheet.can_pin_messages, |sheet, value| sheet.can_pin_messages = value),
            enabled,
        ));
        let shown = self.promote_members_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-promote-members",
            tr("chat_admin_perm_promote"),
            shown,
            Self::field(cx, |sheet| sheet.can_promote_members, |sheet, value| sheet.can_promote_members = value),
            enabled,
        ));
        let shown = self.manage_stories_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-manage-stories",
            tr("chat_admin_perm_stories"),
            shown,
            Self::field(cx, |sheet| sheet.can_manage_stories, |sheet, value| sheet.can_manage_stories = value),
            enabled,
        ));
        let shown = self.manage_video_chats_right(cx);
        rows.push(Self::toggle_row(
            "member-rights-manage-video-chats",
            tr("chat_admin_perm_video_chats"),
            shown,
            Self::field(cx, |sheet| sheet.can_manage_video_chats, |sheet, value| sheet.can_manage_video_chats = value),
            enabled,
        ));
        if !is_channel {
            rows.push(Self::toggle_row(
            "member-rights-anonymous",
            tr("chat_admin_perm_anonymous"),
            self.is_anonymous,
            Self::field(cx, |sheet| sheet.is_anonymous, |sheet, value| sheet.is_anonymous = value),
            enabled,
        ));
        }

        let count = rows.len();
        let rows = rows.into_iter().enumerate().flat_map(|(index, row)| {
            let separator = (index + 1 < count).then(|| divider(theme).into_any_element());
            std::iter::once(row).chain(separator)
        });
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(div().px(px(16.0)).child(MainText::new(tr("chat_member_admin_caps")).px(12.0).color(theme.text.secondary)))
            .child(
                div().w_full().px(px(12.0)).child(
                    vstack(0.0).w_full().bg(theme.bg._200).rounded(px(CARD_RADIUS)).overflow_hidden().children(rows),
                ),
            )
            .when(locked, |card| {
                card.child(
                    div().px(px(16.0)).pt(px(2.0)).child(
                        MainText::new(tr("chat_member_admin_rights_owner_only")).px(13.0).color(theme.text.secondary),
                    ),
                )
            })
    }

    fn title_card(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let rule = rank_rule();
        let write_title = this.clone();
        let input = InputUi::new("member-rights-title")
            .values(self.title.clone())
            .error_message(rule.error_text(self.title_text()))
            .name(key::TITLE)
            .placeholder(tr("chat_member_custom_title_placeholder"))
            .rule(rule)
            .set_value(move |_, value, _, cx| {
                let value = value.to_owned();
                write_title.update(cx, |sheet, cx| {
                    sheet.title.insert(key::TITLE.to_owned(), value);
                    cx.notify();
                });
            });

        let clear = this.clone();
        let set_color = this.clone();
        let well = Binding::entity(
            this,
            |sheet: &Self| sheet.badge_color,
            |sheet: &mut Self, value, _| {
                sheet.badge_color = value;
                sheet.color_touched = true;
            },
        );
        let color_controls = if self.has_badge_color {
            hstack(None::<f32>)
                .child(ColorPickerWell::new("member-rights-badge-color", well))
                .child(
                    plain("member-rights-badge-color-clear")
                        .on_press(move |_, cx| {
                            clear.update(cx, |sheet, cx| {
                                sheet.has_badge_color = false;
                                sheet.color_touched = true;
                                cx.notify();
                            });
                        })
                        .children(IconUi::sized("xmark.circle.fill", px(17.0), theme.text.secondary.opacity(0.6), true)),
                )
                .into_any_element()
        } else {
            plain("member-rights-badge-color-set")
                .on_press(move |_, cx| {
                    set_color.update(cx, |sheet, cx| {
                        sheet.has_badge_color = true;
                        sheet.color_touched = true;
                        cx.notify();
                    });
                })
                .child(MainText::new(tr("roles_color_set")).px(14.0).color(theme.primary._100))
                .into_any_element()
        };

        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                div().px(px(16.0)).child(MainText::new(tr("chat_member_custom_title")).px(12.0).color(theme.text.secondary)),
            )
            .child(div().w_full().px(px(12.0)).child(input.guide_anchor(RiseGuideAnchor::MemberRightsTitle)))
            .child(
                div().w_full().px(px(12.0)).child(
                    hstack(None::<f32>)
                        .w_full()
                        .px(px(16.0))
                        .py(px(12.0))
                        .bg(theme.bg._200)
                        .rounded(px(CARD_RADIUS))
                        .child(MainText::new(tr("chat_member_custom_color")).px(15.0).color(theme.text.primary))
                        .child(spacer())
                        .child(color_controls),
                ),
            )
    }

    fn badge_color_hex(&self) -> String {
        if !self.has_badge_color {
            return String::new();
        }
        let rgba = self.badge_color.to_rgb();
        let channel = |value: f32| (value * 255.0) as u8;
        format!("#{:02X}{:02X}{:02X}", channel(rgba.r), channel(rgba.g), channel(rgba.b))
    }

    fn remove_button(&self, theme: &AppTheme, is_channel: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        div().w_full().px(px(12.0)).child(
            plain("member-rights-remove")
                .w_full()
                .min_h(px(50.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg._200)
                .rounded_full()
                .on_press(move |_, cx| {
                    this.update(cx, |sheet, cx| {
                        let chat_id = sheet.chat_id.clone();
                        let participant_id = sheet.member.user_id.clone();
                        ChatInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.remove_member(&chat_id, &participant_id, cx));
                        sheet.on_saved(cx);
                        sheet.dismiss(cx);
                    });
                })
                .child(
                    MainText::new(tr(if is_channel { "chat_member_remove_channel" } else { "chat_member_remove" }))
                        .px(15.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.semantic.error_100),
                ),
        )
    }

    fn field(cx: &Context<Self>, get: fn(&Self) -> bool, set: fn(&mut Self, bool)) -> Binding<bool> {
        Binding::entity(cx.entity(), move |sheet: &Self| get(sheet), move |sheet: &mut Self, value, _| set(sheet, value))
    }

    /// a disabled row shows the shown value, a right capped by chat-service and not the stored one
    fn toggle_row(
        id: impl Into<SharedString>,
        label: String,
        shown: bool,
        value: Binding<bool>,
        enabled: bool,
    ) -> AnyElement {
        let id: SharedString = id.into();
        if !enabled {
            return div()
                .w_full()
                .opacity(DISABLED_OPACITY)
                .child(SwitchRowUi::new(id, label, Binding::constant(shown)))
                .into_any_element();
        }
        SwitchRowUi::new(id, label, value).into_any_element()
    }

    fn save_bar(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.saving || self.role_writes_in_flight > 0;
        let content_color = theme.content_on(theme.primary._100);
        let this = cx.entity();
        let button = plain("member-rights-save")
            .w_full()
            .min_h(px(48.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.primary._100)
            .rounded_full()
            .disabled(busy)
            .on_press(move |_, cx| this.update(cx, |sheet, cx| sheet.save(cx)))
            .when(busy, |button| button.child(progress_view().tint(content_color)))
            .when(!busy, |button| {
                button.child(MainText::new(tr("chat_member_save")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(content_color))
            });
        div()
            .flex_none()
            .w_full()
            .px(px(12.0))
            .py(px(10.0))
            .bg(theme.bg._100)
            .child(button.guide_anchor(RiseGuideAnchor::MemberRightsSave))
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.has_editable_fields(cx) || self.saving || self.role_writes_in_flight != 0 {
            return;
        }
        self.saving = true;
        cx.notify();
        if self.restrictions_only {
            let completion = Self::after(cx, |sheet, ok, cx| {
                if ok {
                    sheet.finish_saving_member(cx);
                } else {
                    sheet.save_refused(cx);
                }
            });
            self.save_restrictions_if_needed(completion, cx);
            return;
        }
        let completion = Self::after(cx, |sheet, permissions_ok, cx| {
            if !permissions_ok {
                sheet.save_refused(cx);
                return;
            }
            sheet.save_member_identity(cx);
        });
        self.save_member_permissions_if_needed(completion, cx);
    }

    /// the reason is already at the bottom
    /// a refusal usually means the viewer rights changed
    /// so they are read again and the sheet locks or closes
    fn save_refused(&mut self, cx: &mut Context<Self>) {
        self.saving = false;
        cx.notify();
        if self.is_sandbox(cx) {
            return;
        }
        let chat_id = self.chat_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_effective_permissions(&chat_id, None, true, cx));
    }

    fn save_member_permissions_if_needed(&mut self, completion: ChatInteractionsBoolResult, cx: &mut Context<Self>) {
        if !self.perm_touched
            || !self.can_edit_overrides(cx)
            || !self.overrides_apply(cx)
            || self.custom_roles(cx).is_empty()
        {
            completion(true, cx);
            return;
        }
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let (touched, allow, deny) = (self.perm_touched_bits, self.perm_allow, self.perm_deny);
        let this = cx.weak_entity();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.save_member_permissions(
                &chat_id,
                &participant_id,
                touched,
                allow,
                deny,
                Box::new(move |merged_allow: i64, merged_deny: i64, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |sheet, cx| {
                            sheet.perm_allow = merged_allow;
                            sheet.perm_deny = merged_deny;
                            cx.notify();
                        });
                    });
                }),
                completion,
                cx,
            );
        });
    }

    fn save_member_identity(&mut self, cx: &mut Context<Self>) {
        let role = if self.member.is_owner() || !self.can_change_admin(cx) {
            self.member.role.clone()
        } else if self.is_admin {
            ROLE_ADMIN.to_owned()
        } else {
            ROLE_MEMBER.to_owned()
        };
        let current_rank = trimming_whitespaces_and_newlines(&self.member.custom_title).to_owned();
        let can_edit_identity = self.can_edit_identity(cx);
        let rank = if can_edit_identity {
            trimming_whitespaces_and_newlines(self.title_text()).to_owned()
        } else {
            current_rank.clone()
        };
        let sends_color = can_edit_identity && self.color_touched;
        let same_role = role_matches(&role, &self.member.role);
        if same_role && rank == current_rank && !sends_color && self.is_anonymous == self.member.is_anonymous {
            self.save_member_rules_if_needed(cx);
            return;
        }
        let granted_rights = if same_role { HashMap::new() } else { self.granted_rights(self.is_admin, cx) };
        tracing::warn!(
            target: "riseonly::X1766",
            member = %self.member.user_id,
            role = %role,
            same_role,
            viewer_mask = ?self.viewer_mask(cx),
            granted = ?granted_rights,
            "edit member sends role and optimistic rights"
        );
        let custom_color = sends_color.then(|| self.badge_color_hex());
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let is_anonymous = self.is_anonymous;
        let on_result = Self::after(cx, |sheet, ok, cx| {
            if !ok {
                sheet.save_refused(cx);
                return;
            }
            sheet.save_member_rules_if_needed(cx);
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.edit_member(
                &chat_id,
                &participant_id,
                Some(role.as_str()),
                Some(rank.as_str()),
                custom_color.as_deref(),
                Some(is_anonymous),
                granted_rights,
                Some(on_result),
                cx,
            );
        });
    }

    fn save_member_rules_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_admin || self.member.is_owner() || !self.can_set_admin_rights(cx) {
            self.finish_saving_member(cx);
            return;
        }
        let rules = [
            (key::CAN_CHANGE_INFO, self.can_change_info),
            (key::CAN_DELETE_MESSAGES, self.can_delete_messages),
            (key::CAN_BAN_MEMBERS, self.can_ban_members),
            (key::CAN_RESTRICT_MEMBERS, self.can_ban_members),
            (key::CAN_INVITE_USERS, self.can_invite_users),
            (key::CAN_PIN_MESSAGES, self.can_pin_messages),
            (key::CAN_PROMOTE_MEMBERS, self.can_promote_members),
            (key::CAN_MANAGE_STORIES, self.can_manage_stories),
            (key::CAN_MANAGE_VIDEO_CHATS, self.can_manage_video_chats),
            (key::CAN_MANAGE_CHAT, self.can_manage_chat),
            (key::CAN_MANAGE_REACTIONS, self.can_manage_reactions),
        ];
        let rules: HashMap<String, bool> = rules.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let on_result = Self::after(cx, |sheet, ok, cx| {
            if ok {
                sheet.finish_saving_member(cx);
            } else {
                sheet.save_refused(cx);
            }
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.edit_member_rules(&chat_id, &participant_id, rules, Some(on_result), cx);
        });
    }

    fn finish_saving_member(&mut self, cx: &mut Context<Self>) {
        self.saving = false;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::MemberRightsSaved);
        self.on_saved(cx);
        self.dismiss(cx);
    }

    fn restrictions_card(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.saving || self.role_writes_in_flight > 0;
        let this = cx.entity();
        let body: Vec<AnyElement> = if self.restriction_permissions.is_none() {
            if self.restriction_load_failed {
                vec![
                    plain("member-rights-restrictions-retry")
                        .p(px(16.0))
                        .on_press(move |_, cx| this.update(cx, |sheet, cx| sheet.load_restriction_permissions(cx)))
                        .child(MainText::new(tr("error_fetching_data")).px(14.0).color(theme.text.secondary))
                        .into_any_element(),
                ]
            } else {
                vec![div().p(px(16.0)).child(progress_view()).into_any_element()]
            }
        } else {
            let mut rows: Vec<AnyElement> = Vec::new();
            for (index, (keys, bit)) in RESTRICTION_RULES.iter().copied().enumerate() {
                let label = ChatPerm::BITS.iter().find(|entry| entry.mask == bit).map_or("ctx_restrict", |entry| entry.i18n_key);
                let id = SharedString::from(format!("member-rights-restriction-{bit}"));
                let shown = self.restriction_value(keys, bit);
                if disabled {
                    rows.push(
                        div()
                            .w_full()
                            .opacity(DISABLED_OPACITY)
                            .child(SwitchRowUi::new(id, tr(label), Binding::constant(shown)))
                            .into_any_element(),
                    );
                } else {
                    let value = Binding::entity(
                        this.clone(),
                        move |sheet: &Self| sheet.restriction_value(keys, bit),
                        move |sheet: &mut Self, allowed, cx| sheet.set_restriction(keys, bit, allowed, cx),
                    );
                    rows.push(SwitchRowUi::new(id, tr(label), value).into_any_element());
                }
                if index + 1 < RESTRICTION_RULES.len() {
                    rows.push(divider(theme).into_any_element());
                }
            }
            rows
        };
        div().w_full().px(px(12.0)).child(
            vstack(0.0).w_full().bg(theme.bg._200).rounded(px(CARD_RADIUS)).overflow_hidden().children(body),
        )
    }

    fn restriction_value(&self, keys: &[&str], bit: i64) -> bool {
        keys.first()
            .and_then(|first| self.restriction_changes.get(*first))
            .copied()
            .unwrap_or_else(|| ChatPerm::has(self.restriction_permissions.unwrap_or(0), bit))
    }

    fn set_restriction(&mut self, keys: &[&str], bit: i64, allowed: bool, cx: &mut Context<Self>) {
        if self.saving || !self.can_edit_restrictions(cx) {
            return;
        }
        let original = ChatPerm::has(self.restriction_permissions.unwrap_or(0), bit);
        for field in keys {
            if allowed == original {
                self.restriction_changes.remove(*field);
            } else {
                self.restriction_changes.insert((*field).to_owned(), allowed);
            }
        }
    }

    fn load_restriction_permissions(&mut self, cx: &mut Context<Self>) {
        if !self.can_manage_restrictions(cx) {
            return;
        }
        let Some(domain_id) = Self::active_domain_id(cx) else {
            self.restriction_load_failed = true;
            cx.notify();
            return;
        };
        self.restriction_generation = self.restriction_generation.wrapping_add(1);
        let generation = self.restriction_generation;
        self.restriction_load_failed = false;
        cx.notify();
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let this = cx.weak_entity();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_member_restriction_permissions(
                &chat_id,
                &participant_id,
                Box::new(move |permissions: Option<i64>, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = this.update(cx, |sheet, cx| {
                            if sheet.restriction_generation != generation || Self::active_domain_id(cx) != Some(domain_id) {
                                return;
                            }
                            match permissions.filter(|permissions| ChatPerm::has(*permissions, ChatPerm::VIEW_CHANNELS)) {
                                Some(permissions) => {
                                    sheet.restriction_permissions = Some(permissions);
                                    sheet.restriction_domain_id = Some(domain_id);
                                }
                                None => sheet.restriction_load_failed = true,
                            }
                            cx.notify();
                        });
                    });
                }),
                cx,
            );
        });
    }

    fn save_restrictions_if_needed(&mut self, completion: ChatInteractionsBoolResult, cx: &mut Context<Self>) {
        if self.restriction_changes.is_empty() {
            completion(true, cx);
            return;
        }
        if !self.can_edit_restrictions(cx) {
            completion(false, cx);
            return;
        }
        let Some(domain_id) = Self::active_domain_id(cx) else {
            completion(false, cx);
            return;
        };
        let generation = self.restriction_generation;
        let chat_id = self.chat_id.clone();
        let participant_id = self.member.user_id.clone();
        let rules = self.restriction_changes.clone();
        let on_result = Self::after(cx, move |sheet, ok, cx| {
            if sheet.restriction_generation != generation || Self::active_domain_id(cx) != Some(domain_id) {
                return;
            }
            if ok {
                let chat_id = sheet.chat_id.clone();
                let member = sheet.member.clone();
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.refresh_members(&chat_id, None, cx);
                    store.notify_member_restricted(&member, cx);
                });
            }
            completion(ok, cx);
        });
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.edit_member_rules(&chat_id, &participant_id, rules, Some(on_result), cx);
        });
    }

    fn body_cards(&self, theme: &AppTheme, is_channel: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut cards: Vec<AnyElement> = Vec::new();
        let has_roles = !self.custom_roles(cx).is_empty();
        if self.restrictions_only {
            if self.can_manage_restrictions(cx) {
                cards.push(self.restrictions_card(theme, cx).into_any_element());
            } else {
                cards.push(
                    div()
                        .w_full()
                        .px(px(24.0))
                        .child(MainText::new(tr("error_fetching_data")).px(14.0).color(theme.text.secondary))
                        .into_any_element(),
                );
            }
            return cards;
        }
        if self.member.is_owner() {
            let note = if is_channel { "chat_member_owner_note_channel" } else { "chat_member_owner_note" };
            cards.push(
                div()
                    .w_full()
                    .px(px(24.0))
                    .pt(px(12.0))
                    .child(MainText::new(tr(note)).w_full().px(14.0).tac(TextAlignT::Center).color(theme.text.secondary))
                    .into_any_element(),
            );
            if has_roles {
                cards.push(self.roles_card(theme, cx).into_any_element());
            }
            if self.can_edit_identity(cx) {
                cards.push(self.title_card(theme, cx).into_any_element());
            }
            return cards;
        }
        if self.can_change_admin(cx) || self.member.is_admin() {
            cards.push(self.admin_toggle_card(theme, cx).into_any_element());
        }
        if self.is_admin {
            cards.push(self.admin_perms_card(theme, is_channel, cx).into_any_element());
        }
        if has_roles {
            cards.push(self.roles_card(theme, cx).into_any_element());
            if self.can_edit_overrides(cx) && self.overrides_apply(cx) {
                cards.push(self.permission_overrides_card(theme, cx).into_any_element());
            }
        }
        if self.can_edit_identity(cx) {
            cards.push(self.title_card(theme, cx).into_any_element());
        }
        if self.outranks_member(cx)
            && (self.viewer_holds(ChatPerm::BAN_MEMBERS, cx) || self.viewer_holds(ChatPerm::KICK_MEMBERS, cx))
        {
            cards.push(self.remove_button(theme, is_channel, cx).into_any_element());
        }
        cards
    }
}

impl Render for MemberRightsSheetState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_channel = self.is_channel(cx);

        on_change(
            "member-rights-title-filled",
            self.title_text().to_owned(),
            true,
            window,
            cx,
            |_, value, _, cx| {
                GuideFacts::set(
                    RiseGuideSignal::MemberTitleFilled,
                    !trimming_whitespaces_and_newlines(value).is_empty(),
                    cx,
                );
            },
        );
        let can_change_admin = self.can_change_admin(cx);
        let admin_reset = cx.weak_entity();
        on_change("member-rights-can-change-admin", can_change_admin, false, window, cx, move |_, allowed, _, cx| {
            if *allowed {
                return;
            }
            let _ = admin_reset.update(cx, |sheet, cx| {
                sheet.is_admin = sheet.member.is_admin() || sheet.member.is_owner();
                cx.notify();
            });
        });
        let viewer_lost_management = self.viewer_lost_management(cx);
        let lost_dismiss = cx.weak_entity();
        on_change("member-rights-viewer-lost", viewer_lost_management, false, window, cx, move |_, lost, _, cx| {
            if !*lost {
                return;
            }
            let _ = lost_dismiss.update(cx, |sheet, cx| sheet.dismiss(cx));
            if let Some(text) = CanonErrors::text(Some(ErrorCode::ChatPermissionDenied.wire()), &HashMap::new()) {
                let options = NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                };
                Notifier::send(text, options, cx);
            }
        });

        let header = self.header_card(&theme, cx);
        let cards = self.body_cards(&theme, is_channel, cx);
        let has_editable_fields = self.has_editable_fields(cx);
        let close = cx.entity();

        let notifier = self.notifier_host.clone();
        vstack(0.0)
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .h(px(SHEET_HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg._100)
                    .child(
                        MainText::new(tr(self.sheet_title_key(is_channel)))
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(
                        div().absolute().left(px(16.0)).top_0().bottom_0().flex().items_center().child(
                            plain("member-rights-close")
                                .on_press(move |_, cx| close.update(cx, |sheet, cx| sheet.dismiss(cx)))
                                .children(IconUi::sized("xmark", px(17.0), theme.text.primary, false)),
                        ),
                    ),
            )
            .child(
                scroll_view("member-rights-scroll").flex_1().min_h_0().w_full().child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(header)
                        .children(cards)
                        .child(div().flex_none().h(px(20.0))),
                ),
            )
            .when(has_editable_fields, |sheet| sheet.child(self.save_bar(&theme, cx)))
            .child(div().absolute().inset_0().child(notifier))
    }
}

/// member rights sheet on the kit sheet host
/// a fresh target mounts a fresh sheet so old state is gone
/// the root draws it so its frames dont dirty the page
pub struct MemberRightsSheetPresentation {
    presented: Option<MemberRightsSheetPresented>,
}

#[derive(Clone)]
struct MemberRightsSheetPresented {
    chat_id: String,
    target: MemberRightsTarget,
    on_saved: Rc<dyn Fn(&mut App)>,
}

impl MemberRightsSheetPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { presented: None }
    }

    pub fn present(
        &mut self,
        chat_id: String,
        target: MemberRightsTarget,
        on_saved: impl Fn(&mut App) + 'static,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.presented = Some(MemberRightsSheetPresented { chat_id, target, on_saved: Rc::new(on_saved) });
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.presented = None;
        cx.notify();
    }
}

impl Render for MemberRightsSheetPresentation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let reader = this.clone();
        div().absolute().inset_0().child(
            NativeBottomSheetPresentationHost::new(
                ("member-rights-sheet", this.entity_id()),
                Binding::entity(
                    this,
                    |presentation: &Self| presentation.presented.is_some(),
                    |presentation, shown, _| {
                        if !shown {
                            presentation.presented = None;
                        }
                    },
                ),
                move |_, cx| match reader.read(cx).presented.clone() {
                    Some(presented) => {
                        let on_saved = presented.on_saved;
                        MemberRightsSheet::new(
                            presented.chat_id,
                            presented.target.member,
                            presented.target.restrictions_only,
                            move |cx: &mut App| on_saved(cx),
                        )
                        .into_any_element()
                    }
                    None => Empty.into_any_element(),
                },
            )
            .presentation_detents([PresentationDetent::Large])
            .presentation_drag_indicator(true)
            .fills_window(true),
        )
    }
}
