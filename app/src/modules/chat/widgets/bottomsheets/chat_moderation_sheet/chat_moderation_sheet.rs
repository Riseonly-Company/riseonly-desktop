use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, Render, Subscription, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_count, character_prefix};
use rise_i18n::tr;
use rise_platform::haptics::{self, ImpactStyle, NotificationKind};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::Shape;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::chat::shared::message_permissions::ChatModerationRights;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatPerm;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const SHEET_SNAP_FRACTION: f32 = 0.52;
const AUTHOR_NAME_LIMIT: usize = 22;
const OPTION_MARK_SIDE: f32 = 20.0;
const OPTION_MARK_RESPONSE: f64 = 0.22;
const OPTION_MARK_DAMPING: f64 = 0.7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatModerationTarget {
    pub chat_id: String,
    pub message_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub is_channel: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatModerationSelection {
    pub report_spam: bool,
    pub delete_all_from_user: bool,
    pub ban: bool,
    pub restrict: bool,
}

/// which rows the sheet offers, same rights the server checks per action read live
/// so a right taken away while the sheet is open leaves it at once
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatModerationOffers {
    pub delete_all: bool,
    pub ban: bool,
    pub restrict: bool,
}

impl ChatModerationOffers {
    pub fn current(target: Option<&ChatModerationTarget>, cx: &mut App) -> ChatModerationOffers {
        let Some(target) = target else {
            return ChatModerationOffers::default();
        };
        let services = ChatServicesStore::shared(cx);
        let Some(chat) = services.read(cx).chat_info(&target.chat_id, cx) else {
            return ChatModerationOffers::default();
        };
        ChatModerationOffers {
            delete_all: ChatModerationRights::allows(&chat, ChatPerm::MANAGE_MESSAGES, cx),
            ban: ChatModerationRights::allows(&chat, ChatPerm::BAN_MEMBERS, cx),
            restrict: !target.is_channel && ChatModerationRights::allows(&chat, ChatPerm::MUTE_MEMBERS, cx),
        }
    }

    pub fn filtering(&self, selection: ChatModerationSelection) -> ChatModerationSelection {
        let mut picked = selection;
        picked.delete_all_from_user = picked.delete_all_from_user && self.delete_all;
        picked.ban = picked.ban && self.ban;
        picked.restrict = picked.restrict && self.restrict;
        picked
    }
}

type ChatModerationConfirm = Rc<dyn Fn(ChatModerationTarget, ChatModerationSelection, &mut App)>;

/// presentation of the sheet, held outside the chat that opens it
/// the root draws it so its frames never dirty the conversation under it
pub struct ChatModerationSheetStore {
    target: Option<ChatModerationTarget>,
    on_confirm: Option<ChatModerationConfirm>,
}

impl ChatModerationSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { target: None, on_confirm: None }
    }

    pub fn present(
        &mut self,
        target: ChatModerationTarget,
        on_confirm: impl Fn(ChatModerationTarget, ChatModerationSelection, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.target = Some(target);
        self.on_confirm = Some(Rc::new(on_confirm));
        cx.notify();
    }
}

/// root layer of the moderation sheet, the sheet draws in its own overlay and this view
/// takes no click
pub struct ChatModerationSheetHost {
    store: Entity<ChatModerationSheetStore>,
    _store_observation: Subscription,
}

impl ChatModerationSheetHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = ChatModerationSheetStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self { store, _store_observation: store_observation }
    }
}

impl Render for ChatModerationSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_confirm = self.store.read(cx).on_confirm.clone();
        div().absolute().inset_0().child(ChatModerationSheet::new(
            "chat-moderation-sheet",
            Binding::entity(
                self.store.clone(),
                |store: &ChatModerationSheetStore| store.target.clone(),
                |store, value, cx| {
                    store.target = value;
                    cx.notify();
                },
            ),
            move |target, selection, cx| {
                if let Some(on_confirm) = on_confirm.as_ref() {
                    on_confirm(target, selection, cx);
                }
            },
        ))
    }
}

#[derive(IntoElement)]
pub struct ChatModerationSheet {
    id: ElementId,
    target: Binding<Option<ChatModerationTarget>>,
    on_confirm: Rc<dyn Fn(ChatModerationTarget, ChatModerationSelection, &mut App)>,
}

impl ChatModerationSheet {
    pub fn new(
        id: impl Into<ElementId>,
        target: Binding<Option<ChatModerationTarget>>,
        on_confirm: impl Fn(ChatModerationTarget, ChatModerationSelection, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), target, on_confirm: Rc::new(on_confirm) }
    }
}

impl RenderOnce for ChatModerationSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let selection = window.use_keyed_state((self.id.clone(), "selection"), cx, |_, _| {
            ChatModerationSelection::default()
        });

        let presented_read = self.target.clone();
        let presented_write = self.target.clone();
        let is_bottom_sheet = Binding::new(
            move |cx: &App| presented_read.get(cx).is_some(),
            move |value: bool, window: &mut Window, cx: &mut App| {
                if !value {
                    presented_write.set(None, window, cx);
                }
            },
        );

        let body_target = self.target.clone();
        let closing = self.target.clone();
        let on_close: Rc<dyn Fn(&mut Window, &mut App)> =
            Rc::new(move |window: &mut Window, cx: &mut App| closing.set(None, window, cx));
        let body_selection = selection.clone();
        let footer_target = self.target.clone();
        let on_confirm = self.on_confirm.clone();
        BottomSheetUi::new(
            (self.id.clone(), "sheet"),
            is_bottom_sheet,
            move |_: &mut Window, cx: &mut App| {
                let target = body_target.get(cx);
                let offers = ChatModerationOffers::current(target.as_ref(), cx);
                let appearing = body_selection.clone();
                ChatModerationSheetBody {
                    target,
                    offers,
                    selection: body_selection.clone(),
                    theme: rise_ui::theme(cx).clone(),
                    on_close: on_close.clone(),
                }
                .on_appear("chat-moderation-sheet-appear", move |_, cx| {
                    appearing.update(cx, |selection, cx| {
                        *selection = ChatModerationSelection::default();
                        cx.notify();
                    });
                })
            },
            move |_: &mut Window, cx: &mut App| {
                let target = footer_target.clone();
                let selection = selection.clone();
                let on_confirm = on_confirm.clone();
                ChatModerationFooter {
                    theme: rise_ui::theme(cx).clone(),
                    on_continue: Rc::new(move |window: &mut Window, cx: &mut App| {
                        let Some(picked_target) = target.get(cx) else {
                            return;
                        };
                        let chosen = *selection.read(cx);
                        let picked = ChatModerationOffers::current(Some(&picked_target), cx).filtering(chosen);
                        target.set(None, window, cx);
                        let _ = haptics::notification(NotificationKind::Success);
                        on_confirm(picked_target, picked, cx);
                    }),
                }
            },
        )
        .snap_fractions(vec![SHEET_SNAP_FRACTION])
        .fits_content(true)
        .sheet_background(Some(background))
        .footer_background(Some(background))
    }
}

#[derive(IntoElement)]
struct ChatModerationSheetBody {
    target: Option<ChatModerationTarget>,
    offers: ChatModerationOffers,
    selection: Entity<ChatModerationSelection>,
    theme: AppTheme,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ChatModerationSheetBody {
    fn author_name(&self) -> String {
        let name = self.target.as_ref().map_or("", |target| target.sender_name.as_str());
        if character_count(name) > AUTHOR_NAME_LIMIT {
            format!("{}\u{2026}", character_prefix(name, AUTHOR_NAME_LIMIT))
        } else {
            name.to_owned()
        }
    }

    fn row_separator(&self) -> Div {
        div()
            .w_full()
            .pl(px(52.0))
            .child(div().w_full().h(px(0.5)).bg(self.theme.border._200.opacity(0.55)))
    }

    fn option(
        &self,
        id: &'static str,
        is_on: Binding<bool>,
        icon: &'static str,
        tint: Hsla,
        text: String,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let is_selected = is_on.get(cx);
        let theme = &self.theme;
        plain(id)
            .w_full()
            .on_press(move |window: &mut Window, cx: &mut App| {
                let _ = haptics::impact(ImpactStyle::Light);
                is_on.toggle(window, cx);
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .bg(theme.bg._200)
                    .child(
                        Shape::continuous(8.0).round(
                            div()
                                .flex_none()
                                .size(px(29.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(tint.opacity(0.16))
                                .children(IconUi::sized(icon, px(14.0), tint, false)),
                        ),
                    )
                    .child(div().min_w_0().child(MainText::new(text).px(15.0).color(theme.text.primary).line_limit(1)))
                    .child(spacer_min_w(8.0))
                    .child(ChatModerationOptionMark {
                        id: ElementId::from(id),
                        is_on: is_selected,
                        on_color: theme.primary._100,
                        off_color: theme.text.secondary.opacity(0.5),
                    }),
            )
    }

    fn selection_binding(
        &self,
        get: fn(&ChatModerationSelection) -> bool,
        set: fn(&mut ChatModerationSelection, bool),
    ) -> Binding<bool> {
        let read = self.selection.clone();
        let write = self.selection.clone();
        Binding::new(
            move |cx: &App| get(read.read(cx)),
            move |value: bool, _: &mut Window, cx: &mut App| {
                write.update(cx, |selection, cx| {
                    set(selection, value);
                    cx.notify();
                });
            },
        )
    }
}

impl RenderOnce for ChatModerationSheetBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let author_name = self.author_name();
        let offers = self.offers;

        let mut rows = vstack(0.0).w_full().items_stretch().bg(theme.bg._200).child(self.option(
            "moderation-report-spam",
            self.selection_binding(|selection| selection.report_spam, |selection, value| selection.report_spam = value),
            "exclamationmark.bubble",
            theme.semantic.error_100,
            tr("moderation_report_spam"),
            cx,
        ));
        if offers.delete_all {
            rows = rows.child(self.row_separator()).child(self.option(
                "moderation-delete-all",
                self.selection_binding(
                    |selection| selection.delete_all_from_user,
                    |selection, value| selection.delete_all_from_user = value,
                ),
                "trash",
                theme.semantic.error_100,
                format!("{} {}", tr("moderation_delete_all_from"), author_name),
                cx,
            ));
        }
        if offers.ban {
            rows = rows.child(self.row_separator()).child(self.option(
                "moderation-ban",
                self.selection_binding(|selection| selection.ban, |selection, value| selection.ban = value),
                "hand.raised",
                theme.semantic.error_100,
                format!("{} {}", tr("moderation_ban"), author_name),
                cx,
            ));
        }
        if offers.restrict {
            rows = rows.child(self.row_separator()).child(self.option(
                "moderation-restrict",
                self.selection_binding(|selection| selection.restrict, |selection, value| selection.restrict = value),
                "lock",
                theme.text.primary,
                tr("moderation_restrict"),
                cx,
            ));
        }

        let on_close = self.on_close.clone();
        vstack(0.0)
            .w_full()
            .items_stretch()
            .child(
                SheetHeaderBar::new("chat-moderation-header", tr("moderation_title_one"), move |window, cx| {
                    on_close(window, cx)
                })
                .right_system_image(None),
            )
            .child(
                vstack(0.0)
                    .w_full()
                    .pb(px(18.0))
                    .child(MainText::new(tr("moderation_subtitle")).px(13.0).color(theme.text.secondary)),
            )
            .child(div().w_full().px(px(14.0)).child(Shape::continuous(16.0).round(rows)))
            .child(div().w_full().h(px(12.0)))
    }
}

#[derive(IntoElement)]
struct ChatModerationOptionMark {
    id: ElementId,
    is_on: bool,
    on_color: Hsla,
    off_color: Hsla,
}

impl RenderOnce for ChatModerationOptionMark {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_on = self.is_on;
        let progress = window.use_keyed_state((self.id.clone(), "mark"), cx, move |_, _| {
            Animated::new(if is_on { 1.0_f32 } else { 0.0_f32 })
        });
        let animating = progress.clone();
        on_change(
            (self.id.clone(), "mark-change"),
            is_on,
            false,
            window,
            cx,
            move |_, value, _, cx| {
                let target = if *value { 1.0_f32 } else { 0.0_f32 };
                animating.update(cx, |progress, cx| {
                    progress.set_in(target, spring(OPTION_MARK_RESPONSE, OPTION_MARK_DAMPING), RiseClock::monotonic(), cx);
                    cx.notify();
                });
            },
        );
        let shown = progress.read(cx).frame_value(RiseClock::monotonic(), window).clamp(0.0, 1.0);
        div()
            .flex_none()
            .relative()
            .size(px(OPTION_MARK_SIDE))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1.0 - shown)
                    .children(IconUi::sized("circle", px(OPTION_MARK_SIDE), self.off_color, false)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(shown)
                    .children(IconUi::sized("checkmark.circle.fill", px(OPTION_MARK_SIDE), self.on_color, true)),
            )
    }
}

#[derive(IntoElement)]
struct ChatModerationFooter {
    theme: AppTheme,
    on_continue: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for ChatModerationFooter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_continue = self.on_continue.clone();
        div().w_full().px(px(14.0)).pt(px(8.0)).pb(px(6.0)).child(
            ButtonUi::new("chat-moderation-continue")
                .background_color(self.theme.primary._100)
                .height(50.0)
                .on_press(move |window: &mut Window, cx: &mut App| on_continue(window, cx))
                .child(
                    MainText::new(tr("moderation_continue"))
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.theme.content_on(self.theme.primary._100)),
                ),
        )
    }
}
