use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Empty, Entity, FontWeight, IntoElement, ListAlignment, ParentElement, Pixels,
    Point, RenderOnce, SharedString, Styled, Subscription, Window, div, px, transparent_black,
};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::AppTheme;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::{MainText, TextAlignT};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsPresenter;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::PostCollaboratorsSnapshot;
use crate::modules::post::stores::post_actions::post_types::{
    PostCollaboratorKind, PostCollaboratorRow, PostCollaboratorStatus, PostCollaboratorUser,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;

#[derive(IntoElement)]
pub struct CoauthorsSheet;

impl RenderOnce for CoauthorsSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        PostCollaboratorsListSheet::new(PostCollaboratorKind::Coauthor, tr("post_coauthors_title"))
    }
}

#[derive(IntoElement)]
pub struct TaggedPeopleSheet;

impl RenderOnce for TaggedPeopleSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        PostCollaboratorsListSheet::new(PostCollaboratorKind::Tag, tr("post_tagged_people_title"))
    }
}

#[derive(Clone, Default)]
struct PostCollaboratorSections {
    accepted: Vec<PostCollaboratorRow>,
    pending: Vec<PostCollaboratorRow>,
}

impl PostCollaboratorSections {
    fn new(items: &[PostCollaboratorRow], kind: PostCollaboratorKind) -> Self {
        let mut sections = Self::default();
        let wanted = kind.raw_value();
        let pending_status = PostCollaboratorStatus::Pending.raw_value();
        for row in items.iter().filter(|row| row.kind == wanted) {
            if row.status == pending_status {
                sections.pending.push(row.clone());
            } else {
                sections.accepted.push(row.clone());
            }
        }
        sections
    }

    fn count(&self) -> usize {
        self.accepted.len() + self.pending.len()
    }

    fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

/// one list entry per item, gpui list needs a flat indexed model
enum PostCollaboratorListEntry {
    SectionTitle(String),
    Collaborator { row: PostCollaboratorRow, index: usize },
    LoadingSkeleton,
}

impl PostCollaboratorListEntry {
    const SECTION_TITLE_HEIGHT: f32 = 38.0;
    const SEPARATOR_HEIGHT: f32 = 0.5;
    const ROW_HEIGHT: f32 = UserRelationshipRowMetrics::AVATAR_SIZE
        + UserRelationshipRowMetrics::VERTICAL_PADDING * 2.0
        + Self::SEPARATOR_HEIGHT;
    const LOADING_HEIGHT: f32 = 128.0;
    const EMPTY_HEIGHT: f32 = 160.0;

    fn height(&self) -> f32 {
        match self {
            Self::SectionTitle(_) => Self::SECTION_TITLE_HEIGHT,
            Self::Collaborator { .. } => Self::ROW_HEIGHT,
            Self::LoadingSkeleton => Self::LOADING_HEIGHT,
        }
    }

    fn id(&self) -> String {
        match self {
            Self::SectionTitle(title) => format!("title-{title}"),
            Self::Collaborator { row, .. } => format!("row-{}", row.user.id),
            Self::LoadingSkeleton => "loading".to_owned(),
        }
    }
}

struct PostCollaboratorsListModel {
    list: ListUi,
    ids: Vec<String>,
}

impl PostCollaboratorsListModel {
    fn new(cx: &App) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), ids: Vec::new() }
    }

    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        if !self.ids.is_empty() && ids.len() > self.ids.len() && ids.starts_with(&self.ids) {
            self.list.appended(ids.len() - self.ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.ids = ids;
    }
}

/// observers on the presenter and post store plus the hold menu the sheet locks under
/// they notify whichever view renders this sheet
struct PostCollaboratorsListObservers {
    _subscriptions: Vec<Subscription>,
}

impl PostCollaboratorsListObservers {
    fn new(cx: &mut Context<Self>) -> Self {
        let presenter = PostCollaboratorsPresenter::shared(cx);
        let content = RisePostContentPresentation::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&presenter, |_, _, cx| cx.notify()),
                cx.observe(&content, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct PostCollaboratorsListSheet {
    kind: PostCollaboratorKind,
    title: String,
    presentation_override: Option<Binding<bool>>,
    snapshot_override: Option<PostCollaboratorsSnapshot>,
}

impl PostCollaboratorsListSheet {
    pub fn new(kind: PostCollaboratorKind, title: impl Into<String>) -> Self {
        Self { kind, title: title.into(), presentation_override: None, snapshot_override: None }
    }

    pub fn presentation_override(mut self, presentation_override: Option<Binding<bool>>) -> Self {
        self.presentation_override = presentation_override;
        self
    }

    pub fn snapshot_override(mut self, snapshot_override: Option<PostCollaboratorsSnapshot>) -> Self {
        self.snapshot_override = snapshot_override;
        self
    }
}

impl RenderOnce for PostCollaboratorsListSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let id = format!("post-collaborators-sheet-{}", self.kind.raw_value());
        let keyed = |suffix: &str| ElementId::Name(format!("{id}-{suffix}").into());
        window.use_keyed_state(keyed("observers"), cx, |_, cx: &mut Context<PostCollaboratorsListObservers>| {
            PostCollaboratorsListObservers::new(cx)
        });
        let list_model =
            window.use_keyed_state(keyed("list"), cx, |_, cx: &mut Context<PostCollaboratorsListModel>| {
                PostCollaboratorsListModel::new(cx)
            });
        let presenter = PostCollaboratorsPresenter::shared(cx);
        let presented = self.presentation_override.clone().unwrap_or_else(|| {
            Binding::entity(
                presenter.clone(),
                |presenter: &PostCollaboratorsPresenter| presenter.is_sheet_presented(),
                |presenter, presented, cx| presenter.set_presented(presented, cx),
            )
        });

        if self.snapshot_override.is_none() {
            let kind = self.kind;
            let post_id = presenter.read(cx).post_id().to_owned();
            on_change(keyed("load"), post_id, true, window, cx, move |_, post_id: &String, _, cx| {
                if post_id.is_empty() {
                    return;
                }
                PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.load_post_collaborators(post_id, Some(kind), None, true, None, cx);
                });
            });
        }

        let has_override = self.presentation_override.is_some();
        let did_dismiss_presenter = presenter.clone();
        let parts = PostCollaboratorsListSheetParts {
            id: id.clone(),
            kind: self.kind,
            title: self.title,
            presentation_override: self.presentation_override,
            snapshot_override: self.snapshot_override,
            presenter,
            nav_store: env::<NavigationStoreKey>(cx),
            list_model,
        };
        let hold_is_active = HoldContextMenuPresenter::has_active_request(cx);

        BottomSheetUi::new(
            SharedString::from(id),
            presented,
            move |window, cx| parts.content(window, cx),
            |_, _| Empty,
        )
        .snap_fractions(vec![0.58, 0.93])
        .fits_content(true)
        .experimental_bottom_sheet(true)
        .full_page_shows_drag_indicator(true)
        .full_page_hides_footer(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .full_page_interaction_locked(hold_is_active)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
        .on_did_dismiss(move |_, cx| {
            if !has_override {
                did_dismiss_presenter.update(cx, |presenter, cx| presenter.sheet_did_dismiss(cx));
            }
        })
    }
}

#[derive(Clone)]
struct PostCollaboratorsListSheetParts {
    id: String,
    kind: PostCollaboratorKind,
    title: String,
    presentation_override: Option<Binding<bool>>,
    snapshot_override: Option<PostCollaboratorsSnapshot>,
    presenter: Entity<PostCollaboratorsPresenter>,
    nav_store: Option<Entity<NavigationStore>>,
    list_model: Entity<PostCollaboratorsListModel>,
}

impl PostCollaboratorsListSheetParts {
    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("{}-{suffix}", self.id).into())
    }

    fn content(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let snapshot = match &self.snapshot_override {
            Some(snapshot) => snapshot.clone(),
            None => {
                let post_id = self.presenter.read(cx).post_id().to_owned();
                PostServicesStore::shared(cx).update(cx, |service, cx| service.post_collaborators(&post_id, cx))
            }
        };
        let sections = PostCollaboratorSections::new(&snapshot.items, self.kind);

        let left_parts = self.clone();
        let header = SheetHeaderBar::new(self.element_id("header"), self.title.clone(), move |window, cx| {
            match &left_parts.presentation_override {
                Some(presentation_override) => presentation_override.set(false, window, cx),
                None => left_parts.presenter.update(cx, |presenter, cx| presenter.dismiss(cx)),
            }
        })
        .right_system_image(None);

        let (body, body_height) = if !snapshot.loaded_once {
            let skeleton = scroll_view(self.element_id("skeleton"))
                .scroll_disabled(true)
                .shows_indicators(false)
                .child(UserListSkeleton::new().rows(6))
                .into_any_element();
            (skeleton, PostCollaboratorListEntry::ROW_HEIGHT * 6.0)
        } else if sections.is_empty() {
            let empty = div()
                .size_full()
                .px(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .child(MainText::new(tr("no_users")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary))
                .into_any_element();
            (empty, PostCollaboratorListEntry::EMPTY_HEIGHT)
        } else {
            self.list(sections, &snapshot, &theme, cx)
        };

        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(header)
            .child(
                div()
                    .w_full()
                    .flex_basis(px(body_height))
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .min_h_0()
                    .child(body),
            )
            .into_any_element()
    }

    fn list(
        &self,
        sections: PostCollaboratorSections,
        snapshot: &PostCollaboratorsSnapshot,
        theme: &AppTheme,
        cx: &mut App,
    ) -> (AnyElement, f32) {
        let total = sections.count();
        let shows_pending = (self.snapshot_override.is_some() || self.presenter.read(cx).viewer_is_author())
            && !sections.pending.is_empty();
        let shows_headers = self.kind == PostCollaboratorKind::Coauthor;
        let accepted_count = sections.accepted.len();

        let mut entries: Vec<PostCollaboratorListEntry> = Vec::with_capacity(total + 3);
        if !sections.accepted.is_empty() {
            if shows_headers {
                entries.push(PostCollaboratorListEntry::SectionTitle(tr("post_coauthors_accepted_section")));
            }
            entries.extend(
                sections
                    .accepted
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| PostCollaboratorListEntry::Collaborator { row, index }),
            );
        }
        if shows_pending {
            if shows_headers {
                entries.push(PostCollaboratorListEntry::SectionTitle(tr("post_coauthors_pending_section")));
            }
            entries.extend(sections.pending.into_iter().enumerate().map(|(index, row)| {
                PostCollaboratorListEntry::Collaborator { row, index: accepted_count + index }
            }));
        }
        if snapshot.is_loading && snapshot.loaded_once {
            entries.push(PostCollaboratorListEntry::LoadingSkeleton);
        }

        let content_height: f32 = entries.iter().map(PostCollaboratorListEntry::height).sum();
        let ids: Vec<String> = entries.iter().map(PostCollaboratorListEntry::id).collect();
        self.list_model.update(cx, |model, _| model.sync(ids));

        let parts = self.clone();
        let theme = theme.clone();
        let has_more = snapshot.has_more;
        let is_loading = snapshot.is_loading;
        let list = self
            .list_model
            .read(cx)
            .list
            .element(move |index, _, cx| match entries.get(index) {
                Some(PostCollaboratorListEntry::SectionTitle(title)) => {
                    Self::section_title(title.clone(), &theme).into_any_element()
                }
                Some(PostCollaboratorListEntry::Collaborator { row, index }) => {
                    parts.collaborator_row(row, *index, total, has_more, is_loading, &theme, cx)
                }
                Some(PostCollaboratorListEntry::LoadingSkeleton) => div()
                    .w_full()
                    .h(px(PostCollaboratorListEntry::LOADING_HEIGHT))
                    .child(UserListSkeleton::new().rows(2).shows_separators(false))
                    .into_any_element(),
                None => div().into_any_element(),
            })
            .into_any_element();
        (list, content_height)
    }

    fn section_title(text: String, theme: &AppTheme) -> impl IntoElement {
        div()
            .w_full()
            .h(px(PostCollaboratorListEntry::SECTION_TITLE_HEIGHT))
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .pt(px(14.0))
            .pb(px(6.0))
            .child(
                MainText::new(text)
                    .px(13.0)
                    .tac(TextAlignT::Left)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
    }

    fn separator(theme: &AppTheme) -> impl IntoElement {
        div().w_full().h(px(PostCollaboratorListEntry::SEPARATOR_HEIGHT)).bg(theme.border._100)
    }

    #[allow(clippy::too_many_arguments)]
    fn collaborator_row(
        &self,
        row: &PostCollaboratorRow,
        index: usize,
        total: usize,
        has_more: bool,
        is_loading: bool,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let actions = self.row_actions(row, cx);
        let row_id = format!("{}-row-{}", self.id, row.user.id);

        let label: AnyElement = if actions.is_empty() {
            let parts = self.clone();
            let user = row.user.clone();
            plain(ElementId::Name(format!("{row_id}-button").into()))
                .w_full()
                .on_press(move |_, cx| parts.open_profile(&user, cx))
                .child(Self::identity(row, theme))
                .into_any_element()
        } else {
            let action_parts = self.clone();
            let action_row = row.clone();
            let tap_parts = self.clone();
            let tap_user = row.user.clone();
            let content_row = row.clone();
            let content_theme = theme.clone();
            HoldContextMenuContainer::new(
                ElementId::Name(format!("{row_id}-hold").into()),
                HoldContextMenuAlignment::Leading,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |_: &mut Window, _: &mut App| {
                    Self::identity(&content_row, &content_theme).into_any_element()
                }),
            )
            .show_emoji_list(false)
            .source_identity(Some(row.user.id.clone()))
            .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                action_parts.perform(&id, &action_row, cx);
            }))
            .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
                tap_parts.open_profile(&tap_user, cx);
            }))
            .into_any_element()
        };

        let load_parts = self.clone();
        vstack(0.0)
            .w_full()
            .child(
                div()
                    .w_full()
                    .h(px(PostCollaboratorListEntry::ROW_HEIGHT - PostCollaboratorListEntry::SEPARATOR_HEIGHT))
                    .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
                    .py(px(UserRelationshipRowMetrics::VERTICAL_PADDING))
                    .bg(theme.bg._100)
                    .child(label),
            )
            .child(Self::separator(theme))
            .on_appear(ElementId::Name(format!("{row_id}-appear").into()), move |_, cx| {
                if load_parts.snapshot_override.is_some()
                    || !has_more
                    || is_loading
                    || index < total.saturating_sub(3)
                {
                    return;
                }
                let post_id = load_parts.presenter.read(cx).post_id().to_owned();
                let kind = load_parts.kind;
                PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.load_post_collaborators(&post_id, Some(kind), None, false, None, cx);
                });
            })
            .into_any_element()
    }

    fn identity(row: &PostCollaboratorRow, theme: &AppTheme) -> impl IntoElement {
        let user = &row.user;
        let handle = format!("@{}", user.tag);

        let mut name_row = hstack(4.0).child(
            MainText::new(if user.name.is_empty() { handle.clone() } else { user.name.clone() })
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .number_of_lines(1),
        );
        if user.is_official {
            name_row = name_row.child(OfficialBadge::new().size(px(14.0)));
        }
        if user.is_premium {
            name_row = name_row.child(
                PremiumBadge::new()
                    .user_id(Some(SharedString::from(user.id.clone())))
                    .emoji(user.premium_badge_emoji.clone().map(SharedString::from))
                    .size(14.0),
            );
        }

        let mut names = vstack(4.0).align(HorizontalAlignment::Leading).min_w_0().child(name_row);
        if !user.tag.is_empty() {
            names = names.child(MainText::new(handle).px(13.0).color(theme.text.secondary).number_of_lines(1));
        }

        let mut identity = hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .min_h(px(UserRelationshipRowMetrics::AVATAR_SIZE))
            .child(
                UserLogoUi::new(ElementId::Name(format!("post-collaborator-logo-{}", user.id).into()))
                    .source(SharedString::from(user.logo.clone()))
                    .name(SharedString::from(user.name.clone()))
                    .user_id(SharedString::from(user.id.clone()))
                    .size(UserRelationshipRowMetrics::AVATAR_SIZE)
                    .show_story_ring(false)
                    .open_story_on_press(false),
            )
            .child(names)
            .child(spacer_min_w(0.0));

        if row.status == PostCollaboratorStatus::Pending.raw_value() {
            identity = identity.child(
                div().flex_none().px(px(8.0)).py(px(4.0)).bg(theme.bg._300).capsule().child(
                    MainText::new(tr("post_coauthors_pending_badge"))
                        .px(11.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary)
                        .number_of_lines(1),
                ),
            );
        }
        identity
    }

    fn row_actions(&self, row: &PostCollaboratorRow, cx: &App) -> Vec<HoldContextMenuAction> {
        if self.snapshot_override.is_some() || !self.presenter.read(cx).viewer_is_author() {
            return Vec::new();
        }
        let title = match self.kind {
            PostCollaboratorKind::Coauthor => {
                if row.status == PostCollaboratorStatus::Pending.raw_value() {
                    tr("post_coauthor_revoke_invite_action")
                } else {
                    tr("post_coauthor_remove_action")
                }
            }
            PostCollaboratorKind::Tag => tr("post_tag_remove_action"),
        };
        vec![
            HoldContextMenuAction::new("remove", title)
                .system_image("person.crop.circle.badge.minus")
                .destructive(true),
        ]
    }

    fn perform(&self, id: &str, row: &PostCollaboratorRow, cx: &mut App) {
        if self.snapshot_override.is_some() || id != "remove" || !self.presenter.read(cx).viewer_is_author() {
            return;
        }
        let _ = notification(NotificationKind::Warning);
        let post_id = self.presenter.read(cx).post_id().to_owned();
        let kind = self.kind;
        let user_id = row.user.id.clone();
        PostInteractionsStore::shared(cx).update(cx, |interactions, cx| match kind {
            PostCollaboratorKind::Coauthor => interactions.revoke_coauthor(&post_id, &user_id, cx),
            PostCollaboratorKind::Tag => interactions.remove_post_tag(&post_id, &user_id, cx),
        });
    }

    fn open_profile(&self, user: &PostCollaboratorUser, cx: &mut App) {
        if self.snapshot_override.is_some() {
            return;
        }
        self.presenter.update(cx, |presenter, cx| presenter.dismiss(cx));
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let route = RootRoute::UserPage { user_id: Some(user.id.clone()), tag: Some(user.tag.clone()) };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}
