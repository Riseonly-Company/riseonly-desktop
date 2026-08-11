use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, IntoElement, ListAlignment, Render,
    Subscription, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ProfileMetrics};
use rise_ui::{EdgeState, ListUi, PaginationEdge, TabsEvent, TabsUiState};
use rise_widgets::{
    ModalAction, ModalUi, ModalWidth, PageHeaderUi, PagePresentation, PageStateUi, PageStatus,
    ScreenShellUi,
};

use crate::modules::post::components::post::{PostAction, PostCard, PostHandler, PostSurface};
use crate::modules::post::engine::rise_post_rpc::PostScope;
use crate::modules::post::stores::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::PostServicesStore;
use crate::modules::user::components::profile_content::{ProfileContent, ProfileContentTab};
use crate::modules::user::components::profile_top::{
    ProfileTop, ProfileTopAction, ProfileTopHandler,
};
use crate::modules::user::engine::rise_user_engine_models::{
    GoalItem, PlanItem, ProfileKey, ProfileModel,
};
use crate::modules::user::stores::user::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::UserServicesStore;

/// Which profile a route asked for. A tag wins over an id, and an own-profile
/// route stays addressable before the account id is known.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProfileRouteTarget {
    Own,
    ForeignTag(String),
    ForeignUserId(String),
}

impl ProfileRouteTarget {
    pub fn resolve(user_id: Option<&str>, tag: Option<&str>) -> Self {
        if let Some(tag) = tag.map(str::trim).filter(|tag| !tag.is_empty()) {
            return Self::ForeignTag(tag.to_lowercase());
        }
        if let Some(user_id) = user_id.map(str::trim).filter(|id| !id.is_empty()) {
            return Self::ForeignUserId(user_id.to_owned());
        }
        Self::Own
    }

    pub fn key(&self) -> ProfileKey {
        match self {
            Self::Own => ProfileKey::Own,
            Self::ForeignTag(tag) => ProfileKey::Tag(tag.clone()),
            Self::ForeignUserId(id) => ProfileKey::Id(id.clone()),
        }
    }
}

pub struct Profile {
    user: UserInteractionsStore,
    user_services: Entity<UserServicesStore>,
    post: PostInteractionsStore,
    post_services: Entity<PostServicesStore>,
    key: ProfileKey,
    tabs: Entity<TabsUiState>,
    lists: Vec<ListUi>,
    lengths: Vec<usize>,
    active: ProfileContentTab,
    feed_tag: Option<String>,
    shows_about: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Profile {
    pub fn new(
        user: UserInteractionsStore,
        post: PostInteractionsStore,
        target: ProfileRouteTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = rise_ui::theme(cx as &App).clone();
        let user_services = user.services().clone();
        let post_services = post.services().clone();
        let key = target.key();

        let tabs = cx.new(|cx| {
            TabsUiState::new("profile.tabs", ProfileContentTab::items(), cx)
                .distribute_when_it_fits(true)
        });

        let subscriptions = vec![
            cx.subscribe(&tabs, |view, _, event: &TabsEvent, cx| {
                let TabsEvent::Selected(index) = event;
                view.select(*index, cx);
            }),
            cx.observe(&user_services, |view, _, cx| {
                view.sync_feed_tag(cx);
                cx.notify();
            }),
            cx.observe(&post_services, |view, _, cx| {
                view.sync_lengths(cx);
                cx.notify();
            }),
        ];

        let lists: Vec<ListUi> = ProfileContentTab::ALL
            .iter()
            .map(|_| ListUi::new(&theme, 0, ListAlignment::Top))
            .collect();

        let entity = cx.entity();
        for (index, list) in lists.iter().enumerate() {
            let tab = ProfileContentTab::ALL[index];
            list.on_edge(
                &entity,
                move |view: &Profile, cx: &App| {
                    let Some(scope) = view.scope_of(tab) else {
                        return EdgeState::default();
                    };
                    let services = view.post_services.read(cx);
                    EdgeState {
                        has_more_top: false,
                        has_more_bottom: services.has_more(&scope),
                        top_in_flight: false,
                        bottom_in_flight: services.is_loading_more(&scope),
                    }
                },
                move |view: &mut Profile, edge, cx| {
                    if edge == PaginationEdge::Bottom
                        && let Some(scope) = view.scope_of(tab)
                    {
                        view.post.load_more(scope, cx);
                    }
                },
            );
        }

        user.open_profile(key.clone());

        let _ = window;

        let mut screen = Self {
            user,
            user_services,
            post,
            post_services,
            key,
            tabs,
            lists,
            lengths: vec![0; ProfileContentTab::ALL.len()],
            active: ProfileContentTab::Publications,
            feed_tag: None,
            shows_about: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        screen.sync_feed_tag(cx);
        screen.sync_lengths(cx);
        screen
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = ProfileContentTab::ALL.get(index).copied() else {
            return;
        };

        self.active = tab;
        if let Some(scope) = self.scope_of(tab) {
            self.post.activate_feed(scope);
        }
        cx.notify();
    }

    fn scope_of(&self, tab: ProfileContentTab) -> Option<PostScope> {
        let tag = self.feed_tag.as_deref()?;
        Some(PostScope::user(tag, tab.feed()?))
    }

    /// A profile's post lists are addressed by tag, which only the payload
    /// carries — so the first list can only be asked for once the profile lands.
    fn sync_feed_tag(&mut self, cx: &mut Context<Self>) {
        let tag = self.user_services.read(cx).feed_tag(&self.key);
        if tag == self.feed_tag {
            return;
        }

        self.feed_tag = tag;
        if let Some(scope) = self.scope_of(self.active) {
            self.post.activate_feed(scope);
        }
    }

    fn sync_lengths(&mut self, cx: &mut Context<Self>) {
        for (index, tab) in ProfileContentTab::ALL.iter().enumerate() {
            let Some(scope) = self.scope_of(*tab) else {
                continue;
            };
            let length = self.post_services.read(cx).items(&scope).len();
            let previous = self.lengths[index];
            if length == previous {
                continue;
            }

            if length > previous {
                self.lists[index].appended(length - previous);
            } else {
                self.lists[index].replaced(length);
            }
            self.lengths[index] = length;
        }
    }

    fn profile(&self, cx: &App) -> ProfileModel {
        self.user_services
            .read(cx)
            .profile(&self.key)
            .cloned()
            .unwrap_or_else(|| ProfileModel::placeholder(String::new(), self.route_tag()))
    }

    fn route_tag(&self) -> String {
        match &self.key {
            ProfileKey::Tag(tag) => tag.clone(),
            _ => String::new(),
        }
    }

    fn top_handler(&self) -> ProfileTopHandler<Self> {
        Rc::new(|view: &mut Self, action, _window, cx| match action {
            ProfileTopAction::ToggleFollow => {
                view.user.toggle_follow(view.key.clone(), cx as &App);
            }
            ProfileTopAction::ShowAbout => {
                view.shows_about = true;
                cx.notify();
            }
            ProfileTopAction::OpenLink(_) => {}
        })
    }

    fn post_handler(&self) -> PostHandler<Self> {
        Rc::new(|view: &mut Self, action, row_id, _window, cx| {
            view.dispatch_post(action, row_id, cx);
        })
    }

    fn dispatch_post(&mut self, action: PostAction, row_id: String, cx: &mut Context<Self>) {
        let Some(scope) = self.scope_of(self.active) else {
            return;
        };
        let Some(post) = self
            .post_services
            .read(cx)
            .items(&scope)
            .iter()
            .find(|item| item.row_id() == row_id)
            .cloned()
        else {
            return;
        };

        match action {
            PostAction::Like | PostAction::DoubleTapLike => self.post.toggle_like(&post),
            PostAction::Favorite => self.post.toggle_favorite(&post),
            PostAction::Repost => self.post.toggle_repost(&post),
            PostAction::Comments => self.post.open_comments(&post, cx),
            PostAction::OpenAuthor => self.post.open_profile(post.author.id.clone(), cx),
            PostAction::ShowLikes | PostAction::OpenOptions(_) | PostAction::OpenHashtag(_) => {}
        }
    }

    fn body(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let services = self.user_services.read(cx);
        let status = services.status(&self.key);
        let has_content = services.has_content(&self.key);

        match status.presentation(has_content) {
            PagePresentation::Loading => ProfileTop::skeleton(theme).into_any_element(),
            PagePresentation::Empty | PagePresentation::Error => PageStateUi::error(
                theme,
                tr("profile_load_error"),
                move |view: &mut Self, _, _| {
                    let key = view.key.clone();
                    view.user.refresh_profile(key);
                },
                cx,
            )
            .into_any_element(),
            PagePresentation::Content => self.page(theme, cx).into_any_element(),
            PagePresentation::ContentWithRefreshError => {
                let pill = PageStateUi::refresh_error(
                    theme,
                    tr("profile_load_error"),
                    move |view: &mut Self, _, _| {
                        let key = view.key.clone();
                        view.user.refresh_profile(key);
                    },
                    cx,
                )
                .into_any_element();

                PageStateUi::over_content(theme, self.page(theme, cx), Some(pill))
                    .into_any_element()
            }
        }
    }

    /// One list carries the header, the tab bar and the rows: the profile is a
    /// single scroll in the reference, and two nested ones would fight over the
    /// wheel and lose the pagination edge.
    fn page(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tab = self.active;
        let index = ProfileContentTab::ALL
            .iter()
            .position(|candidate| *candidate == tab)
            .unwrap_or(0);

        let is_viewer = self.user_services.read(cx).is_viewer(&self.key);
        let follow_in_flight = self.user_services.read(cx).is_follow_in_flight(&self.key);
        let top_handler = self.top_handler();
        let post_handler = self.post_handler();
        let post_services = self.post_services.clone();
        let scope = self.scope_of(tab);
        let tabs = self.tabs.clone();
        let theme = theme.clone();
        let entity = cx.entity();
        let now_ms = now_ms();

        // One clone per render, and the row builder borrows it: the header needs
        // an owned model to outlive this call, and cloning it per row would copy
        // every goal, plan and link on the frame path.
        let mut profile = self.profile(cx as &App);
        let rows = match self.rows_kind(tab, cx as &App) {
            RowsKind::Posts => Rows::Posts,
            RowsKind::Goals => Rows::Goals(std::mem::take(&mut profile.goals)),
            RowsKind::Plans => Rows::Plans(std::mem::take(&mut profile.plans)),
            RowsKind::Empty => Rows::Empty(tab.empty_symbol(), tab.empty_key()),
        };

        div().size_full().child(self.lists[index].element({
            let theme = theme.clone();
            move |row, _window, cx| {
                let metrics = theme.profile;
                let content = div().w_full().flex().justify_center();

                match row {
                    0 => entity.update(cx, |_, cx| {
                        content
                            .child(div().w_full().max_w(metrics.content_width).child(
                                ProfileTop::render(
                                    &theme,
                                    &profile,
                                    is_viewer,
                                    follow_in_flight,
                                    &top_handler,
                                    cx,
                                ),
                            ))
                            .into_any_element()
                    }),
                    1 => content
                        .pt(metrics.tabs_top)
                        .pb(metrics.tabs_bottom)
                        .child(
                            div()
                                .w_full()
                                .max_w(metrics.content_width)
                                .px(metrics.padding_x)
                                .child(tabs.clone()),
                        )
                        .into_any_element(),
                    _ => {
                        let item = row - Self::HEADER_ROWS;
                        let element = match &rows {
                            Rows::Goals(goals) => goals
                                .get(item)
                                .map(|goal| ProfileContent::goal(&theme, goal).into_any_element()),
                            Rows::Plans(plans) => plans.get(item).map(|plan| {
                                ProfileContent::plan(&theme, plan, now_ms).into_any_element()
                            }),
                            Rows::Posts => scope.as_ref().and_then(|scope| {
                                post_services
                                    .read(cx)
                                    .item(scope, item)
                                    .cloned()
                                    .map(|post| {
                                        entity.update(cx, |_, cx| {
                                            PostCard::render(
                                                &theme,
                                                &post,
                                                PostSurface::FeedRow,
                                                now_ms,
                                                false,
                                                &post_handler,
                                                cx,
                                            )
                                            .into_any_element()
                                        })
                                    })
                            }),
                            Rows::Empty(symbol, key) => {
                                Some(PageStateUi::empty(&theme, symbol, tr(key)).into_any_element())
                            }
                        };

                        let Some(element) = element else {
                            return div().into_any_element();
                        };

                        content
                            .px(if matches!(rows, Rows::Posts) {
                                gpui::px(0.0)
                            } else {
                                metrics.padding_x
                            })
                            .pb(metrics.row_gap)
                            .child(div().w_full().max_w(metrics.content_width).child(element))
                            .into_any_element()
                    }
                }
            }
        }))
    }

    const HEADER_ROWS: usize = 2;

    /// What the active tab draws, decided without touching the payload's
    /// vectors: `render` asks this every frame and a clone here would copy every
    /// goal and plan on the frame path.
    fn rows_kind(&self, tab: ProfileContentTab, cx: &App) -> RowsKind {
        let services = self.user_services.read(cx);
        let profile = services.profile(&self.key);

        match tab {
            ProfileContentTab::Goals => {
                match profile.is_some_and(|profile| !profile.goals.is_empty()) {
                    true => RowsKind::Goals,
                    false => RowsKind::Empty,
                }
            }
            ProfileContentTab::Plans => {
                match profile.is_some_and(|profile| !profile.plans.is_empty()) {
                    true => RowsKind::Plans,
                    false => RowsKind::Empty,
                }
            }
            _ => {
                let drained =
                    self.scope_of(tab)
                        .and_then(|scope| {
                            self.post_services.read(cx).page(&scope).map(|page| {
                                page.status == PageStatus::Loaded && page.items.is_empty()
                            })
                        })
                        .unwrap_or(false);

                match drained {
                    true => RowsKind::Empty,
                    false => RowsKind::Posts,
                }
            }
        }
    }

    fn row_count(&self, cx: &App) -> usize {
        let tab = self.active;

        match self.rows_kind(tab, cx) {
            RowsKind::Empty => 1,
            RowsKind::Goals => self
                .user_services
                .read(cx)
                .profile(&self.key)
                .map(|profile| profile.goals.len())
                .unwrap_or(0),
            RowsKind::Plans => self
                .user_services
                .read(cx)
                .profile(&self.key)
                .map(|profile| profile.plans.len())
                .unwrap_or(0),
            RowsKind::Posts => self
                .scope_of(tab)
                .map(|scope| self.post_services.read(cx).items(&scope).len())
                .unwrap_or(0),
        }
    }

    fn about_modal(
        &self,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = self.profile(cx as &App);
        let metrics = theme.profile;
        let body = theme
            .typography
            .style(ProfileMetrics::BIO_SIZE, FontWeight::NORMAL);
        let label = theme
            .typography
            .style(ProfileMetrics::LABEL_SIZE, FontWeight::NORMAL);

        let mut column = div().flex().flex_col().gap(metrics.about_gap);

        let section = |key: &'static str, value: String, column: gpui::Div| {
            if value.trim().is_empty() {
                return column;
            }
            column
                .child(
                    div()
                        .text_size(label.size)
                        .line_height(label.line_height)
                        .font(label.font.clone())
                        .text_color(theme.text.secondary)
                        .child(tr(key)),
                )
                .child(
                    div()
                        .text_size(body.size)
                        .line_height(body.line_height)
                        .font(body.font.clone())
                        .text_color(theme.text.primary)
                        .child(value),
                )
        };

        column = section("profile_field_tag", tagged(&profile.tag), column);
        column = section(
            "profile_field_description",
            profile.description.clone(),
            column,
        );
        column = section(
            "profile_field_gender",
            gender(profile.gender.as_deref()),
            column,
        );
        column = section("profile_field_languages", profile.p_lang.join(", "), column);
        column = section(
            "social_links_title",
            profile
                .social_links
                .iter()
                .map(|link| {
                    if link.title.is_empty() {
                        link.url.clone()
                    } else {
                        format!("{} — {}", link.title, link.url)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            column,
        );

        let screen = cx.entity();
        let close = move |_window: &mut Window, cx: &mut App| {
            screen.update(cx, |screen, cx| {
                screen.shows_about = false;
                cx.notify();
            });
        };

        ModalUi::new("profile.about")
            .title(tr("profile_more_button"))
            .width(ModalWidth::Medium)
            .track_focus(&self.focus)
            .on_dismiss(close.clone())
            .child(column)
            .action(ModalAction::primary("close", tr("ok")).on_click(close))
            .render(theme, window, cx)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowsKind {
    Posts,
    Goals,
    Plans,
    Empty,
}

enum Rows {
    Posts,
    Goals(Vec<GoalItem>),
    Plans(Vec<PlanItem>),
    Empty(&'static str, &'static str),
}

impl Render for Profile {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx as &App).clone();

        let count = self.row_count(cx as &App) + Self::HEADER_ROWS;
        let index = ProfileContentTab::ALL
            .iter()
            .position(|candidate| *candidate == self.active)
            .unwrap_or(0);
        if self.lists[index].item_count() != count {
            self.lists[index].replaced(count);
        }

        let profile = self.profile(cx as &App);
        let title = if profile.tag.is_empty() {
            tr("navbtn_profile")
        } else {
            format!("@{}", profile.tag)
        };

        let body = self.body(&theme, cx);
        let shell = ScreenShellUi::new()
            .header(PageHeaderUi::new().title(title), &theme)
            .content(body)
            .render(&theme);

        if self.shows_about {
            return div()
                .size_full()
                .child(shell)
                .child(self.about_modal(&theme, window, cx))
                .into_any_element();
        }

        shell.into_any_element()
    }
}

fn tagged(tag: &str) -> String {
    if tag.trim().is_empty() {
        return String::new();
    }
    format!("@{tag}")
}

fn gender(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some("male") => tr("gender_male").to_string(),
        Some("female") => tr("gender_female").to_string(),
        Some("other") => tr("gender_other").to_string(),
        _ => String::new(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_route_wins_over_an_id_and_a_blank_route_is_your_own() {
        assert_eq!(
            ProfileRouteTarget::resolve(Some("u1"), Some("Aianov")),
            ProfileRouteTarget::ForeignTag("aianov".into())
        );
        assert_eq!(
            ProfileRouteTarget::resolve(Some("u1"), None),
            ProfileRouteTarget::ForeignUserId("u1".into())
        );
        assert_eq!(
            ProfileRouteTarget::resolve(None, None),
            ProfileRouteTarget::Own
        );
        assert_eq!(
            ProfileRouteTarget::resolve(Some("   "), Some("  ")),
            ProfileRouteTarget::Own,
            "whitespace is not an identity"
        );
    }

    #[test]
    fn each_route_addresses_the_key_its_payload_comes_from() {
        assert_eq!(ProfileRouteTarget::Own.key(), ProfileKey::Own);
        assert_eq!(
            ProfileRouteTarget::ForeignTag("aianov".into()).key(),
            ProfileKey::Tag("aianov".into())
        );
        assert_eq!(
            ProfileRouteTarget::ForeignUserId("u1".into()).key(),
            ProfileKey::Id("u1".into())
        );
    }

    #[test]
    fn the_header_rows_sit_above_the_first_item_of_every_tab() {
        assert_eq!(
            Profile::HEADER_ROWS,
            2,
            "the profile header and the tab bar are rows of the same list"
        );
    }

    #[test]
    fn the_error_state_names_a_string_the_catalogue_carries() {
        let catalogue = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/locales/locale-en.json");
        let text = std::fs::read_to_string(catalogue).expect("the English catalogue ships");

        for key in [
            "profile_load_error",
            "navbtn_profile",
            "profile_field_tag",
            "profile_field_description",
            "profile_field_gender",
            "profile_field_languages",
            "social_links_title",
            "gender_male",
            "gender_female",
            "gender_other",
        ] {
            assert!(
                text.contains(&format!("\"{key}\"")),
                "{key} is untranslated"
            );
        }
    }
}
