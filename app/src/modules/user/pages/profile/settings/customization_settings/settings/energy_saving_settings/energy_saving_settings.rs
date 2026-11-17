use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, px};
use rise_energy::rise_energy_saving::{
    RiseEnergySavingFeature, RiseEnergySavingPolicy, RiseEnergySavingSettings, RiseEnergySavingStore,
};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, color};
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::switch_ui::SwitchUi;

use rise_canon::RiseGuideAnchor;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};

struct FeatureRow {
    feature: RiseEnergySavingFeature,
    icon: &'static str,
    color: Hsla,
    title_key: &'static str,
    subtitle_key: &'static str,
}

fn feature_rows() -> [FeatureRow; 7] {
    [
        FeatureRow {
            feature: RiseEnergySavingFeature::AnimatedStickers,
            icon: "face.smiling.inverse",
            color: SettingsIconPalette::stickers(),
            title_key: "settings_energy_feature_stickers",
            subtitle_key: "settings_energy_feature_stickers_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::AnimatedEmoji,
            icon: "sparkles",
            color: color("#FF9500"),
            title_key: "settings_energy_feature_emoji",
            subtitle_key: "settings_energy_feature_emoji_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::AnimatedGifs,
            icon: "photo.stack",
            color: color("#5AC8FA"),
            title_key: "settings_energy_feature_gifs",
            subtitle_key: "settings_energy_feature_gifs_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::LoopAnimations,
            icon: "repeat",
            color: color("#34C759"),
            title_key: "settings_energy_feature_loop",
            subtitle_key: "settings_energy_feature_loop_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::ChatWallpaperMotion,
            icon: "photo.on.rectangle.angled",
            color: color("#0083E0"),
            title_key: "settings_energy_feature_wallpaper",
            subtitle_key: "settings_energy_feature_wallpaper_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::InterfaceEffects,
            icon: "wand.and.stars",
            color: color("#6C5CE7"),
            title_key: "settings_energy_feature_effects",
            subtitle_key: "settings_energy_feature_effects_hint",
        },
        FeatureRow {
            feature: RiseEnergySavingFeature::MediaPrefetch,
            icon: "arrow.down.circle",
            color: color("#8E8E93"),
            title_key: "settings_energy_feature_prefetch",
            subtitle_key: "settings_energy_feature_prefetch_hint",
        },
    ]
}

struct EnergySavingSettingsState {
    _subscriptions: Vec<Subscription>,
}

impl EnergySavingSettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = RiseEnergySavingStore::shared(cx);
        Self { _subscriptions: vec![cx.observe(&store, |_, _, cx| cx.notify())] }
    }
}

fn set_threshold_percent(percent: i64, cx: &mut App) {
    RiseEnergySavingStore::shared(cx).update(cx, |store, cx| store.set_threshold(percent, cx));
}

fn set_threshold(index: f64, cx: &mut App) {
    let stops = RiseEnergySavingPolicy::THRESHOLD_BREAKPOINTS;
    let position = (index.round().max(0.0) as usize).min(stops.len() - 1);
    set_threshold_percent(stops[position], cx);
}

#[derive(IntoElement)]
pub struct EnergySavingSettings;

impl EnergySavingSettings {
    pub fn new() -> Self {
        Self
    }

    fn status_group(store: &RiseEnergySavingStore) -> GroupedBtns {
        let active = store.is_active();
        let follows_system = store.settings().threshold_percent > RiseEnergySavingPolicy::OFF_THRESHOLD_PERCENT;
        let subtitle = if active && store.system_low_power_mode() {
            tr("settings_energy_status_system")
        } else if let Some(percent) = store.battery_percent() {
            tr_args("settings_energy_status_battery", &[FormatArg::Int(percent)])
        } else {
            tr("settings_energy_status_unknown")
        };
        let status = GroupedBtn::new(
            "energy_status",
            if active { tr("settings_energy_status_on") } else { tr("settings_energy_status_off") },
        )
        .icon(if active { "bolt.batteryblock.fill" } else { "bolt.batteryblock" })
        .icon_background(if active { color("#34C759") } else { color("#8E8E93") })
        .subtitle(subtitle)
        .show_chevron(false);
        let group = GroupedBtns::new("energy-status-group", vec![status]);
        if follows_system { group.end_group_title(tr("settings_energy_status_hint")) } else { group }
    }

    fn threshold_group(threshold: i64, theme: &AppTheme) -> GroupedBtns {
        GroupedBtns::new(
            "energy-threshold-group",
            vec![
                GroupedBtn::new("energy_threshold", "")
                    .guide_anchor(RiseGuideAnchor::EnergyThreshold)
                    .content_slot(Self::threshold_card(threshold, theme)),
            ],
        )
        .title(tr("settings_energy_auto_title").to_uppercase())
        .end_group_title(tr("settings_energy_auto_hint"))
    }

    fn threshold_card(threshold: i64, theme: &AppTheme) -> impl IntoElement + use<> {
        let last_stop = (RiseEnergySavingPolicy::THRESHOLD_BREAKPOINTS.len() - 1) as f64;
        vstack(10.0)
            .w_full()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .child(MainText::new(tr("settings_energy_auto_when")).px(15.0))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(Self::threshold_label(threshold))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .primary(true)
                            .monospaced_digits(true),
                    ),
            )
            .child(SteppedSliderUi::new(
                "energy-threshold-slider",
                RiseEnergySavingPolicy::threshold_index(threshold) as f64,
                0.0..=last_stop,
                1.0,
                |index, _, cx| set_threshold(index, cx),
                |index, _, cx| set_threshold(index, cx),
            ))
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .child(MainText::new(tr("settings_energy_off")).px(12.0).color(theme.text.secondary))
                    .child(spacer())
                    .child(MainText::new(tr("settings_energy_always")).px(12.0).color(theme.text.secondary)),
            )
    }

    fn preset_group(threshold: i64, theme: &AppTheme) -> GroupedBtns {
        let items = RiseEnergySavingPolicy::THRESHOLD_BREAKPOINTS
            .iter()
            .copied()
            .map(|percent| {
                GroupedBtn::new(format!("energy_preset_{percent}"), Self::threshold_label(percent))
                    .leading_slot(Self::selection_indicator(threshold == percent, theme))
                    .accessibility_selected(threshold == percent)
                    .show_chevron(false)
                    .callback(move |_, cx| {
                        let _ = selection_changed();
                        set_threshold_percent(percent, cx);
                    })
            })
            .collect();
        GroupedBtns::new("energy-preset-group", items)
    }

    fn selection_indicator(selected: bool, theme: &AppTheme) -> impl IntoElement + use<> {
        let indicator = div()
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .flex()
            .flex_none()
            .items_center()
            .justify_center();
        if selected {
            indicator.child(
                div()
                    .size(px(21.0))
                    .circle()
                    .bg(theme.primary._100)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("checkmark", px(12.0), theme.bg._200, false)),
            )
        } else {
            indicator.children(IconUi::sized("circle", px(21.0), theme.border._300, false))
        }
    }

    pub fn threshold_label(percent: i64) -> String {
        if percent <= RiseEnergySavingPolicy::OFF_THRESHOLD_PERCENT {
            return tr("settings_energy_off");
        }
        if percent >= RiseEnergySavingPolicy::ALWAYS_THRESHOLD_PERCENT {
            return tr("settings_energy_always");
        }
        tr_args("settings_energy_below_percent", &[FormatArg::Int(percent)])
    }

    fn features_group(store_entity: &Entity<RiseEnergySavingStore>) -> GroupedBtns {
        let rows = feature_rows();
        let first_feature = rows[0].feature;
        let items = rows
            .into_iter()
            .map(|row| {
                let feature = row.feature;
                let binding = Binding::entity(
                    store_entity.clone(),
                    move |store: &RiseEnergySavingStore| store.is_reduced(feature),
                    move |store: &mut RiseEnergySavingStore, reduced, cx| store.set_reduced(feature, reduced, cx),
                );
                let toggled = store_entity.clone();
                let button = GroupedBtn::new(format!("energy_feature_{}", feature.raw_value()), tr(row.title_key))
                    .icon(row.icon)
                    .icon_background(row.color)
                    .subtitle(tr(row.subtitle_key))
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new(("energy-feature-switch", feature as u64), binding))
                    .callback(move |_, cx| {
                        toggled.update(cx, |store, cx| {
                            let reduced = store.is_reduced(feature);
                            store.set_reduced(feature, !reduced, cx);
                        });
                    });
                if feature == first_feature { button.guide_anchor(RiseGuideAnchor::EnergyFirstFeature) } else { button }
            })
            .collect();
        GroupedBtns::new("energy-features-group", items)
            .title(tr("settings_energy_reduce_title").to_uppercase())
            .end_group_title(tr("settings_energy_reduce_hint"))
    }
}

impl RenderOnce for EnergySavingSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let _state = window.use_keyed_state("energy-saving-settings-state", cx, |_, cx| EnergySavingSettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let store_entity = RiseEnergySavingStore::shared(cx);
        let store = store_entity.read(cx);
        let threshold = store.settings().threshold_percent;
        let is_default = *store.settings() == RiseEnergySavingSettings::default();
        let status_group = Self::status_group(store);

        let reset = GroupedBtn::new("energy_reset", tr("settings_reset_default"))
            .icon("arrow.counterclockwise")
            .icon_background(SettingsIconPalette::reset())
            .disabled(is_default)
            .show_chevron(false)
            .callback(|_, cx| {
                let _ = selection_changed();
                RiseEnergySavingStore::shared(cx).update(cx, |store, cx| store.reset_to_defaults(cx));
            });

        let sections = vstack(20.0)
            .w_full()
            .pt(px(12.0))
            .child(status_group)
            .child(Self::threshold_group(threshold, &theme))
            .child(Self::preset_group(threshold, &theme))
            .child(Self::features_group(&store_entity))
            .child(GroupedBtns::new("energy-reset-group", vec![reset]))
            .child(spacer_min_h(80.0));

        SettingsWrapperUi::new(tr("settings_energy"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("energy-saving-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(sections),
        )
    }
}
