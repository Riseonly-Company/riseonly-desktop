use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, transparent_black, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_i18n::date_format::date_formatter::Style as DateFormatterStyle;
use rise_i18n::tr;
use rise_theme::{AppTheme, color};
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedEndAction};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{SaiWebSocketManager, SaiWebSocketManagerPhase};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::session::stores::session_actions::session_actions::SessionActionsStore;
use crate::modules::session::stores::session_actions::session_types::SessionEntry;
use crate::modules::session::stores::session_interactions::session_interactions::SessionInteractionsStore;
use crate::modules::session::stores::session_services::session_services::SessionServicesStore;

enum SessionRowFormat {}

impl SessionRowFormat {
    fn relative(date: &Date) -> String {
        let now = RiseClock::now().time_interval_since_1970();
        RiseFormat::elapsed((date.time_interval_since_1970() * 1000.0) as i64, (now * 1000.0) as i64)
    }

    fn date_medium(date: &Date) -> String {
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(DateFormatterStyle::Medium);
        formatter.set_time_style(DateFormatterStyle::Short);
        formatter.string_from(date)
    }
}

struct Style {
    icon: &'static str,
    background: Hsla,
    foreground: Hsla,
}

enum SessionDeviceStyle {}

impl SessionDeviceStyle {
    fn style(device: &str) -> Style {
        let lower = device.to_lowercase();
        let foreground = white();
        let blue = color("#0A84FF");
        let green = color("#34C759");
        let web_green = color("#30D158");
        let indigo = color("#5E5CE6");
        let orange = color("#FF9F0A");
        let style = |icon, background| Style { icon, background, foreground };

        if lower.contains("ipad") {
            return style("ipad", blue);
        }
        if lower.contains("iphone") {
            return style("iphone", blue);
        }
        if lower.contains("ipod") {
            return style("ipodtouch", blue);
        }
        if lower.contains("mac") || lower.contains("book") || lower.contains("imac") {
            return style("laptopcomputer", blue);
        }
        if lower.contains("android") {
            return style("candybarphone", green);
        }
        if lower.contains("windows") || lower.contains("pc") {
            return style("desktopcomputer", indigo);
        }
        if lower.contains("linux") {
            return style("terminal.fill", orange);
        }
        if ["web", "chrome", "safari", "firefox", "edge", "opera", "browser"].iter().any(|name| lower.contains(*name)) {
            return style("globe", web_green);
        }
        style("iphone", blue)
    }
}

/// page state and its stores
struct SessionsSettingsState {
    sheet_session: Option<SessionEntry>,
    waits_for_network: bool,
    _subscriptions: Vec<Subscription>,
}

impl SessionsSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = SessionServicesStore::shared(cx);
        let actions = SessionActionsStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        Self {
            sheet_session: None,
            waits_for_network: false,
            _subscriptions: vec![
                cx.observe(&store, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&socket, |state, socket, cx| {
                    let phase = socket.read(cx).phase();
                    if SessionsSettings::socket_waits_for_network(phase) {
                        state.waits_for_network = true;
                    } else if matches!(phase, SaiWebSocketManagerPhase::Connected(..)) {
                        state.waits_for_network = false;
                    }
                    cx.notify();
                }),
            ],
        }
    }

    fn sync_waits_for_network(&mut self, cx: &mut Context<Self>) {
        let socket = SaiWebSocketManager::shared(cx);
        self.waits_for_network = SessionsSettings::socket_waits_for_network(socket.read(cx).phase());
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct SessionsSettings;

impl SessionsSettings {
    pub fn new() -> Self {
        Self
    }

    fn socket_waits_for_network(phase: &SaiWebSocketManagerPhase) -> bool {
        match phase {
            SaiWebSocketManagerPhase::Disconnected | SaiWebSocketManagerPhase::Failed(_) => true,
            SaiWebSocketManagerPhase::Idle
            | SaiWebSocketManagerPhase::Connecting(_)
            | SaiWebSocketManagerPhase::Connected(..) => false,
        }
    }

    fn load_error_state(state: &Entity<SessionsSettingsState>, theme: &AppTheme) -> AnyElement {
        let state = state.clone();
        vstack(12.0)
            .w_full()
            .px(px(32.0))
            .pt(px(120.0))
            .child(SecondaryText::new(tr("error_fetching_data")).px(14.0).tac(TextAlignT::Center))
            .child(
                ButtonUi::new("sessions-settings-retry")
                    .background_color(theme.primary._100)
                    .height(42.0)
                    .fit_content(true)
                    .padding_horizontal(18.0)
                    .on_press(move |_, cx| {
                        state.update(cx, |state, cx| state.sync_waits_for_network(cx));
                        SessionInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_sessions(cx));
                    })
                    .child(MainText::new(tr("common_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
            )
            .into_any_element()
    }

    fn current_section(state: &Entity<SessionsSettingsState>, theme: &AppTheme, cx: &mut App) -> Option<AnyElement> {
        let current = SessionServicesStore::current_session(cx)?;
        let others = SessionServicesStore::other_sessions(cx);
        let store = SessionServicesStore::shared(cx);
        let is_mutating = store.read(cx).is_mutating();
        let mut section = GroupedBtns::new(
            "sessions-settings-current",
            vec![Self::session_row(&current, state, theme, Some(RiseGuideAnchor::SessionsCurrent))],
        )
        .title(tr("sessions_settings_current_session"));
        if !others.is_empty() {
            section = section.end_group_title(tr("sessions_settings_end_other_sessions")).end_action(
                GroupedEndAction::new(tr("sessions_settings_delete_other_sessions"), |_, cx| {
                    Self::present_terminate_others_confirm(cx);
                })
                .disabled(is_mutating),
            );
        }
        Some(div().w_full().child(section).into_any_element())
    }

    fn present_terminate_others_confirm(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("sessions_settings_terminate_others_confirm_title"),
            "",
            Some(tr("sessions_settings_delete_other_sessions")),
            None,
            false,
            None,
            None,
            |cx| {
                SessionInteractionsStore::shared(cx).update(cx, |store, cx| store.terminate_all_other(cx));
            },
            cx,
        );
    }

    fn other_section(
        state: &Entity<SessionsSettingsState>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let rest = SessionServicesStore::other_sessions(cx);
        if rest.is_empty() {
            return None;
        }
        let relative_id = SessionServicesStore::sessions_relative_id(cx);
        on_change("sessions-settings-relative-id", relative_id, false, window, cx, |_, _, _, cx| {
            SessionInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_sessions(cx));
        });
        let first_id = rest.first().map(|session| session.id.clone());
        let items = rest
            .iter()
            .map(|session| {
                let anchor = (Some(&session.id) == first_id.as_ref()).then_some(RiseGuideAnchor::SessionsFirstOther);
                Self::session_row(session, state, theme, anchor)
            })
            .collect();
        Some(
            GroupedBtns::new("sessions-settings-other", items)
                .title(tr("sessions_settings_other_session"))
                .on_appear("sessions-settings-other-appear", |_, cx| {
                    SessionInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_sessions(cx));
                })
                .into_any_element(),
        )
    }

    fn session_row(
        session: &SessionEntry,
        state: &Entity<SessionsSettingsState>,
        theme: &AppTheme,
        guide_anchor: Option<RiseGuideAnchor>,
    ) -> GroupedBtn {
        let style = SessionDeviceStyle::style(&session.device);
        let opened = session.clone();
        let state = state.clone();
        let mut row = GroupedBtn::new(session.id.clone(), session.device.clone())
            .icon(style.icon)
            .icon_background(style.background)
            .icon_foreground(style.foreground)
            .pretitle(session.app_line.clone())
            .guide_anchor(guide_anchor)
            .show_chevron(!session.is_current)
            .show_chevron_when_trailing_slot(false)
            .callback(move |_, cx| {
                let opened = opened.clone();
                state.update(cx, |state, cx| {
                    state.sheet_session = Some(opened);
                    cx.notify();
                });
            });
        if !session.location.is_empty() {
            row = row.subtitle(session.location.clone());
        }
        if session.is_current {
            row = row.trailing_slot(
                MainText::new(tr("session_status_active"))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.semantic.success_100),
            );
        } else {
            row = row.trailing(SessionRowFormat::relative(&session.last_active));
        }
        row
    }

    fn session_terminate_footer(state: &Entity<SessionsSettingsState>, theme: &AppTheme, cx: &App) -> AnyElement {
        let is_current = state.read(cx).sheet_session.as_ref().is_some_and(|session| session.is_current);
        let state = state.clone();
        let press: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let session = state.update(cx, |state, cx| {
                cx.notify();
                state.sheet_session.take()
            });
            let Some(session) = session else {
                return;
            };
            SessionInteractionsStore::shared(cx).update(cx, |store, cx| {
                if session.is_current {
                    store.terminate_current_session(cx);
                } else {
                    store.terminate_session(&session.id, cx);
                }
            });
        });
        let label = hstack(8.0)
            .w_full()
            .py(px(15.0))
            .justify_center()
            .bg(theme.semantic.error_100)
            .rounded_full()
            .children(IconUi::sized(
                if is_current { "rectangle.portrait.and.arrow.right" } else { "xmark.circle.fill" },
                px(16.0),
                white(),
                false,
            ))
            .child(
                MainText::new(if is_current { tr("session_terminate_current") } else { tr("determinate_session") })
                    .px(16.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white()),
            )
            .liquid_press(
                "sessions-settings-terminate",
                LiquidPressShape::RoundedRect(16.0),
                true,
                false,
                false,
                Some(press),
            );
        div()
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(div().w_full().child(label).guide_anchor(RiseGuideAnchor::SessionSheetTerminate))
            .into_any_element()
    }
}

impl RenderOnce for SessionsSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("sessions-settings-state", cx, |_, cx| SessionsSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let is_empty = SessionServicesStore::sessions(cx).is_empty();
        let actions = SessionActionsStore::shared(cx);
        let status = if state.read(cx).waits_for_network && is_empty {
            SaiWsSliceStatus::Rejected
        } else {
            actions.read(cx).get_sessions_sai.status()
        };
        let sheet_open = state.read(cx).sheet_session.is_some();
        on_change("sessions-settings-sheet-open", sheet_open, false, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::SessionSheetOpen, *open, cx);
        });

        let content_state = state.clone();
        let content_theme = theme.clone();
        let error_state = state.clone();
        let error_theme = theme.clone();
        let data = SaiDataRender::new(status, move |window, cx| {
            vstack(22.0)
                .w_full()
                .pt(px(12.0))
                .children(SessionsSettings::current_section(&content_state, &content_theme, cx))
                .children(SessionsSettings::other_section(&content_state, &content_theme, window, cx))
                .child(
                    div().w_full().px(px(16.0)).child(
                        MainText::new(tr("sessions_location_note"))
                            .px(13.0)
                            .tac(TextAlignT::Left)
                            .color(content_theme.text.secondary),
                    ),
                )
                .child(div().flex_none().h(px(80.0)))
        })
        .id("sessions-settings-data")
        .is_empty(is_empty)
        .no_data_text(tr("no_data_yet"))
        .error_text(tr("error_fetching_data"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, _| div().w_full().pt(px(12.0)).child(SkeletonListUi::default()))
        .sai_data_render_error(move |_, _| SessionsSettings::load_error_state(&error_state, &error_theme));

        let binding_state = state.clone();
        let is_bottom_sheet = Binding::entity(
            binding_state,
            |state: &SessionsSettingsState| state.sheet_session.is_some(),
            |state: &mut SessionsSettingsState, open, _| {
                if !open {
                    state.sheet_session = None;
                }
            },
        );
        let sheet_state = state.clone();
        let footer_state = state.clone();
        let footer_theme = theme.clone();
        let sheet = BottomSheetUi::new(
            "sessions-settings-sheet",
            is_bottom_sheet,
            move |_, cx| {
                let session = sheet_state.read(cx).sheet_session.clone();
                div().w_full().when_some(session, |sheet, session| sheet.child(SessionSheetInner { session }))
            },
            move |_, cx| SessionsSettings::session_terminate_footer(&footer_state, &footer_theme, cx),
        )
        .snap_fractions(vec![0.62])
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .fits_content(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()));

        let appear_state = state.clone();
        let wrapper = SettingsWrapperUi::new(tr("settings_sessions_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("sessions-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(data),
        );

        div().relative().size_full().child(wrapper).child(sheet).on_appear("sessions-settings-appear", move |_, cx| {
            SessionInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_sessions_loaded(cx));
            appear_state.update(cx, |state, cx| state.sync_waits_for_network(cx));
        })
        .and_on_disappear(|_, cx| GuideFacts::set(RiseGuideSignal::SessionSheetOpen, false, cx))
    }
}

#[derive(IntoElement)]
struct SessionSheetInner {
    session: SessionEntry,
}

impl SessionSheetInner {
    fn info_row(id: &'static str, icon: &'static str, hex: &str, text: String, trailing: String) -> GroupedBtn {
        GroupedBtn::new(id, text)
            .icon(icon)
            .icon_background(color(hex))
            .icon_foreground(white())
            .trailing(trailing)
            .show_chevron(false)
    }

    fn info_rows(&self) -> Vec<GroupedBtn> {
        let session = &self.session;
        let mut rows = vec![Self::info_row(
            "app",
            "app.badge.fill",
            "#0A84FF",
            tr("session_info_app"),
            session.app_line.clone(),
        )];
        if !session.location.is_empty() {
            rows.push(Self::info_row(
                "loc",
                "mappin.and.ellipse",
                "#34C759",
                tr("session_info_location"),
                session.location.clone(),
            ));
        }
        if !session.ip.is_empty() {
            rows.push(Self::info_row("ip", "network", "#30D158", tr("session_info_ip"), session.ip.clone()));
        }
        if !session.is_current {
            rows.push(Self::info_row(
                "last",
                "clock.fill",
                "#FF9F0A",
                tr("session_info_last_active"),
                SessionRowFormat::relative(&session.last_active),
            ));
        }
        if let Some(first) = &session.first_active {
            rows.push(Self::info_row(
                "first",
                "calendar",
                "#5E5CE6",
                tr("session_info_first_login"),
                SessionRowFormat::date_medium(first),
            ));
        }
        rows
    }
}

impl RenderOnce for SessionSheetInner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let style = SessionDeviceStyle::style(&self.session.device);
        let is_current = self.session.is_current;
        let status_text =
            if is_current { tr("session_status_active") } else { SessionRowFormat::relative(&self.session.last_active) };
        scroll_view("sessions-settings-sheet-scroll").w_full().shows_indicators(false).child(
            vstack(20.0)
                .w_full()
                .pt(px(4.0))
                .pb(px(8.0))
                .child(
                    div().pt(px(6.0)).child(
                        div()
                            .size(px(84.0))
                            .rounded(px(24.0))
                            .bg(style.background)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(style.icon, px(36.0), style.foreground, false)),
                    ),
                )
                .child(
                    vstack(6.0)
                        .w_full()
                        .px(px(16.0))
                        .child(
                            MainText::new(self.session.device.clone())
                                .px(20.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            MainText::new(status_text)
                                .px(14.0)
                                .tac(TextAlignT::Center)
                                .font_weight(if is_current { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                .color(if is_current { theme.semantic.success_100 } else { theme.text.secondary }),
                        ),
                )
                .child(GroupedBtns::new("sessions-settings-sheet-info", self.info_rows())),
        )
    }
}
