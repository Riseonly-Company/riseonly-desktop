use std::rc::Rc;

use gpui::{App, Entity, FontWeight, IntoElement, RenderOnce, Window};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText, TextAlignT};

#[derive(IntoElement)]
pub struct MediaPickerPermissionDeniedView {
    pub theme_store: Entity<ThemeStore>,
    pub on_open_settings: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_choose_file: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MediaPickerPermissionDeniedView {
    pub fn new(
        theme_store: Entity<ThemeStore>,
        on_open_settings: impl Fn(&mut Window, &mut App) + 'static,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            theme_store,
            on_open_settings: Rc::new(on_open_settings),
            on_cancel: Rc::new(on_cancel),
            on_choose_file: None,
        }
    }

    pub fn on_choose_file(mut self, on_choose_file: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_choose_file = on_choose_file;
        self
    }
}

impl RenderOnce for MediaPickerPermissionDeniedView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme_store.read(cx).current_theme().clone();
        let on_cancel = self.on_cancel.clone();
        let on_open_settings = self.on_open_settings.clone();

        let choose_file = self.on_choose_file.clone().map(|on_choose_file| {
            let label = div()
                .flex()
                .items_center()
                .px(px(18.0))
                .h(px(38.0))
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text.primary)
                .child(tr("media_picker_choose_file"))
                .liquid_chrome_capsule_with(0.18, true, None, false);
            plain("media-picker-permission-choose-file")
                .on_press(move |window, cx| on_choose_file(window, cx))
                .child(label)
        });

        let cancel_label = div()
            .flex()
            .items_center()
            .px(px(18.0))
            .h(px(38.0))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text.primary)
            .child(tr("media_picker_cancel"))
            .liquid_chrome_capsule_with(0.18, true, None, false);

        let open_settings_label = div()
            .flex()
            .items_center()
            .px(px(18.0))
            .h(px(38.0))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.content_on(theme.primary._100))
            .capsule()
            .bg(theme.primary._100)
            .child(tr("media_picker_open_settings"));

        vstack(14.0)
            .size_full()
            .justify_center()
            .pb(px(36.0))
            .children(IconUi::sized("photo.on.rectangle", px(42.0), theme.text.secondary, false))
            .child(
                MainText::new(tr("media_picker_no_access_title"))
                    .px(17.0)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div().px(px(32.0)).child(
                    MainText::new(tr("media_picker_no_access_subtitle"))
                        .px(13.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(4),
                ),
            )
            .child(
                hstack(12.0)
                    .pt(px(6.0))
                    .child(
                        plain("media-picker-permission-cancel")
                            .on_press(move |window, cx| on_cancel(window, cx))
                            .child(cancel_label),
                    )
                    .child(
                        plain("media-picker-permission-open-settings")
                            .on_press(move |window, cx| on_open_settings(window, cx))
                            .child(open_settings_label),
                    ),
            )
            .children(choose_file)
    }
}
