use gpui::{
    AnyElement, App, Context, Div, ElementId, Empty, Entity, FontWeight, ListAlignment, SharedString, Subscription, Window,
    transparent_black,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::MainText;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::stores::message::message_services::message_poll_voters_services::{
    PollOptionVoters, PollOptionVotersLoadState, PollVoterRow, PollVotersPresentation, PollVotersPresentationOption,
    PollVotersStore,
};
use crate::modules::post::pages::posts::post_detail::profile_post_list_body::splice_list_rows;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;
use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;

const SHEET_ID: &str = "poll-voters-sheet";
const COLLAPSED_SHEET_FRACTION: f32 = 0.58;
const EXPANDED_SHEET_FRACTION: f32 = 0.93;
const HEADER_HEIGHT: f32 = 116.0;
const CARD_CORNER_RADIUS: f32 = 14.0;
const BOTTOM_ROW_ID: &str = "bottom";
const SEARCH_MAX_SCALARS: usize = 128;

fn element_id(suffix: impl AsRef<str>) -> ElementId {
    ElementId::Name(format!("{SHEET_ID}-{}", suffix.as_ref()).into())
}

/// state of the voters sheet, its observer notifies whichever view renders the sheet
struct PollVotersSheetState {
    selected_sheet_fraction: f32,
    query: String,
    search_active: bool,
    list: ListUi,
    list_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

/// one row of the flattened sections, the answer title then its card of voters
/// skeleton or footer too. first and last round the card corners
struct PollVotersListRow {
    id: String,
    option: usize,
    kind: PollVotersListRowKind,
    first: bool,
    last: bool,
}

enum PollVotersListRowKind {
    Title,
    Voter(usize),
    Skeleton,
    Footer,
}

impl PollVotersSheetState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = PollVotersStore::shared(cx);
        Self {
            selected_sheet_fraction: COLLAPSED_SHEET_FRACTION,
            query: String::new(),
            search_active: false,
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            _subscriptions: vec![cx.observe(&store, |_, _, cx| cx.notify())],
        }
    }

    fn reset_presentation(&mut self, cx: &mut Context<Self>) {
        self.query.clear();
        self.search_active = false;
        self.selected_sheet_fraction = COLLAPSED_SHEET_FRACTION;
        cx.notify();
    }
}

#[derive(IntoElement, Default)]
pub struct PollVotersSheet;

impl PollVotersSheet {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PollVotersSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(element_id("state"), cx, |_, cx: &mut Context<PollVotersSheetState>| {
            PollVotersSheetState::new(cx)
        });
        let store = PollVotersStore::shared(cx);
        let message_id = store.read(cx).presentation().map(|presentation| presentation.message_id.clone());

        let change_state = state.clone();
        on_change(element_id("message"), message_id, false, window, cx, move |_, message_id, _, cx| {
            if message_id.is_some() {
                change_state.update(cx, |state, cx| state.reset_presentation(cx));
            }
        });

        let visible = Binding::new(
            {
                let store = store.clone();
                move |cx: &App| store.read(cx).presentation().is_some()
            },
            {
                let store = store.clone();
                move |value: bool, _, cx| {
                    if !value {
                        store.update(cx, |store, cx| store.dismiss(cx));
                    }
                }
            },
        );

        let parts = PollVotersSheetParts { state: state.clone(), store: store.clone(), nav_store: env::<NavigationStoreKey>(cx) };
        let dismiss_store = store;

        BottomSheetUi::new(SHEET_ID, visible, move |window, cx| parts.sheet_content(window, cx), |_, _| Empty)
            .snap_fractions(vec![COLLAPSED_SHEET_FRACTION, EXPANDED_SHEET_FRACTION])
            .selected_snap_fraction(Some(Binding::entity(
                state,
                |state: &PollVotersSheetState| state.selected_sheet_fraction,
                |state, value, _| state.selected_sheet_fraction = value,
            )))
            .snap_animation_duration(0.4)
            .experimental_bottom_sheet(true)
            .experimental_uses_liquid_glass_surface(false)
            .full_page_shows_drag_indicator(true)
            .full_page_hides_footer(true)
            .full_page_avoids_keyboard_when_footer_hidden(true)
            .footer_shows_top_border(false)
            .content_scrollable(true)
            .sheet_background(Some(theme.bg._100))
            .footer_background(Some(transparent_black()))
            .on_dismiss(move |_, cx| dismiss_store.update(cx, |store, cx| store.dismiss(cx)))
    }
}

#[derive(Clone)]
struct PollVotersSheetParts {
    state: Entity<PollVotersSheetState>,
    store: Entity<PollVotersStore>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PollVotersSheetParts {
    fn sheet_content(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100.opacity(0.94))
            .children(self.results(&theme, cx).map(|results| div().absolute().inset_0().child(results)))
            .child(div().absolute().top_0().left_0().right_0().child(self.header(&theme, cx)))
            .into_any_element()
    }

    fn results(&self, theme: &AppTheme, cx: &mut App) -> Option<AnyElement> {
        let presentation = self.store.read(cx).presentation()?.clone();
        let query = self.state.read(cx).query.clone();
        let search = trimming_whitespaces_and_newlines(&query).to_owned();
        let rows = Self::list_rows(&presentation, &search);
        let mut ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        ids.push(BOTTOM_ROW_ID.to_owned());
        self.state.update(cx, |state, _| splice_list_rows(state.list.state(), &mut state.list_ids, &ids));

        let parts = self.clone();
        let theme = theme.clone();
        let list = self.state.read(cx).list.element(move |index, _, _| {
            let Some(row) = rows.get(index) else {
                return div().w_full().h(px(24.0)).into_any_element();
            };
            div()
                .w_full()
                .when(index == 0, |cell| cell.pt(px(HEADER_HEIGHT)))
                .child(parts.list_row(&presentation, row, &search, &theme))
                .into_any_element()
        });
        Some(list.into_any_element())
    }

    fn list_rows(presentation: &PollVotersPresentation, search: &str) -> Vec<PollVotersListRow> {
        let mut rows = Vec::new();
        for (option_index, option) in presentation.options.iter().enumerate() {
            let Some(state) = presentation.voters.get(&option.id).filter(|state| state.total_count > 0) else {
                continue;
            };
            let matching: Vec<usize> = state
                .voters
                .iter()
                .enumerate()
                .filter(|(_, voter)| search.is_empty() || voter.matches(search))
                .map(|(index, _)| index)
                .collect();
            if !search.is_empty() && matching.is_empty() {
                continue;
            }
            rows.push(PollVotersListRow {
                id: format!("title-{}", option.id),
                option: option_index,
                kind: PollVotersListRowKind::Title,
                first: false,
                last: false,
            });
            let mut card: Vec<(String, PollVotersListRowKind)> = matching
                .into_iter()
                .map(|index| {
                    (format!("voter-{}-{}", option.id, state.voters[index].id), PollVotersListRowKind::Voter(index))
                })
                .collect();
            if state.voters.is_empty() && state.load_state != PollOptionVotersLoadState::Failed {
                card.push((format!("skeleton-{}", option.id), PollVotersListRowKind::Skeleton));
            } else if search.is_empty()
                && (state.load_state == PollOptionVotersLoadState::Failed || state.has_more)
            {
                card.push((format!("footer-{}", option.id), PollVotersListRowKind::Footer));
            }
            let count = card.len();
            rows.extend(card.into_iter().enumerate().map(|(position, (id, kind))| PollVotersListRow {
                id,
                option: option_index,
                kind,
                first: position == 0,
                last: position + 1 == count,
            }));
        }
        rows
    }

    fn list_row(
        &self,
        presentation: &PollVotersPresentation,
        row: &PollVotersListRow,
        search: &str,
        theme: &AppTheme,
    ) -> AnyElement {
        let Some(option) = presentation.options.get(row.option) else {
            return Empty.into_any_element();
        };
        let Some(state) = presentation.voters.get(&option.id) else {
            return Empty.into_any_element();
        };
        let content = match row.kind {
            PollVotersListRowKind::Title => return Self::section_title(option, state, theme).into_any_element(),
            PollVotersListRowKind::Voter(index) => match state.voters.get(index) {
                Some(voter) => self.voter_row(&option.id, voter),
                None => return Empty.into_any_element(),
            },
            PollVotersListRowKind::Skeleton => UserListSkeleton::new()
                .rows(state.total_count.clamp(0, 3) as usize)
                .shows_separators(false)
                .into_any_element(),
            PollVotersListRowKind::Footer => {
                if !search.is_empty() {
                    return Empty.into_any_element();
                }
                match self.section_footer(&option.id, state, theme) {
                    Some(footer) => footer,
                    None => return Empty.into_any_element(),
                }
            }
        };
        div()
            .w_full()
            .px(px(12.0))
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .bg(theme.bg._200.opacity(0.6))
                    .when(row.first, |card| card.rounded_t(px(CARD_CORNER_RADIUS)))
                    .when(row.last, |card| card.rounded_b(px(CARD_CORNER_RADIUS)))
                    .child(content),
            )
            .into_any_element()
    }

    fn section_title(option: &PollVotersPresentationOption, state: &PollOptionVoters, theme: &AppTheme) -> Div {
        hstack(8.0)
            .w_full()
            .items_baseline()
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .pt(px(18.0))
            .pb(px(6.0))
            .child(
                div().flex_1().min_w_0().child(
                    MainText::new(format!("{} - {}%", option.text, option.percent))
                        .px(13.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                ),
            )
            .child(MainText::new(state.total_count.to_string()).px(13.0).color(theme.text.secondary).number_of_lines(1))
    }

    fn section_footer(&self, option_id: &str, state: &PollOptionVoters, theme: &AppTheme) -> Option<AnyElement> {
        if state.load_state == PollOptionVotersLoadState::Failed {
            return Some(self.footer_button(option_id, tr("common_retry"), false, theme));
        }
        if state.has_more {
            let hidden = state.hidden_count();
            let more = if hidden > 0 { format!(" {hidden}") } else { String::new() };
            return Some(self.footer_button(
                option_id,
                format!("{}{more}", tr("common_show_more")),
                state.load_state == PollOptionVotersLoadState::Loading,
                theme,
            ));
        }
        None
    }

    fn footer_button(&self, option_id: &str, title: String, is_loading: bool, theme: &AppTheme) -> AnyElement {
        let store = self.store.clone();
        let load_option_id = option_id.to_owned();
        plain(element_id(format!("more-{option_id}")))
            .disabled(is_loading)
            .on_press(move |_, cx| store.update(cx, |store, cx| store.load_more(&load_option_id, cx)))
            .child(
                hstack(8.0)
                    .w_full()
                    .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
                    .min_h(px(44.0))
                    .child(MainText::new(title).px(15.0).color(theme.primary._100).number_of_lines(1))
                    .child(spacer())
                    .child(
                        div()
                            .opacity(if is_loading { 1.0 } else { 0.0 })
                            .child(progress_view().control_size(ControlSize::Small)),
                    ),
            )
            .into_any_element()
    }

    fn voter_row(&self, option_id: &str, voter: &PollVoterRow) -> AnyElement {
        let open_parts = self.clone();
        let open_voter = voter.clone();
        let name = if voter.name.is_empty() { format!("@{}", voter.tag) } else { voter.name.clone() };
        let mut name_row =
            hstack(4.0).min_w_0().child(MainText::new(name).px(16.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1));
        if voter.is_official {
            name_row = name_row.child(OfficialBadge::new().size(px(14.0)));
        }
        if voter.is_premium {
            name_row = name_row.child(
                PremiumBadge::new()
                    .user_id(Some(SharedString::from(voter.id.clone())))
                    .emoji(voter.premium_badge_emoji.clone().map(SharedString::from))
                    .size(14.0),
            );
        }
        plain(element_id(format!("voter-{option_id}-{}", voter.id)))
            .on_press(move |_, cx| open_parts.open_profile(&open_voter, cx))
            .child(
                hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
                    .w_full()
                    .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
                    .py(px(6.0))
                    .child(
                        UserLogoUi::new(SharedString::from(format!("{SHEET_ID}-logo-{option_id}-{}", voter.id)))
                            .source(voter.logo.clone().map(SharedString::from))
                            .name(SharedString::from(voter.name.clone()))
                            .user_id(SharedString::from(voter.id.clone()))
                            .size(40.0),
                    )
                    .child(name_row)
                    .child(spacer()),
            )
            .into_any_element()
    }

    fn header(&self, theme: &AppTheme, cx: &App) -> Div {
        let search_active = self.state.read(cx).search_active;
        let title = self.store.read(cx).presentation().map(|presentation| presentation.question.clone()).unwrap_or_default();
        let query_binding = Binding::entity(
            self.state.clone(),
            |state: &PollVotersSheetState| state.query.clone(),
            |state, value: String, _| {
                state.query = value.chars().take(SEARCH_MAX_SCALARS).collect();
            },
        );
        let placeholder = SharedString::from(tr("search_placeholder"));
        let search_field = if search_active {
            SearchFieldUi::new(element_id("search-inline"), query_binding)
                .placeholder(placeholder)
                .uses_inline_text_field(true)
                .auto_focus(true)
        } else {
            let activate_parts = self.clone();
            SearchFieldUi::new(element_id("search"), query_binding)
                .placeholder(placeholder)
                .on_activate(move |_, cx| activate_parts.activate_search(cx))
        };
        let close_store = self.store.clone();
        vstack(4.0)
            .w_full()
            .bg(theme.bg._100.opacity(0.92))
            .child(
                SheetHeaderBar::new(element_id("header"), title, move |_, cx| {
                    close_store.update(cx, |store, cx| store.dismiss(cx));
                })
                .right_system_image(None),
            )
            .child(div().w_full().px(px(12.0)).pb(px(10.0)).child(search_field))
    }

    fn activate_search(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.selected_sheet_fraction = EXPANDED_SHEET_FRACTION;
            state.search_active = true;
            cx.notify();
        });
    }

    fn open_profile(&self, voter: &PollVoterRow, cx: &mut App) {
        self.store.update(cx, |store, cx| store.dismiss(cx));
        if let Some(nav_store) = &self.nav_store {
            let route = RootRoute::UserPage { user_id: Some(voter.id.clone()), tag: Some(voter.tag.clone()) };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}
