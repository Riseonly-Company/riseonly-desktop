use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, Hsla, SharedString, Subscription, Window, rgb};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;
use rise_ui::theme;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::gifts::engine::rise_gifts_engine_models::{RiseGiftAppearanceExt, RiseGiftItemKind, RiseGiftWornDTO};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_services::gift_appearance_services::GiftAppearanceServicesStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_market_types::GiftsAppearanceState;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

#[derive(IntoElement)]
pub struct CustomizationSettings {
    id: ElementId,
}

impl Default for CustomizationSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomizationSettings {
    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("customization-settings")) }
    }
}

impl RenderOnce for CustomizationSettings {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let back_store = nav_store.clone();
        SettingsWrapperUi::new(tr("customization_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(CustomizationSettingsBody { id: named_child(&self.id, "body"), nav_store })
    }
}

#[derive(IntoElement)]
struct CustomizationSettingsBody {
    id: ElementId,
    nav_store: Option<Entity<NavigationStore>>,
}

impl CustomizationSettingsBody {
    fn route_button(
        &self,
        id: &'static str,
        icon: &'static str,
        icon_background: Hsla,
        text: String,
        anchor: Option<RiseGuideAnchor>,
        route: RootRoute,
    ) -> GroupedBtn {
        let nav_store = self.nav_store.clone();
        GroupedBtn::new(id, text).icon(icon).icon_background(icon_background).guide_anchor(anchor).callback(
            move |_, cx| {
                if let Some(nav_store) = &nav_store {
                    let route = route.clone();
                    nav_store.update(cx, |nav_store, cx| {
                        nav_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                    });
                }
            },
        )
    }

    fn hex(value: u32) -> Hsla {
        rgb(value).into()
    }
}

impl RenderOnce for CustomizationSettingsBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = theme(cx).clone();

        let first = GroupedBtns::new(
            named_child(&self.id, "first"),
            vec![
                self.route_button(
                    "theme",
                    "paintpalette",
                    Self::hex(0x00A9BB),
                    tr("settings_customization_your_theme"),
                    Some(RiseGuideAnchor::CustomizationTheme),
                    RootRoute::ThemeSettings,
                ),
                self.route_button(
                    "my_themes",
                    "swatchpalette",
                    Self::hex(0xE17055),
                    tr("settings_my_themes"),
                    None,
                    RootRoute::ThemesList,
                ),
                self.route_button(
                    "cwp",
                    "photo.on.rectangle.angled",
                    Self::hex(0x0083E0),
                    tr("settings_customization_chat_wallpapers"),
                    Some(RiseGuideAnchor::CustomizationWallpapers),
                    RootRoute::ChatWallpapersSettings,
                ),
                self.route_button(
                    "tab_bar",
                    "dock.rectangle",
                    Self::hex(0x5E5CE6),
                    tr("settings_tab_bar"),
                    None,
                    RootRoute::TabBarSettings,
                ),
            ],
        );

        let behavior = GroupedBtns::new(
            named_child(&self.id, "behavior"),
            vec![
                self.route_button(
                    "energy",
                    "bolt.batteryblock",
                    Self::hex(0xFDCB6E),
                    tr("settings_energy"),
                    Some(RiseGuideAnchor::CustomizationEnergy),
                    RootRoute::EnergySavingSettings,
                ),
                self.route_button(
                    "launch_animation",
                    "play.circle",
                    SettingsIconPalette::customization(),
                    tr("settings_launch_animation"),
                    None,
                    RootRoute::LaunchAnimationSettings,
                ),
                self.route_button(
                    "textsize",
                    "textformat.size",
                    Self::hex(0x00B894),
                    tr("settings_text_size"),
                    None,
                    RootRoute::TextSizeSettings,
                ),
            ],
        );

        let palette = GroupedBtns::new(
            named_child(&self.id, "palette"),
            vec![
                self.route_button(
                    "bg",
                    "rectangle.fill",
                    Self::hex(0x0984E3),
                    tr("settings_bg_color"),
                    None,
                    RootRoute::BgColorSettings,
                ),
                self.route_button(
                    "block",
                    "square.grid.2x2.fill",
                    Self::hex(0x00B894),
                    tr("settings_block_color"),
                    None,
                    RootRoute::BlockColorSettings,
                ),
                self.route_button(
                    "btn",
                    "capsule.fill",
                    Self::hex(0x6C5CE7),
                    tr("settings_btn_color"),
                    Some(RiseGuideAnchor::CustomizationButtonColor),
                    RootRoute::BtnColorSettings,
                ),
                self.route_button(
                    "primary",
                    "circle.lefthalf.filled",
                    Self::hex(0xE84393),
                    tr("settings_primary_color"),
                    None,
                    RootRoute::PrimaryColorSettings,
                ),
                self.route_button(
                    "text",
                    "textformat",
                    Self::hex(0x00A9BB),
                    tr("settings_text_color"),
                    None,
                    RootRoute::TextColorSettings,
                ),
                self.route_button(
                    "secondaryText",
                    "textformat.alt",
                    Self::hex(0x5AC8FA),
                    tr("settings_secondary_text_color"),
                    None,
                    RootRoute::SecondaryTextColorSettings,
                ),
                self.route_button(
                    "border",
                    "square.dashed",
                    Self::hex(0x8E8E93),
                    tr("settings_border_color"),
                    None,
                    RootRoute::BorderColorSettings,
                ),
            ],
        )
        .title(tr("settings_color_palette").to_uppercase());

        let reset = GroupedBtns::new(
            named_child(&self.id, "reset"),
            vec![
                GroupedBtn::new("custom_reset_full", tr("settings_reset_full_theme"))
                    .icon("arrow.counterclockwise")
                    .icon_background(SettingsIconPalette::reset())
                    .guide_anchor(RiseGuideAnchor::CustomizationResetTheme)
                    .show_chevron(false)
                    .callback(|_, cx| {
                        ThemeStore::shared(cx).update(cx, |store, cx| {
                            let palette = store.reference_builtin_palette();
                            store.apply(palette, cx);
                        });
                        GuideSignalBus::emit(RiseGuideSignal::ThemeReset);
                    }),
            ],
        );

        scroll_view(named_child(&self.id, "scroll"))
            .size_full()
            .bg(theme.bg._100)
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(20.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(first)
                    .child(CustomizationAppearanceGroup { id: named_child(&self.id, "appearance") })
                    .child(behavior)
                    .child(palette)
                    .child(reset)
                    .child(spacer_min_h(80.0))
                    .music_mini_player_bottom_padding(0.0, 0.0),
            )
    }
}

struct CustomizationAppearanceGroupState {
    _subscriptions: [Subscription; 2],
}

impl CustomizationAppearanceGroupState {
    fn new(cx: &mut Context<Self>) -> Self {
        let appearance = GiftAppearanceServicesStore::shared(cx);
        let gifts = GiftsServicesStore::shared(cx);
        Self { _subscriptions: [cx.observe(&appearance, |_, _, cx| cx.notify()), cx.observe(&gifts, |_, _, cx| cx.notify())] }
    }
}

#[derive(IntoElement)]
struct CustomizationAppearanceGroup {
    id: ElementId,
}

impl CustomizationAppearanceGroup {
    /// catalog item name in the app language first, a pushed instance carries the default language name
    fn worn_title(worn: Option<&RiseGiftWornDTO>, names: &GiftsAppearanceState, cx: &mut App) -> String {
        let Some(worn) = worn else {
            return tr("customization_appearance_none");
        };
        GiftsServicesStore::shared(cx)
            .read(cx)
            .localized_name(worn.gift_id, cx)
            .or_else(|| names.worn_name(Some(worn)))
            .unwrap_or_default()
    }

    fn icon(kind: RiseGiftItemKind) -> &'static str {
        match kind {
            RiseGiftItemKind::AvatarFrame => "person.crop.circle",
            RiseGiftItemKind::Nameplate => "person.text.rectangle",
            RiseGiftItemKind::ProfileEffect | RiseGiftItemKind::Item => "sparkles",
        }
    }

    fn tint(kind: RiseGiftItemKind) -> Hsla {
        match kind {
            RiseGiftItemKind::AvatarFrame => rgb(0xA45CFF).into(),
            RiseGiftItemKind::Nameplate => rgb(0xF5A623).into(),
            RiseGiftItemKind::ProfileEffect | RiseGiftItemKind::Item => rgb(0x3F8EF7).into(),
        }
    }
}

impl RenderOnce for CustomizationAppearanceGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let _state = window
            .use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| CustomizationAppearanceGroupState::new(cx));
        let own = GiftAppearanceServicesStore::shared(cx).update(cx, |store, cx| store.own_appearance(cx));
        let names = GiftsServicesStore::shared(cx).read(cx).appearance().clone();

        let mut items = Vec::new();
        for kind in RiseGiftItemKind::cosmetics() {
            let Some(key) = kind.appearance_title_key() else {
                continue;
            };
            let worn_title = match &own {
                Some(own) => Self::worn_title(own.worn(kind), &names, cx),
                None => String::new(),
            };
            items.push(
                GroupedBtn::new(format!("appearance_{}", kind.slug()), tr(key))
                    .icon(Self::icon(kind))
                    .icon_background(Self::tint(kind))
                    .trailing(worn_title)
                    .callback(move |_, cx| {
                        GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_wardrobe(kind, cx));
                    }),
            );
        }

        GroupedBtns::new(named_child(&self.id, "group"), items)
            .title(tr("customization_appearance_title").to_uppercase())
            .end_group_title(tr("customization_appearance_note"))
            .on_appear(named_child(&self.id, "task"), |_, cx| {
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_appearance(cx));
            })
    }
}
