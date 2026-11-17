use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Axis, Bounds, Context, ElementId, Entity, Hsla, Pixels, ScrollHandle, SharedString, Subscription, Task,
    WeakEntity, Window, canvas, point,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_errors::ErrorCode as RiseErrorCode;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::motion::{CubicBezier, duration_from_seconds};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::bot::components::bot_ui_components::{BotSettingsFieldRow, BotStateView, BotToggleRow};
use crate::modules::bot::shared::bot_ui_validation::{BotFieldRules, BotFormValidation, BotMiniAppURLPolicy};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::BotActionsStore;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotMiniAppLaunchData, BotMiniAppModel, BotMiniAppUpdateValues,
};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;

const PRODUCTION_URL_POINTER: &str = "/production_url";
const DISABLED_SAVE_OPACITY: f32 = 0.42;
const URL_SCROLL_SECONDS: f64 = 0.25;
const URL_SCROLL_STEP: Duration = Duration::from_millis(16);

struct BotMiniAppEditorState {
    short_name: String,
    title: String,
    description: String,
    production_url: String,
    development_url: String,
    icon_url: String,
    is_active: bool,
    did_hydrate: bool,
    launch_error: Option<String>,
    scroll_handle: ScrollHandle,
    url_section_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    url_scroll: Option<Task<()>>,
    url_top_inset: Rc<Cell<f32>>,
    _observations: Vec<Subscription>,
}

impl BotMiniAppEditorState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = BotActionsStore::shared(cx);
        let services = BotServicesStore::shared(cx);
        Self {
            short_name: String::new(),
            title: String::new(),
            description: String::new(),
            production_url: String::new(),
            development_url: String::new(),
            icon_url: String::new(),
            is_active: true,
            did_hydrate: false,
            launch_error: None,
            scroll_handle: ScrollHandle::new(),
            url_section_bounds: Rc::new(Cell::new(None)),
            url_scroll: None,
            url_top_inset: Rc::new(Cell::new(0.0)),
            _observations: vec![
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn production_url_valid(&self) -> bool {
        Url::from_string(&self.production_url).is_some_and(|url| BotMiniAppURLPolicy::allows(&url, false))
    }

    fn development_url_valid(&self) -> bool {
        if self.development_url.is_empty() {
            return true;
        }
        Url::from_string(&self.development_url).is_some_and(|url| BotMiniAppURLPolicy::allows(&url, true))
    }

    fn can_save(&self) -> bool {
        BotFormValidation::is_valid_mini_app_short_name(&self.short_name)
            && BotFieldRules::mini_app_title().is_satisfied(&self.title)
            && BotFieldRules::mini_app_description().is_satisfied(&self.description)
            && self.production_url_valid()
            && self.development_url_valid()
    }

    fn hydrate_if_possible(&mut self, mini_app: Option<&BotMiniAppModel>, cx: &mut Context<Self>) {
        let Some(mini_app) = mini_app else {
            return;
        };
        if self.did_hydrate {
            return;
        }
        self.did_hydrate = true;
        self.short_name = mini_app.short_name.clone();
        self.title = mini_app.title.clone();
        self.description = mini_app.description.clone();
        self.production_url = mini_app.production_url.clone();
        self.development_url = mini_app.development_url.clone();
        self.icon_url = mini_app.icon_url.clone();
        self.is_active = mini_app.is_active;
        cx.notify();
    }

    fn scroll_to_url_section(&mut self, cx: &mut Context<Self>) {
        let Some(section) = self.url_section_bounds.get() else {
            return;
        };
        let handle = self.scroll_handle.clone();
        let start = handle.offset();
        let top = handle.bounds().origin.y + px(self.url_top_inset.get());
        let target_y = start.y - (section.origin.y - top);
        let duration = duration_from_seconds(URL_SCROLL_SECONDS);
        let began = RiseClock::monotonic();
        self.url_scroll = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                let elapsed = RiseClock::monotonic().saturating_sub(began);
                let fraction = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);
                let eased = CubicBezier::EASE_IN_OUT.progress(fraction) as f32;
                handle.set_offset(point(start.x, start.y + (target_y - start.y) * eased));
                let finished = fraction >= 1.0;
                let alive = this
                    .update(cx, |state, cx| {
                        if finished {
                            state.url_scroll = None;
                        }
                        cx.notify();
                    })
                    .is_ok();
                if finished || !alive {
                    break;
                }
                cx.background_executor().timer(URL_SCROLL_STEP).await;
            }
        }));
    }
}

type SubmitAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
struct EditorActions {
    bot_id: String,
    mini_app_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl EditorActions {
    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn load(&self, state: &Entity<BotMiniAppEditorState>, cx: &mut App) {
        let Some(mini_app_id) = self.mini_app_id.as_deref() else {
            state.update(cx, |state, cx| {
                state.did_hydrate = true;
                cx.notify();
            });
            return;
        };
        let mini_app = BotServicesStore::mini_app(&self.bot_id, mini_app_id, cx);
        state.update(cx, |state, cx| state.hydrate_if_possible(mini_app.as_ref(), cx));
        BotInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.load_mini_app(&self.bot_id, mini_app_id, cx));
    }

    fn save(&self, state: &Entity<BotMiniAppEditorState>, cx: &mut App) {
        let (can_save, short_name, title, description, production_url, development_url, icon_url, is_active) = {
            let state = state.read(cx);
            (
                state.can_save(),
                state.short_name.clone(),
                trimming_whitespaces_and_newlines(&state.title).to_owned(),
                trimming_whitespaces_and_newlines(&state.description).to_owned(),
                trimming_whitespaces_and_newlines(&state.production_url).to_owned(),
                trimming_whitespaces_and_newlines(&state.development_url).to_owned(),
                trimming_whitespaces_and_newlines(&state.icon_url).to_owned(),
                state.is_active,
            )
        };
        if !can_save {
            return;
        }
        let interactions = BotInteractionsStore::shared(cx);
        if let Some(mini_app_id) = self.mini_app_id.as_deref() {
            let popping = self.clone();
            interactions.update(cx, |store, cx| {
                store.update_mini_app(
                    &self.bot_id,
                    mini_app_id,
                    BotMiniAppUpdateValues {
                        title: Some(title),
                        description: Some(description),
                        production_url: Some(production_url),
                        development_url: Some(development_url),
                        icon_url: Some(icon_url),
                        is_active: Some(is_active),
                    },
                    Some(Rc::new(move |_: BotMiniAppModel, cx: &mut App| popping.pop(cx))),
                    cx,
                );
            });
            return;
        }
        let finishing = self.clone();
        interactions.update(cx, |store, cx| {
            store.create_mini_app(
                &self.bot_id,
                &short_name,
                &title,
                &description,
                &production_url,
                &development_url,
                &icon_url,
                Some(Rc::new(move |created: BotMiniAppModel, cx: &mut App| {
                    if is_active {
                        finishing.pop(cx);
                        return;
                    }
                    let finishing = finishing.clone();
                    cx.defer(move |cx| {
                        let popping = finishing.clone();
                        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.update_mini_app(
                                &finishing.bot_id,
                                &created.id,
                                BotMiniAppUpdateValues { is_active: Some(false), ..BotMiniAppUpdateValues::default() },
                                Some(Rc::new(move |_: BotMiniAppModel, cx: &mut App| popping.pop(cx))),
                                cx,
                            );
                        });
                    });
                })),
                cx,
            );
        });
    }

    fn delete_mini_app(&self, cx: &mut App) {
        let Some(mini_app_id) = self.mini_app_id.as_deref() else {
            return;
        };
        let popping = self.clone();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.delete_mini_app(
                &self.bot_id,
                mini_app_id,
                Some(Rc::new(move |cx: &mut App| popping.pop(cx))),
                cx,
            );
        });
    }

    fn launch_development_preview(&self, state: &Entity<BotMiniAppEditorState>, cx: &mut App) {
        let Some(mini_app_id) = self.mini_app_id.as_deref() else {
            return;
        };
        let launching = self.clone();
        let answering = state.downgrade();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.create_mini_app_launch_data(
                &self.bot_id,
                mini_app_id,
                None,
                None,
                true,
                None,
                Some(Rc::new(move |launch: BotMiniAppLaunchData, cx: &mut App| {
                    let url = Url::from_string(&launch.launch_url)
                        .filter(|url| BotMiniAppURLPolicy::allows(url, true));
                    let Some(url) = url else {
                        let _ = answering.update(cx, |state, cx| {
                            state.launch_error = Some(tr("bot_mini_app_url_blocked"));
                            cx.notify();
                        });
                        return;
                    };
                    let Some(owner) = answering.upgrade() else {
                        return;
                    };
                    let title = owner.read(cx).title.clone();
                    if let Some(nav_store) = &launching.nav_store {
                        let route = RootRoute::BotMiniApp {
                            url,
                            title,
                            allows_development_http: true,
                            chat_id: None,
                            bot_id: None,
                            is_web_link: false,
                        };
                        nav_store.update(cx, |nav_store, cx| {
                            nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                        });
                    }
                })),
                None,
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
pub struct BotMiniAppEditor {
    bot_id: String,
    mini_app_id: Option<String>,
}

impl BotMiniAppEditor {
    pub fn new(bot_id: impl Into<String>, mini_app_id: Option<String>) -> Self {
        Self { bot_id: bot_id.into(), mini_app_id }
    }

    fn child_id(&self, name: &str) -> ElementId {
        let mini_app = self.mini_app_id.as_deref().unwrap_or_default();
        ElementId::Name(SharedString::from(format!("bot-mini-app-editor-{}-{mini_app}-{name}", self.bot_id)))
    }

    fn field_binding(
        state: &Entity<BotMiniAppEditorState>,
        get: fn(&BotMiniAppEditorState) -> String,
        set: fn(&mut BotMiniAppEditorState, String),
    ) -> Binding<String> {
        Binding::entity(state.clone(), get, move |state, value, _| set(state, value))
    }

    fn field_row(
        &self,
        name: &str,
        icon: &'static str,
        icon_background: Hsla,
        placeholder: String,
        text: Binding<String>,
        submit: &SubmitAction,
    ) -> BotSettingsFieldRow {
        let submit = submit.clone();
        BotSettingsFieldRow::new(self.child_id(name), icon, icon_background, placeholder, text)
            .on_submit(move |window, cx| submit(window, cx))
    }

    fn identity_section(&self, state: &Entity<BotMiniAppEditorState>, submit: &SubmitAction) -> GroupedBtns {
        let is_editing = self.mini_app_id.is_some();
        let short_name = Binding::entity(
            state.clone(),
            |state| state.short_name.clone(),
            move |state, value: String, _| {
                if !is_editing {
                    state.short_name = BotFormValidation::normalized_mini_app_short_name(&value);
                }
            },
        );
        let title = Self::field_binding(
            state,
            |state| state.title.clone(),
            |state, value| state.title = BotFieldRules::mini_app_title().clamped(&value),
        );
        let description = Self::field_binding(
            state,
            |state| state.description.clone(),
            |state, value| state.description = BotFieldRules::mini_app_description().clamped(&value),
        );
        let icon_url = Self::field_binding(state, |state| state.icon_url.clone(), |state, value| state.icon_url = value);

        GroupedBtns::new(
            self.child_id("identity"),
            vec![
                GroupedBtn::new("shortName", "").disabled(is_editing).content_slot(self.field_row(
                    "short-name",
                    "number",
                    color("#5856D6"),
                    tr("bot_mini_app_short_name"),
                    short_name,
                    submit,
                )),
                GroupedBtn::new("title", "").content_slot(self.field_row(
                    "title",
                    "textformat",
                    color("#007AFF"),
                    tr("bot_mini_app_title"),
                    title,
                    submit,
                )),
                GroupedBtn::new("description", "").content_slot(
                    self.field_row(
                        "description",
                        "text.alignleft",
                        SettingsIconPalette::privacy_description(),
                        tr("bot_mini_app_description"),
                        description,
                        submit,
                    )
                    .axis(Axis::Vertical),
                ),
                GroupedBtn::new("icon", "").content_slot(self.field_row(
                    "icon",
                    "app.fill",
                    color("#AF52DE"),
                    tr("bot_mini_app_icon_url"),
                    icon_url,
                    submit,
                )),
            ],
        )
        .title(tr("bot_mini_app_identity"))
        .corner_radius(22.0)
    }

    fn url_section(
        &self,
        state: &Entity<BotMiniAppEditorState>,
        submit: &SubmitAction,
        production_state: (bool, bool),
        development_state: (bool, bool),
        theme: &AppTheme,
    ) -> GroupedBtns {
        let production = Self::field_binding(
            state,
            |state| state.production_url.clone(),
            |state, value| state.production_url = value,
        );
        let development = Self::field_binding(
            state,
            |state| state.development_url.clone(),
            |state, value| state.development_url = value,
        );
        GroupedBtns::new(
            self.child_id("urls"),
            vec![
                GroupedBtn::new("production", "").content_slot(
                    self.field_row(
                        "production",
                        "lock.fill",
                        color("#34C759"),
                        tr("bot_mini_app_production_url"),
                        production,
                        submit,
                    )
                    .trailing(url_state_icon(production_state.0, production_state.1, theme)),
                ),
                GroupedBtn::new("development", "").content_slot(
                    self.field_row(
                        "development",
                        "hammer.fill",
                        color("#FF9500"),
                        tr("bot_mini_app_development_url"),
                        development,
                        submit,
                    )
                    .trailing(url_state_icon(development_state.0, development_state.1, theme)),
                ),
            ],
        )
        .title(tr("bot_mini_app_urls"))
        .end_group_title(tr("bot_mini_app_url_policy_hint"))
        .corner_radius(22.0)
    }

    fn status_section(&self, state: &Entity<BotMiniAppEditorState>) -> GroupedBtns {
        GroupedBtns::new(
            self.child_id("status"),
            vec![GroupedBtn::new("active", "").content_slot(BotToggleRow::new(
                self.child_id("active"),
                "power",
                color("#34C759"),
                tr("bot_mini_app_active"),
                tr("bot_mini_app_active_hint"),
                Binding::entity(state.clone(), |state| state.is_active, |state, value, _| state.is_active = value),
            ))],
        )
        .title(tr("bot_mini_app_availability"))
        .corner_radius(22.0)
    }
}

fn url_state_icon(valid: bool, empty: bool, theme: &AppTheme) -> Div {
    let tint: Hsla = if valid { UIColor::system_green().resolved_color(theme.appearance) } else { theme.semantic.error_100 };
    let symbol = if valid { "checkmark.circle.fill" } else { "exclamationmark.circle.fill" };
    div().children((!empty).then(|| IconUi::sized(symbol, px(17.0), tint, true)).flatten())
}

impl RenderOnce for BotMiniAppEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| BotMiniAppEditorState::new(cx));
        let actions_store = BotActionsStore::shared(cx);
        let editor_actions =
            EditorActions { bot_id: self.bot_id.clone(), mini_app_id: self.mini_app_id.clone(), nav_store: nav_store.clone() };
        let is_editing = self.mini_app_id.is_some();
        let mini_app = self.mini_app_id.as_deref().and_then(|mini_app_id| {
            BotServicesStore::mini_app(&self.bot_id, mini_app_id, cx)
        });

        let (get_status, get_error, pending, production_url_refused, mutation_error) = {
            let actions = actions_store.read(cx);
            let refused = actions
                .create_mini_app_sai
                .failure()
                .or_else(|| actions.update_mini_app_sai.failure())
                .is_some_and(|failure| {
                    failure.code.as_deref() == Some(RiseErrorCode::CommonUrlInvalid.wire())
                        && failure.canonical_field_pointer().as_deref() == Some(PRODUCTION_URL_POINTER)
                });
            let mutation_error = actions
                .create_mini_app_sai
                .error()
                .or_else(|| actions.update_mini_app_sai.error())
                .or_else(|| actions.delete_mini_app_sai.error())
                .or_else(|| actions.create_mini_app_launch_data_sai.error());
            (
                actions.get_mini_app_sai.status(),
                actions.get_mini_app_sai.error(),
                actions.create_mini_app_sai.is_pending() || actions.update_mini_app_sai.is_pending(),
                refused,
                mutation_error,
            )
        };
        let status =
            if !is_editing || mini_app.is_some() { SaiWsSliceStatus::Fulfilled } else { get_status };
        let (can_save, production_valid, development_valid, production_empty, development_empty, url_bounds) = {
            let current = state.read(cx);
            (
                current.can_save(),
                current.production_url_valid(),
                current.development_url_valid(),
                current.production_url.is_empty(),
                current.development_url.is_empty(),
                current.url_section_bounds.clone(),
            )
        };
        let (launch_error, scroll_handle) = {
            let current = state.read(cx);
            (current.launch_error.clone(), current.scroll_handle.clone())
        };

        let refused_state = state.clone();
        on_change(self.child_id("refused-change"), production_url_refused, false, window, cx, move |_, refused, _, cx| {
            if *refused {
                refused_state.update(cx, |state, cx| state.scroll_to_url_section(cx));
            }
        });
        let hydrating = state.clone();
        on_change(self.child_id("mini-app-change"), mini_app, false, window, cx, move |_, mini_app, _, cx| {
            hydrating.update(cx, |state, cx| state.hydrate_if_possible(mini_app.as_ref(), cx));
        });
        on_change(self.child_id("ready-change"), can_save, true, window, cx, |_, ready, _, cx| {
            GuideFacts::set(RiseGuideSignal::BotMiniAppReady, *ready, cx);
        });
        let launch_alert_state = state.clone();
        on_change(self.child_id("launch-alert"), launch_error, false, window, cx, move |_, launch_error, _, cx| {
            let Some(message) = launch_error.clone() else {
                return;
            };
            NativeConfirmAlert::present_notice(
                &tr("bot_mini_app_launch_error"),
                &message,
                None,
                None,
                move |cx: &mut App| {
                    launch_alert_state.update(cx, |state, cx| {
                        state.launch_error = None;
                        cx.notify();
                    });
                },
                cx,
            );
        });

        let saving_state = state.clone();
        let saving_actions = editor_actions.clone();
        let save_enabled = can_save && !pending;
        let save_button = div()
            .opacity(if save_enabled { 1.0 } else { DISABLED_SAVE_OPACITY })
            .child(
                PageHeaderIconButton::new(self.child_id("save"), "checkmark", move |_, cx| {
                    saving_actions.save(&saving_state, cx);
                })
                .disabled(!save_enabled),
            )
            .guide_anchor(RiseGuideAnchor::BotMiniAppSave);

        let retry_state = state.clone();
        let retry_actions = editor_actions.clone();
        let render_error_id = self.child_id("render-error");
        let render_error_message = get_error.clone();
        let appear_id = self.child_id("task");
        let disappear_id = self.child_id("closed");
        let appearing = state.clone();
        let appear_actions = editor_actions.clone();
        let title = tr(if is_editing { "bot_mini_app_edit" } else { "bot_mini_app_create" });
        let error_text = get_error.or_else(|| Some(tr("bot_mini_app_load_error")));

        let content_state = state.clone();
        let content_actions = editor_actions;
        let content_theme = theme;
        let content = SaiDataRender::new(status, move |_, cx| {
            let state = content_state;
            let actions = content_actions;
            let theme = content_theme;
            state.read(cx).url_top_inset.set(list_shell_floating_top_inset(cx));
            let saving = state.clone();
            let saving_actions = actions.clone();
            let submit: SubmitAction =
                Rc::new(move |_: &mut Window, cx: &mut App| saving_actions.save(&saving, cx));
            let url_section = self.url_section(
                &state,
                &submit,
                (production_valid && !production_url_refused, production_empty),
                (development_valid, development_empty),
                &theme,
            );
            let owner_tools = is_editing.then(|| {
                let previewing_state = state.clone();
                let previewing = actions.clone();
                let confirming = actions.clone();
                let mut preview = GroupedBtn::new("preview", tr("bot_mini_app_dev_preview"))
                    .icon("hammer.fill")
                    .icon_background(color("#FF9500"))
                    .disabled(development_empty || !development_valid)
                    .show_chevron(false)
                    .callback(move |_, cx| previewing.launch_development_preview(&previewing_state, cx));
                if development_empty {
                    preview = preview.subtitle(tr("bot_mini_app_dev_preview_missing"));
                }
                GroupedBtns::new(
                    self.child_id("owner-tools"),
                    vec![
                        preview,
                        GroupedBtn::new("delete", tr("bot_mini_app_delete"))
                            .icon("trash.fill")
                            .icon_background(UIColor::system_red().resolved_color(theme.appearance))
                            .destructive(true)
                            .show_chevron(false)
                            .callback(move |_, cx| {
                                let deleting = confirming.clone();
                                NativeConfirmAlert::present_delete(
                                    &tr("bot_mini_app_delete"),
                                    &tr("bot_mini_app_delete_message"),
                                    Some(tr("bot_mini_app_delete")),
                                    None,
                                    false,
                                    None,
                                    None,
                                    move |cx: &mut App| deleting.delete_mini_app(cx),
                                    cx,
                                );
                            }),
                    ],
                )
                .title(tr("bot_mini_app_owner_tools"))
                .corner_radius(22.0)
            });
            let error_line = |error: String, theme: &AppTheme| {
                div().w_full().px(px(18.0)).child(MainText::new(error).px(13.0).color(theme.semantic.error_100))
            };

            scroll_view(self.child_id("scroll"))
                .size_full()
                .scroll_handle(scroll_handle)
                .list_shell_scroll_under_chrome(cx)
                .bg(theme.bg._100)
                .child(
                    vstack(20.0)
                        .w_full()
                        .pt(px(12.0))
                        .child(
                            vstack(20.0)
                                .w_full()
                                .child(
                                    div()
                                        .w_full()
                                        .child(self.identity_section(&state, &submit))
                                        .guide_anchor(RiseGuideAnchor::BotMiniAppIdentity),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .relative()
                                        .child(url_section)
                                        .child(
                                            canvas(
                                                move |bounds, _, _| url_bounds.set(Some(bounds)),
                                                |_, _, _, _| {},
                                            )
                                            .absolute()
                                            .size_full(),
                                        ),
                                )
                                .guide_anchor(RiseGuideAnchor::BotMiniAppUrls),
                        )
                        .children(production_url_refused.then(|| error_line(tr("bot_mini_app_url_unreachable"), &theme)))
                        .child(div().w_full().child(self.status_section(&state)))
                        .children(owner_tools.map(|owner_tools| div().w_full().child(owner_tools)))
                        .children(
                            mutation_error
                                .filter(|_| !production_url_refused)
                                .map(|error| error_line(error, &theme)),
                        )
                        .child(spacer_min_h(80.0)),
                )
        })
        .is_empty(false)
        .error_text(error_text)
        .sai_data_render_error(move |_, _| {
            let retrying = retry_state.clone();
            let actions = retry_actions.clone();
            BotStateView::new(
                render_error_id.clone(),
                "exclamationmark.arrow.triangle.2.circlepath",
                tr("bot_mini_app_load_error"),
            )
            .message(render_error_message.clone())
            .action_title(tr("bot_retry"))
            .action(move |_, cx| actions.load(&retrying, cx))
        });

        SettingsWrapperUi::new(title, move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .right_jsx(save_button)
        .content(content)
        .on_appear(appear_id, move |_, cx| appear_actions.load(&appearing, cx))
        .on_disappear(disappear_id, |_, cx| {
            GuideFacts::set(RiseGuideSignal::BotMiniAppReady, false, cx);
        })
    }
}
