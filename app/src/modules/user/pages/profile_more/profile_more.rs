use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::user::components::profile_top::profile_about_sheet::ProfileAboutSheet;
use crate::modules::user::components::profile_top::profile_featured_tracks_sheet::ProfileFeaturedTracksSheet;
use crate::modules::user::components::social::social_media_links_sheet::SocialMediaLinksSheet;
use crate::modules::user::components::user_relationship_list::UserRelationshipList;
use crate::modules::user::pages::profile::profile::ProfileRouteTarget;
use crate::modules::user::shared::profile_missing_tag_state::ProfileMissingTagState;
use crate::modules::user::shared::social_platform::SocialLink;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_types::{ProfileModel, UserListKind};
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const PICKER_INSET: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileMoreTab {
    About,
    Links,
    Music,
    Subscribers,
    Subscriptions,
    Friends,
}

impl ProfileMoreTab {
    /// tab title, the people tabs carry the count the profile header shows
    fn title(self, profile: &ProfileModel) -> SharedString {
        let key = match self {
            Self::About => "profile_about_title",
            Self::Links => "chat_profile_tab_link",
            Self::Music => "chat_profile_tab_music",
            Self::Subscribers => "user_subscribers_text",
            Self::Subscriptions => "user_subs_text",
            Self::Friends => "user_friends_text",
        };
        let count = match self {
            Self::Subscribers => Some(profile.subscribers_count),
            Self::Subscriptions if !profile.subs_count_hidden => Some(profile.subs_count),
            Self::Friends => Some(profile.friends_count),
            _ => None,
        };
        match count.filter(|_| !profile.id.is_empty()) {
            Some(count) => SharedString::from(format!("{} {}", tr(key), count.max(0))),
            None => SharedString::from(tr(key)),
        }
    }

    /// tabs in order about and music and subscribers and subscriptions and friends
    /// links only if any exist and music hides when empty
    pub fn available(has_links: bool, music_empty: bool) -> Vec<Self> {
        let mut tabs = vec![Self::About];
        if !music_empty {
            tabs.push(Self::Music);
        }
        tabs.extend([Self::Subscribers, Self::Subscriptions, Self::Friends]);
        if has_links {
            tabs.push(Self::Links);
        }
        tabs
    }

    fn relationship_kind(self) -> Option<(UserListKind, &'static str)> {
        match self {
            Self::Subscribers => Some((UserListKind::Subscribers, "user_subscribers_text")),
            Self::Subscriptions => Some((UserListKind::Subscriptions, "user_subs_text")),
            Self::Friends => Some((UserListKind::Friends, "user_friends_text")),
            Self::About | Self::Links | Self::Music => None,
        }
    }
}

struct ProfileMoreState {
    selected: ProfileMoreTab,
    music_empty_for: Option<String>,
    featured_track_id: Option<String>,
    live_drag_state: Entity<AnimatedTabsPagerLiveState>,
    _observations: Vec<Subscription>,
}

impl ProfileMoreState {
    fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let services = UserServicesStore::shared(cx);
        let actions = UserActionsStore::shared(cx);
        Self {
            selected: ProfileMoreTab::About,
            music_empty_for: None,
            featured_track_id: None,
            live_drag_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            _observations: vec![
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct ProfileMore {
    id: ElementId,
    user_id: Option<String>,
    tag: Option<String>,
}

impl ProfileMore {
    pub fn new(id: impl Into<ElementId>, user_id: Option<String>, tag: Option<String>) -> Self {
        Self { id: id.into(), user_id, tag }
    }

    /// same profile the center page draws,
    /// the signed-in one or the cached foreign answer. treat_as_foreign decides achievements
    /// loading and music editing
    fn resolve(&self, cx: &mut App) -> (ProfileModel, bool) {
        let own = AuthServicesStore::shared(cx).read(cx).profile().cloned().unwrap_or_default();
        let target = ProfileRouteTarget::resolve(false, self.user_id.as_deref(), self.tag.as_deref());
        let foreign = match target {
            ProfileRouteTarget::Own | ProfileRouteTarget::Invalid => None,
            ProfileRouteTarget::ForeignUserId(user_id) if user_id == own.id => None,
            ProfileRouteTarget::ForeignTag(tag) if tag == own.tag => None,
            ProfileRouteTarget::ForeignUserId(user_id) => {
                let is_deleted = ProfileMissingTagState::shared(cx).read(cx).is_deleted_user(&user_id);
                Some(
                    UserServicesStore::shared(cx)
                        .update(cx, |store, cx| store.cached_foreign_profile_user_id(&user_id, None, cx))
                        .and_then(|payload| payload.as_profile_model())
                        .unwrap_or(ProfileModel { id: user_id, is_deleted, ..ProfileModel::default() }),
                )
            }
            ProfileRouteTarget::ForeignTag(tag) => Some(
                UserServicesStore::shared(cx)
                    .update(cx, |store, cx| store.cached_foreign_profile_tag(&tag, cx))
                    .and_then(|payload| payload.as_profile_model())
                    .unwrap_or(ProfileModel { tag, ..ProfileModel::default() }),
            ),
        };
        match foreign {
            Some(model) => (UserServicesStore::shared(cx).read(cx).apply_follow_overlay(model), true),
            None => (own, false),
        }
    }

    fn page(
        &self,
        tab: ProfileMoreTab,
        profile: &ProfileModel,
        treat_as_foreign: bool,
        links: &[SocialLink],
        state: &Entity<ProfileMoreState>,
    ) -> AnyElement {
        match tab {
            ProfileMoreTab::About if profile.is_deleted => {
                ChatProfileEmptyState::new("person.crop.circle.badge.xmark", tr("chat_deleted_account")).into_any_element()
            }
            ProfileMoreTab::About if treat_as_foreign && profile.id.is_empty() => div().into_any_element(),
            ProfileMoreTab::About => ProfileAboutSheet::new(self.child_id("about"), Binding::constant(true), profile.clone())
                .achievements_user_id(treat_as_foreign.then(|| profile.id.clone()))
                .into_any_element(),
            ProfileMoreTab::Links => SocialMediaLinksSheet::new(self.child_id("links"), links.to_vec()).into_any_element(),
            ProfileMoreTab::Music => {
                let empty_state = state.clone();
                let empty_id = profile.id.clone();
                ProfileFeaturedTracksSheet::new(
                    self.child_id("music"),
                    profile.id.clone(),
                    Binding::entity(
                        state.clone(),
                        |state: &ProfileMoreState| state.featured_track_id.clone(),
                        |state, value, _| state.featured_track_id = value,
                    ),
                )
                .is_owner(!treat_as_foreign)
                .on_empty(move |_, cx| {
                    let empty_id = empty_id.clone();
                    empty_state.update(cx, |state, cx| {
                        state.music_empty_for = Some(empty_id);
                        state.selected = ProfileMoreTab::About;
                        cx.notify();
                    });
                })
                .into_any_element()
            }
            ProfileMoreTab::Subscribers | ProfileMoreTab::Subscriptions | ProfileMoreTab::Friends => {
                let user_id = Some(profile.id.clone()).filter(|id| !id.is_empty());
                let (kind, title_key) = tab.relationship_kind().unwrap_or((UserListKind::Friends, "user_friends_text"));
                UserRelationshipList::new(user_id, kind, tr(title_key)).into_any_element()
            }
        }
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::from(name.to_owned()))
    }
}

impl RenderOnce for ProfileMore {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| ProfileMoreState::new(cx));
        let (profile, treat_as_foreign) = self.resolve(cx);
        let links = SocialLink::from(&profile.social_links);
        let music_empty = !profile.id.is_empty() && state.read(cx).music_empty_for.as_deref() == Some(profile.id.as_str());
        let tabs = ProfileMoreTab::available(!links.is_empty(), music_empty || profile.id.is_empty() || profile.is_deleted);
        let selected = state.read(cx).selected;
        let selected = if tabs.contains(&selected) { selected } else { ProfileMoreTab::About };
        let selected_index = tabs.iter().position(|tab| *tab == selected).unwrap_or(0);
        let live_drag_state = state.read(cx).live_drag_state.clone();

        let select_state = state.clone();
        let select_tabs = tabs.clone();
        let select: Rc<dyn Fn(i64, &mut App)> = Rc::new(move |index: i64, cx: &mut App| {
            let Some(tab) = usize::try_from(index).ok().and_then(|index| select_tabs.get(index).copied()) else {
                return;
            };
            select_state.update(cx, |state, cx| {
                if state.selected != tab {
                    state.selected = tab;
                    cx.notify();
                }
            });
        });
        let binding_select = select.clone();
        let tap_select = select;
        let active_state = state.clone();
        let active_tabs = tabs.clone();
        let active_tab = Binding::new(
            move |cx: &App| {
                let selected = active_state.read(cx).selected;
                active_tabs.iter().position(|tab| *tab == selected).unwrap_or(0) as i64
            },
            move |index, _, cx| binding_select(index, cx),
        );
        let header_tabs =
            tabs.iter().enumerate().map(|(index, tab)| TabHeaderConfig::new(index as i64, tab.title(&profile))).collect();
        let progress_state = live_drag_state.clone();
        let base_state = live_drag_state.clone();
        let release_state = live_drag_state.clone();
        let tap_state = live_drag_state.clone();
        let picker = AnimatedTabsHeader::new(
            self.child_id("tabs"),
            header_tabs,
            active_tab.clone(),
            Rc::new(move |cx: &App| progress_state.read(cx).drag_progress()),
            0.0,
            Rc::new(move |index: i64, _: &mut Window, cx: &mut App| tap_select(index, cx)),
        )
        .drag_base_index(Rc::new(move |cx: &App| base_state.read(cx).drag_base_index()))
        .drag_release_animation(Rc::new(move |cx: &App| release_state.read(cx).drag_release_animation()))
        .suppresses_tap(Rc::new(move |cx: &App| tap_state.read(cx).suppresses_tab_taps()))
        .force_scrollable(true)
        .arrows_step_selection(true);

        let pages: Vec<AnyElement> = tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let mounted = index.abs_diff(selected_index) <= 1;
                let content = if mounted {
                    self.page(*tab, &profile, treat_as_foreign, &links, &state)
                } else {
                    div().into_any_element()
                };
                div().size_full().child(content).into_any_element()
            })
            .collect();
        let suppress_state = live_drag_state.clone();
        let pager = AnimatedTabsPager::new(
            self.child_id("pager"),
            tabs.len() as i64,
            active_tab,
            Binding::constant(0.0),
            Binding::constant(0.0),
        )
        .live_drag_state(Some(live_drag_state))
        .yields_first_tab_back_swipe_to_navigation(true)
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed, _: &mut Window, cx: &mut App| {
            suppress_state.update(cx, |live, cx| {
                live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })))
        .children(pages);

        vstack(0.0)
            .size_full()
            .bg(background)
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .px(px(PICKER_INSET))
                    .pt(px(PICKER_INSET))
                    .pb(px(PICKER_INSET / 2.0))
                    .child(div().relative().w_full().h(AnimatedTabsMetrics::BAR_HEIGHT).child(picker)),
            )
            .child(div().w_full().flex_1().min_h_0().child(pager))
    }
}
