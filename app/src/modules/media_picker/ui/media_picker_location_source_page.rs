use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, Window, white};
use rise_i18n::{LanguageStore, tr};
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::{MainText, TextAlignT};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::ui::qr_code::rise_phone_handoff_code::RisePhoneHandoffCode;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::components::pins::map_self_pin_layer::MapSelfProfile;
use crate::modules::map::components::rise_map_view::RiseMapView;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;

#[derive(IntoElement)]
pub struct MediaPickerLocationSourcePage {
    id: ElementId,
    pub on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MediaPickerLocationSourcePage {
    const ON_PHONE_CODE_SIDE: f32 = 148.0;

    pub fn new(id: impl Into<ElementId>, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_cancel: Rc::new(on_cancel) }
    }

    fn header(&self, theme: &AppTheme) -> Div {
        let on_cancel = self.on_cancel.clone();
        div().w_full().px(px(14.0)).py(px(10.0)).bg(theme.bg._100).child(
            zstack(Alignment::Center)
                .w_full()
                .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
                .child(
                    MainText::new(tr("chat_msg_location"))
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .child(
                    hstack(0.0)
                        .w_full()
                        .child(
                            MediaPickerHeaderIconButton::new("location-source-cancel", "xmark", move |window, cx| {
                                on_cancel(window, cx)
                            })
                            .font_size(15.0),
                        )
                        .child(spacer()),
                ),
        )
    }

    fn map(theme: &AppTheme, cx: &mut App) -> Div {
        let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
        let language_code = LanguageStore::active(cx).map(|store| store.code().to_owned()).unwrap_or_default();
        let mut map = RiseMapView::new("location-source-map", appearance, language_code);
        map.self_profile = MapInteractionsStore::own_map_profile(cx).map(|own| MapSelfProfile {
            id: own.user.id,
            name: own.user.name,
            logo: own.user.logo,
        });
        map.pin_palette = Some(MapPinPalette {
            primary: theme.primary._100,
            ring: white(),
            stale: theme.text.secondary,
            plate: theme.bg._100.opacity(0.92),
            plate_text: theme.text.primary,
            letter: AvatarPalette::letter_color(),
            low_battery: UIColor::system_red().resolved_color(theme.appearance),
        });
        div().w_full().px(px(16.0)).child(
            div()
                .w_full()
                .h(px(260.0))
                .clip_shape(Shape::continuous(12.0))
                .child(map),
        )
    }

    fn from_phone_note(theme: &AppTheme) -> Div {
        div().w_full().px(px(16.0)).flex().justify_center().child(
            div()
                .px(px(14.0))
                .py(px(10.0))
                .child(
                    hstack(6.0)
                        .children(IconUi::sized("iphone", px(16.0), theme.text.primary, false))
                        .child(
                            MainText::new(tr("map_location_from_phone"))
                                .px(13.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(2),
                        ),
                )
                .liquid_chrome_rounded_rectangle(22.0),
        )
    }
}

impl RenderOnce for MediaPickerLocationSourcePage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let map = Self::map(&theme, cx);
        vstack(0.0)
            .id(self.id.clone())
            .size_full()
            .bg(theme.bg._100)
            .child(self.header(&theme))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    scroll_view("location-source-scroll").child(
                        vstack(18.0)
                            .w_full()
                            .pt(px(10.0))
                            .pb(px(108.0))
                            .child(map)
                            .child(Self::from_phone_note(&theme))
                            .child(
                                vstack(10.0)
                                    .w_full()
                                    .px(px(24.0))
                                    .items_center()
                                    .child(
                                        MainText::new(tr("map_open_on_phone"))
                                            .px(13.0)
                                            .tac(TextAlignT::Center)
                                            .color(theme.text.secondary)
                                            .w_full(),
                                    )
                                    .child(RisePhoneHandoffCode::new(
                                        "location-source-on-phone",
                                        AppEnvironment::current().public_site_url().absolute_string().to_owned(),
                                        Self::ON_PHONE_CODE_SIDE,
                                    )),
                            ),
                    ),
                ),
            )
    }
}
