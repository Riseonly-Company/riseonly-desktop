use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, RenderOnce,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};
use rise_i18n::{FormatArg, tr, tr_args, tr_plural_with};
use rise_theme::ThemeStore;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationSheet};
use crate::core::widgets::overlay::premium_gift_celebration_store::{
    PremiumGiftCelebrationStoreGift, PremiumGiftCelebrationStore,
};

const PREMIUM_PRODUCT_NAME: &str = "Riseonly Premium";

const MARK_FRAME_SHARE: f32 = 0.83;

type OpenChat = Rc<dyn Fn(String, &mut Window, &mut App)>;
type SheetAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// mounted behind the interruptions gate
/// waits for a signed in account with a published session and never shows over a call
/// asks the server on appear and every foreground
pub struct PremiumGiftCelebrationHost {
    store: Entity<PremiumGiftCelebrationStore>,
    on_open_chat: OpenChat,
    _store_observation: Subscription,
    _foreground_observer: RiseSubscription,
    _foreground_signals: Task<()>,
}

impl PremiumGiftCelebrationHost {
    pub fn new(
        on_open_chat: impl Fn(String, &mut Window, &mut App) + 'static,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = PremiumGiftCelebrationStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        let (activations, mut inbox) = mpsc::unbounded::<()>();
        let foreground_observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = activations.unbounded_send(());
            });
        let foreground_signals = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let checked = this.update(cx, |host: &mut Self, cx: &mut Context<Self>| {
                    host.store.update(cx, |store, cx| store.schedule_check(cx));
                });
                if checked.is_err() {
                    break;
                }
            }
        });
        Self {
            store,
            on_open_chat: Rc::new(on_open_chat),
            _store_observation: store_observation,
            _foreground_observer: foreground_observer,
            _foreground_signals: foreground_signals,
        }
    }
}

impl Render for PremiumGiftCelebrationHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let read = self.store.clone();
        let write = self.store.clone();
        let done = self.store.clone();
        let open = self.store.clone();
        let appear = self.store.clone();
        let on_open_chat = self.on_open_chat.clone();
        PremiumGiftCelebrationSheet::new(
            Binding::new(
                move |cx| read.read(cx).is_visible(),
                move |is_presented, _, cx| {
                    if !is_presented {
                        write.update(cx, |store, cx| store.dismiss(cx));
                    }
                },
            ),
            self.store.read(cx).gift().cloned(),
            move |_, cx| done.update(cx, |store, cx| store.dismiss(cx)),
            move |window, cx| {
                let chat_id = open.update(cx, |store, cx| store.chat_with_giver(cx));
                open.update(cx, |store, cx| store.dismiss(cx));
                if let Some(chat_id) = chat_id {
                    on_open_chat(chat_id, window, cx);
                }
            },
        )
        .on_appear("premium_gift_celebration_appear", move |_, cx| {
            appear.update(cx, |store, cx| store.schedule_check(cx));
        })
    }
}

#[derive(IntoElement)]
pub struct PremiumGiftCelebrationSheet {
    is_presented: Binding<bool>,
    gift: Option<PremiumGiftCelebrationStoreGift>,
    on_done: SheetAction,
    on_open_chat: SheetAction,
}

impl PremiumGiftCelebrationSheet {
    pub fn new(
        is_presented: Binding<bool>,
        gift: Option<PremiumGiftCelebrationStoreGift>,
        on_done: impl Fn(&mut Window, &mut App) + 'static,
        on_open_chat: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            is_presented,
            gift,
            on_done: Rc::new(on_done),
            on_open_chat: Rc::new(on_open_chat),
        }
    }
}

impl RenderOnce for PremiumGiftCelebrationSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_done = self.on_done;
        let on_open_chat = self.on_open_chat;
        let gift = self.gift;
        div().size_full().child(PurchaseCelebrationSheet::new(
            PurchaseCelebration::Premium,
            self.is_presented,
            tr("subscription_success_continue"),
            move |window, cx| on_done(window, cx),
            |_, _| {
                LottieView::new(PremiumDiamond::ANIMATION)
                    .size(PurchaseCelebration::ICON_TIP_TO_TIP / MARK_FRAME_SHARE)
                    .looping(false)
                    .speed(0.0)
            },
            move |_, _| PremiumGiftCelebrationLines {
                gift: gift.clone(),
                on_open_chat: on_open_chat.clone(),
            },
        ))
    }
}

#[derive(IntoElement)]
struct PremiumGiftCelebrationLines {
    gift: Option<PremiumGiftCelebrationStoreGift>,
    on_open_chat: SheetAction,
}

impl PremiumGiftCelebrationLines {
    fn gift_line(gift: Option<&PremiumGiftCelebrationStoreGift>) -> String {
        let Some(gift) = gift else {
            return String::new();
        };
        let period = tr_plural_with("premium_gift_months", gift.months, FormatArg::from(gift.months));
        if let Some(name) = gift.payer_name.as_ref() {
            return tr_args("premium_gifted_from", &[FormatArg::from(name), FormatArg::from(period)]);
        }
        tr_args("premium_gifted_period", &[FormatArg::from(period)])
    }
}

impl RenderOnce for PremiumGiftCelebrationLines {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let column = div().flex().flex_col().items_center();
        if let Some(apology) = self.gift.as_ref().and_then(|gift| gift.tag_apology.as_ref()) {
            return column
                .child(
                    div().px_6().child(
                        MainText::new(tr("tag_apology_title"))
                            .px(22.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD),
                    ),
                )
                .child(
                    div().pt_2().px_6().child(
                        SecondaryText::new(tr_args(
                            "tag_apology_body",
                            &[FormatArg::from(apology.old_tag.as_str()), FormatArg::from(apology.new_tag.as_str())],
                        ))
                        .px(14.0)
                        .tac(TextAlignT::Center),
                    ),
                );
        }
        let on_open_chat = self.on_open_chat;
        column
            .child(
                div().px_6().child(
                    MainText::new(tr("premium_gifted_title"))
                        .px(22.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD),
                ),
            )
            .child(
                div().pt_2().child(
                    MainText::new(PREMIUM_PRODUCT_NAME)
                        .px(20.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD),
                ),
            )
            .child(
                div()
                    .pt_1p5()
                    .px_6()
                    .child(SecondaryText::new(Self::gift_line(self.gift.as_ref())).px(14.0).tac(TextAlignT::Center)),
            )
            .child(
                div().pt_2().child(
                    div()
                        .id("premium_gifted_open_chat")
                        .py_2()
                        .px_4()
                        .cursor_pointer()
                        .on_click(move |_, window, cx| on_open_chat(window, cx))
                        .child(
                            MainText::new(tr("premium_gifted_open_chat"))
                                .px(15.0)
                                .tac(TextAlignT::Center)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(primary),
                        ),
                ),
            )
    }
}
