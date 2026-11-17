use gpui::{
    App, Context, ElementId, Entity, FontWeight, Global, Hsla, KeyBinding, SharedString, Subscription, Task,
    Window, actions, white,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::number_format::decimal_number_formatter::DecimalNumberFormatter;
use rise_i18n::{I18n, Locale, tr};
use rise_canon::rise_ledger::HistoryFilter as RiseLedgerHistoryFilter;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabConfig;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine};
use rise_ui::swiftui::page_refresh::{PAGE_REFRESH_BINDING, PageFocus};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::animated_tabs_ui::animated_tabs::AnimatedTabs;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftShopShelfGrid;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftItemKind;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::sparks::components::sparks_icon_view::{SparksAmountLabel, SparksIconView, SparksPageHero};
use crate::modules::sparks::engine::rise_sparks_engine_models::RiseSparksLoadPhase;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksHistoryRow;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;

actions!(my_sparks_page, [RefreshWallet]);

const KEY_CONTEXT: &str = "MySparksPage";

struct MySparksPageKeyBindingsInstalled;

impl Global for MySparksPageKeyBindingsInstalled {}

fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<MySparksPageKeyBindingsInstalled>() {
        return;
    }
    cx.set_global(MySparksPageKeyBindingsInstalled);
    cx.bind_keys([KeyBinding::new(PAGE_REFRESH_BINDING, RefreshWallet, Some(KEY_CONTEXT))]);
}

pub struct MySparksPage {
    refresh_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MySparksPage {
    const FILTERS: [RiseLedgerHistoryFilter; 4] = [
        RiseLedgerHistoryFilter::All,
        RiseLedgerHistoryFilter::Received,
        RiseLedgerHistoryFilter::Spent,
        RiseLedgerHistoryFilter::Purchases,
    ];
    const ROW_HEIGHT: f32 = 60.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);
        let service = SparksServicesStore::shared(cx);
        Self {
            refresh_task: None,
            _subscriptions: vec![cx.observe(&service, |_, _, cx| cx.notify())],
        }
    }

    fn refresh_wallet(&mut self, cx: &mut Context<Self>) {
        if self.refresh_task.is_some() {
            return;
        }
        let work = SparksInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.pull_to_refresh_wallet(cx));
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            work.await;
            this.update(cx, |this, _| this.refresh_task = None).ok();
        }));
    }

    fn prefetch_neighbours(cx: &mut App) {
        let selected = SparksServicesStore::shared(cx).read(cx).selected_filter;
        let Some(index) = Self::FILTERS.iter().position(|filter| *filter == selected) else {
            return;
        };
        let neighbours: Vec<RiseLedgerHistoryFilter> = [index.checked_sub(1), Some(index + 1)]
            .into_iter()
            .flatten()
            .filter_map(|neighbour| Self::FILTERS.get(neighbour).copied())
            .collect();
        SparksInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.prefetch_history(&neighbours, cx));
    }

    fn title_key(filter: RiseLedgerHistoryFilter) -> &'static str {
        match filter {
            RiseLedgerHistoryFilter::All => "sparks_history_tab_all",
            RiseLedgerHistoryFilter::Received => "sparks_history_tab_received",
            RiseLedgerHistoryFilter::Spent => "sparks_history_tab_spent",
            RiseLedgerHistoryFilter::Purchases => "sparks_history_tab_purchases",
        }
    }
}

impl Render for MySparksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let service = SparksServicesStore::shared(cx);
        let selected_filter = service.read(cx).selected_filter;
        let balance = service.read(cx).balance().clone();
        on_change("my-sparks-selected-filter", selected_filter, false, window, cx, |_, _, _, cx| {
            Self::prefetch_neighbours(cx);
        });
        on_change("my-sparks-balance", balance, false, window, cx, |_, _, _, cx| {
            SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.wallet_moved(cx));
            Self::prefetch_neighbours(cx);
        });
        let page_focus = PageFocus::keyed("my-sparks-page-focus", window, cx);
        let focus = page_focus.read(cx).handle.clone();
        let leaving_focus = page_focus.clone();

        div()
            .id("my-sparks-page")
            .key_context(KEY_CONTEXT)
            .track_focus(&focus)
            .size_full()
            .on_action(cx.listener(|this: &mut Self, _: &RefreshWallet, _, cx| this.refresh_wallet(cx)))
            .child(
                SettingsWrapperUi::new(tr("sparks_my_title"), move |_, cx| {
                    if let Some(nav_store) = nav_store.as_ref() {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(MySparksPageBody),
            )
            .on_appear("my-sparks-page-appear", move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.open_wallet(cx));
                Self::prefetch_neighbours(cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
    }
}

#[derive(IntoElement)]
struct MySparksPageBody;

impl RenderOnce for MySparksPageBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let service = SparksServicesStore::shared(cx);
        let header = Self::header(service.read(cx), &theme);
        let history_tabs = Self::history_tabs(&service, cx);
        scroll_view("my-sparks-scroll")
            .refreshable(|_, cx| {
                SparksInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.pull_to_refresh_wallet(cx))
            })
            .list_shell_scroll_under_chrome_resetting(true, cx)
            .bg(theme.bg._100)
            .child(
                vstack(0.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(div().w_full().pb(px(20.0)).child(header))
                    .child(div().w_full().child(history_tabs))
                    .child(div().flex_none().h(px(32.0))),
            )
    }
}

impl MySparksPageBody {
    fn header(service: &SparksServicesStore, theme: &AppTheme) -> AnyElement {
        vstack(10.0)
            .w_full()
            .child(SparksPageHero::new())
            .child(SecondaryText::new(tr("sparks_balance_caption")).px(14.0).tac(TextAlignT::Center).w_full())
            .child(Self::balance_value(service))
            .child(div().w_full().pt(px(8.0)).child(Self::why_sparks(theme)))
            .child(
                div().w_full().px(px(16.0)).pt(px(6.0)).child(
                    ButtonUi::new("my-sparks-buy")
                        .height(50.0)
                        .on_press(|_, cx| {
                            SparksInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.open_purchase_sheet(None, cx));
                        })
                        .child(
                            hstack(8.0).child(SparksIconView::new().size(20.0)).child(
                                MainText::new(tr("sparks_buy_button"))
                                    .px(17.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(white()),
                            ),
                        ),
                ),
            )
            .child(Self::secondary_button(theme, "gift.fill", "gift_market_title_self", false, false, |_, cx| {
                GiftsInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.open_shop_for_self(RiseGiftItemKind::Item, cx));
            }))
            .child(div().w_full().pt(px(6.0)).child(Self::received_card(service, theme)))
            .child(Self::secondary_button(
                theme,
                "arrow.down.to.line",
                "sparks_withdraw_button",
                service.received_total() <= 0,
                service.is_transferring(),
                |_, cx| {
                    SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.withdraw_received(cx));
                },
            ))
            .into_any_element()
    }

    fn why_sparks(theme: &AppTheme) -> impl IntoElement {
        vstack(6.0)
            .w_full()
            .px(px(28.0))
            .child(
                MainText::new(tr("sparks_why_title"))
                    .px(17.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .w_full(),
            )
            .child(
                MainText::new(tr("sparks_why_text"))
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.primary.opacity(0.88))
                    .line_height(19.0)
                    .w_full(),
            )
    }

    fn secondary_button(
        theme: &AppTheme,
        system_image: &str,
        title_key: &'static str,
        disabled: bool,
        is_pending: bool,
        on_press: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div().w_full().px(px(16.0)).child(
            ButtonUi::new(ElementId::Name(format!("my-sparks-{title_key}").into()))
                .disabled(disabled)
                .is_pending(is_pending)
                .background_color(theme.bg._200)
                .height(50.0)
                .on_press(on_press)
                .child(
                    hstack(8.0)
                        .children(IconUi::sized(system_image, px(17.0), theme.text.primary, system_image.ends_with(".fill")))
                        .child(
                            MainText::new(tr(title_key))
                                .px(17.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        ),
                ),
        )
    }

    fn balance_value(service: &SparksServicesStore) -> AnyElement {
        if service.has_balance() {
            return hstack(8.0)
                .child(SparksIconView::new().size(34.0))
                .child(
                    MainText::new(
                        DecimalNumberFormatter::new(I18n::shared().code()).string(service.balance().available.max(0) as u64),
                    )
                    .px(40.0)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1)
                    .monospaced_digits(true),
                )
                .into_any_element();
        }
        if let RiseSparksLoadPhase::Failed(message) = service.balance_phase() {
            return vstack(8.0)
                .w_full()
                .h(px(48.0))
                .justify_center()
                .child(SecondaryText::new(message.clone()).px(14.0).tac(TextAlignT::Center).w_full())
                .child(retry_button("my-sparks-balance-retry"))
                .into_any_element();
        }
        SkeletonBlock::new(160.0, 44.0, 12.0).into_any_element()
    }

    fn received_card(service: &SparksServicesStore, theme: &AppTheme) -> impl IntoElement {
        let total: AnyElement = if service.has_balance() {
            SparksAmountLabel::new(service.received_total()).px(15.0).icon_size(16.0).into_any_element()
        } else {
            SkeletonLine::new(56.0, SkeletonLine::default().diameter).into_any_element()
        };
        let channels = service.received_channels().into_iter().map(|channel| {
            hstack(8.0)
                .w_full()
                .h(px(28.0))
                .child(
                    UserLogoUi::new(ElementId::Name(format!("my-sparks-channel-{}", channel.chat_id).into()))
                        .source(SharedString::from(channel.logo.clone()))
                        .name(SharedString::from(channel.title.clone()))
                        .color_scope_id(SharedString::from(channel.chat_id.clone()))
                        .size(24.0)
                        .show_border(false)
                        .show_story_ring(false)
                        .open_story_on_press(false),
                )
                .child(SecondaryText::new(channel.title.clone()).px(13.0).line_limit(1).min_w_0())
                .child(spacer_min_w(8.0))
                .child(div().flex_none().child(SparksAmountLabel::new(channel.amount).px(13.0).icon_size(13.0)))
        });
        div().w_full().px(px(16.0)).child(
            vstack(6.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .p(px(14.0))
                .continuous_corners(14.0)
                .bg(theme.primary._100.opacity(0.06))
                .child(
                    hstack(8.0)
                        .w_full()
                        .child(MainText::new(tr("sparks_received_title")).px(15.0).font_weight(FontWeight::MEDIUM))
                        .child(spacer_min_w(8.0))
                        .child(total),
                )
                .children(channels)
                .child(SecondaryText::new(tr("sparks_received_note")).px(12.0).w_full()),
        )
    }

    /// swiped like the other tabbed screens, pages take their rows height and scroll with the wallet
    fn history_tabs(service: &Entity<SparksServicesStore>, cx: &mut App) -> AnimatedTabs {
        let tabs = MySparksPage::FILTERS
            .iter()
            .enumerate()
            .map(|(index, filter)| {
                let filter = *filter;
                TabConfig::new(index as i64, tr(MySparksPage::title_key(filter)), move |_, _| {
                    SparksHistoryPage::new(filter, MySparksPage::ROW_HEIGHT).into_any_element()
                })
            })
            .collect();
        let service = service.clone();
        let interactions = SparksInteractionsStore::shared(cx);
        let active_tab = Binding::new(
            move |cx| {
                let selected = service.read(cx).selected_filter;
                MySparksPage::FILTERS.iter().position(|filter| *filter == selected).unwrap_or(0) as i64
            },
            move |index, _, cx| {
                let Some(filter) = usize::try_from(index).ok().and_then(|index| MySparksPage::FILTERS.get(index).copied())
                else {
                    return;
                };
                interactions.update(cx, |interactions, cx| interactions.select_filter(filter, cx));
            },
        );
        AnimatedTabs::new("my-sparks-history-tabs", tabs, active_tab)
            .horizontal_padding(16.0)
            .top_padding(0.0)
            .bottom_padding(8.0)
            .sizes_pages_vertically_to_content(true)
            .minimum_page_height(MySparksPage::ROW_HEIGHT * 3.0)
    }
}

fn retry_button(id: impl Into<ElementId>) -> ButtonUi {
    ButtonUi::new(id)
        .fit_content(true)
        .padding_horizontal(20.0)
        .on_press(|_, cx| {
            SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.retry_wallet(cx));
        })
        .child(
            MainText::new(tr("common_retry"))
                .px(15.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white()),
        )
}

#[derive(IntoElement)]
struct SparksHistoryPage {
    filter: RiseLedgerHistoryFilter,
    row_height: f32,
}

impl SparksHistoryPage {
    const LOADER_HEIGHT: f32 = 44.0;
    const INITIAL_ROWS: i64 = 14;

    fn new(filter: RiseLedgerHistoryFilter, row_height: f32) -> Self {
        Self { filter, row_height }
    }

    /// the last row is built only when the scroll gets within a couple of rows of it
    fn load_more(filter: RiseLedgerHistoryFilter, cx: &mut App) {
        let service = SparksServicesStore::shared(cx);
        let service = service.read(cx);
        if filter != service.selected_filter || !service.has_more_history(filter) || service.is_loading_more(filter) {
            return;
        }
        SparksInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_history(cx));
    }

    fn skeleton_row(&self, index: usize) -> impl IntoElement {
        hstack(12.0)
            .id(ElementId::Name(format!("sparks-history-skeleton-{}-{index}", self.filter.slug()).into()))
            .w_full()
            .px(px(16.0))
            .h(px(self.row_height))
            .child(SkeletonCircle::new(40.0))
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(150.0, SkeletonLine::default().diameter))
                    .child(SkeletonLine::new(90.0, 8.0)),
            )
            .child(spacer())
            .child(SkeletonLine::new(52.0, SkeletonLine::default().diameter))
    }
}

impl RenderOnce for SparksHistoryPage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let service = SparksServicesStore::shared(cx);
        let service = service.read(cx);
        let filter = self.filter;
        let row_height = self.row_height;
        let rows = service.rows(filter);
        let page = vstack(0.0).w_full();
        if !rows.is_empty() {
            let row_count = rows.len();
            let last_id = rows.last().map(|row| row.id.clone());
            let grid = GiftShopShelfGrid::new(
                ElementId::Name(format!("sparks-history-grid-{}", filter.slug()).into()),
                rows,
                1,
                move |_: &mut Window, _: &mut App| div().h(px(row_height)),
                move |row: &SparksHistoryRow, _: &mut Window, _: &mut App| -> AnyElement {
                    let view = SparksHistoryRowView::new(row.clone(), row_height, filter);
                    if last_id.as_deref() == Some(row.id.as_str()) {
                        let id = ElementId::Name(format!("sparks-history-last-{}-{}", filter.slug(), row.id).into());
                        return view.on_appear(id, move |_, cx| Self::load_more(filter, cx)).into_any_element();
                    }
                    view.into_any_element()
                },
            )
            .spacing(0.0)
            .initial_rows(Self::INITIAL_ROWS);
            return page
                .child(div().w_full().h(px(row_count as f32 * row_height)).child(grid))
                .when(service.is_loading_more(filter), |page| {
                    page.child(
                        div()
                            .w_full()
                            .h(px(Self::LOADER_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(progress_view().tint(theme.primary._100)),
                    )
                });
        }
        if service.has_history(filter) {
            return page.child(
                div()
                    .w_full()
                    .px(px(32.0))
                    .py(px(28.0))
                    .child(SecondaryText::new(tr("sparks_history_empty")).px(14.0).tac(TextAlignT::Center).w_full()),
            );
        }
        if let RiseSparksLoadPhase::Failed(message) = service.history_phase(filter) {
            return page.child(
                vstack(10.0)
                    .w_full()
                    .py(px(24.0))
                    .child(SecondaryText::new(message).px(14.0).tac(TextAlignT::Center).w_full())
                    .child(retry_button(ElementId::Name(format!("sparks-history-retry-{}", filter.slug()).into()))),
            );
        }
        page.children((0..6).map(|index| self.skeleton_row(index)))
    }
}

#[derive(IntoElement)]
struct SparksHistoryRowView {
    row: SparksHistoryRow,
    height: f32,
    filter: RiseLedgerHistoryFilter,
}

impl SparksHistoryRowView {
    fn new(row: SparksHistoryRow, height: f32, filter: RiseLedgerHistoryFilter) -> Self {
        Self { row, height, filter }
    }

    fn child_id(&self, part: &str) -> ElementId {
        ElementId::Name(format!("sparks-history-{part}-{}-{}", self.filter.slug(), self.row.id).into())
    }

    fn content(&self, theme: &AppTheme, nav_store: &Option<Entity<NavigationStore>>) -> Div {
        let color: Option<Hsla> = self.row.is_incoming.then_some(theme.primary._100);
        let sign = if self.row.is_incoming { "+" } else { "\u{2212}" };
        hstack(12.0)
            .w_full()
            .px(px(16.0))
            .h(px(self.height))
            .child(self.avatar(theme, nav_store))
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .min_w_0()
                    .child(
                        MainText::new(tr(self.row.title_key()))
                            .px(15.0)
                            .font_weight(FontWeight::MEDIUM)
                            .number_of_lines(1),
                    )
                    .when_some(self.subtitle(), |column, subtitle| {
                        column.child(SecondaryText::new(subtitle).px(12.0).line_limit(1))
                    }),
            )
            .child(spacer_min_w(8.0))
            .child(
                hstack(3.0)
                    .flex_none()
                    .child(
                        MainText::new(format!("{sign}{}", RiseFormat::compact_count(self.row.amount)))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(color)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    )
                    .child(SparksIconView::new().size(14.0)),
            )
    }

    fn subtitle(&self) -> Option<String> {
        let date = self.row.created_at.as_ref().map(|date| {
            RiseFormat::date_time_label(date, &RiseClock::now(), &Calendar::current(), Locale::app_locale().identifier())
        });
        match (self.row.counterparty_name.as_deref(), date) {
            (Some(name), Some(date)) => Some(format!("{name} \u{b7} {date}")),
            (Some(name), None) => Some(name.to_owned()),
            (None, Some(date)) => Some(date),
            (None, None) => None,
        }
    }

    /// avatar of a user counterparty opens their profile, the rest of the row opens the post
    fn avatar(&self, theme: &AppTheme, nav_store: &Option<Entity<NavigationStore>>) -> AnyElement {
        let Some(user_id) = self.row.counterparty_user_id.clone() else {
            return self.avatar_image(theme);
        };
        let route = RootRoute::UserPage { user_id: Some(user_id), tag: self.row.counterparty_tag.clone() };
        let nav_store = nav_store.clone();
        plain(self.child_id("avatar-open"))
            .flex_none()
            .on_press(move |_, cx| {
                cx.stop_propagation();
                if let Some(nav_store) = nav_store.as_ref() {
                    let route = route.clone();
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(self.avatar_image(theme))
            .into_any_element()
    }

    fn avatar_image(&self, theme: &AppTheme) -> AnyElement {
        if let Some(name) = self.row.counterparty_name.as_ref() {
            return UserLogoUi::new(self.child_id("avatar"))
                .source(self.row.counterparty_logo.clone().map(SharedString::from))
                .name(SharedString::from(name.clone()))
                .size(40.0)
                .show_border(false)
                .show_story_ring(false)
                .open_story_on_press(false)
                .into_any_element();
        }
        div()
            .flex_none()
            .size(px(40.0))
            .flex()
            .items_center()
            .justify_center()
            .continuous_corners(12.0)
            .bg(theme.primary._100.opacity(0.08))
            .child(SparksIconView::new().size(20.0))
            .into_any_element()
    }
}

impl RenderOnce for SparksHistoryRowView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let content = self.content(&theme, &nav_store);
        let route = if let Some(post_id) = self.row.post_id.clone() {
            RootRoute::PostDetail { post_id: Some(post_id), origin: None }
        } else if let Some(user_id) = self.row.counterparty_user_id.clone() {
            RootRoute::UserPage { user_id: Some(user_id), tag: self.row.counterparty_tag.clone() }
        } else {
            return content.into_any_element();
        };
        plain(self.child_id("open"))
            .w_full()
            .on_press(move |_, cx| {
                if let Some(nav_store) = nav_store.as_ref() {
                    let route = route.clone();
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(content)
            .into_any_element()
    }
}
