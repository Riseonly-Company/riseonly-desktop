use gpui::{
    BoxShadow, Context, Entity, FontWeight, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, hsla, point, px,
};
use rise_i18n::{tr, tr_named};
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;

use super::app_update_store::{AppUpdatePhase, AppUpdateStore};

const WIDTH: f32 = 240.0;

pub struct AppUpdatePill {
    store: Entity<AppUpdateStore>,
    _observation: Subscription,
}

impl AppUpdatePill {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = AppUpdateStore::shared(cx);
        store.update(cx, |store, cx| store.start(cx));
        let observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _observation: observation,
        }
    }
}

impl Render for AppUpdatePill {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let title = match store.phase() {
            AppUpdatePhase::Idle => return div().into_any_element(),
            AppUpdatePhase::Available => tr_named(
                "desktop_update_button",
                &[("count", &store.count().to_string())],
            ),
            AppUpdatePhase::Downloading(fraction) => tr_named(
                "desktop_update_downloading",
                &[("percent", &((fraction * 100.0).floor() as i64).to_string())],
            ),
            AppUpdatePhase::Restarting => tr("desktop_update_restarting"),
            AppUpdatePhase::Failed => tr("desktop_update_failed"),
        };
        let theme = rise_ui::theme(cx);
        let store = self.store.clone();
        div()
            .w(px(WIDTH))
            .rounded_full()
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.24),
                offset: point(px(0.0), px(4.0)),
                blur_radius: px(16.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                ButtonUi::new("app-update-pill")
                    .background_color(theme.primary._100)
                    .on_press(move |_, cx| store.update(cx, |store, cx| store.install(cx)))
                    .child(
                        MainText::new(title)
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .monospaced_digits(true)
                            .number_of_lines(1),
                    ),
            )
            .into_any_element()
    }
}
