use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, ListAlignment, ListState, SharedString, Subscription, Window, list,
    white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine};
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::components::map_cooling_card::MapCoolingCard;
use crate::modules::map::engine::rise_map_engine_models::RiseMapLoadState;
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepositoryList;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_privacy_repository::RiseMapPrivacySnapshot;
use crate::modules::map::pages::map_settings::map_merge_settings_page::MapMergeSettingsSection;
use crate::modules::map::shared::map_settings_section::MapSettingsSection;
use crate::modules::map::shared::map_settings_section::MapSettingsSectionPrivacy;
use crate::modules::map::stores::map_actions::map_privacy_types::{
    MapPrivacyAction, MapPrivacyKind, MapPrivacyListTarget, MapPrivacyRule,
};
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlace;
use crate::modules::map::stores::map_actions::map_types::{MapSettings, MapSettingsPatch, MapUser};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;
use crate::modules::notify::stores::notify_actions::notify_settings_types::{
    NotificationCategory, NotificationSettingsModelValues,
};
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

fn update_settings(patch: MapSettingsPatch, cx: &mut App) {
    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.update_settings(patch, cx));
}

fn pop(nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
    if let Some(nav_store) = nav_store.as_ref() {
        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
    }
}

fn push_section(nav_store: &Option<Entity<NavigationStore>>, section: MapSettingsSection, cx: &mut App) {
    if let Some(nav_store) = nav_store.as_ref() {
        nav_store.update(cx, |nav_store, cx| {
            nav_store.push(RootRoute::MapSettingsSection(section), NavigationStoreSettlingPush::Drop, cx);
        });
    }
}

trait MapSettingsSectionGlyph {
    fn icon(self) -> &'static str;
    fn color(self) -> Hsla;
}

impl MapSettingsSectionGlyph for MapSettingsSection {
    fn icon(self) -> &'static str {
        match self {
            MapSettingsSection::Location | MapSettingsSection::PresencePrivacy => "location.fill",
            MapSettingsSection::Tracks | MapSettingsSection::TracksPrivacy => {
                "point.topleft.down.to.point.bottomright.curvepath.fill"
            }
            MapSettingsSection::Merge => "arrow.triangle.merge",
            MapSettingsSection::Home => "house.fill",
            MapSettingsSection::Privacy => "lock.fill",
            MapSettingsSection::Notifications => "bell.fill",
            MapSettingsSection::MediaPrivacy => "photo.on.rectangle.angled",
            MapSettingsSection::FriendsPrivacy => "person.2.fill",
        }
    }

    fn color(self) -> Hsla {
        match self {
            MapSettingsSection::Location | MapSettingsSection::PresencePrivacy => SettingsIconPalette::profile(),
            MapSettingsSection::Tracks | MapSettingsSection::TracksPrivacy => SettingsIconPalette::privacy_goals(),
            MapSettingsSection::Merge => SettingsIconPalette::customization(),
            MapSettingsSection::Home => SettingsIconPalette::goals_plans(),
            MapSettingsSection::Privacy => SettingsIconPalette::stickers(),
            MapSettingsSection::Notifications => SettingsIconPalette::notifications(),
            MapSettingsSection::MediaPrivacy => SettingsIconPalette::memory_photos(),
            MapSettingsSection::FriendsPrivacy => SettingsIconPalette::privacy_birthday(),
        }
    }
}

/// the settings row of a section, a trait since the section type lives below the page
pub trait MapSettingsSectionRow {
    fn row(self, on_press: impl Fn(&mut Window, &mut App) + 'static) -> GroupedBtn;
}

impl MapSettingsSectionRow for MapSettingsSection {
    fn row(self, on_press: impl Fn(&mut Window, &mut App) + 'static) -> GroupedBtn {
        GroupedBtn::new(self.raw_value(), tr(self.title_key()))
            .icon(self.icon())
            .icon_background(self.color())
            .icon_foreground(white())
            .guide_anchor(self.guide_anchor())
            .callback(on_press)
    }
}

/// scroll view under the shell chrome with a background
/// separate element because the shell puts its floating top inset
/// into the content's environment and not the page's
#[derive(IntoElement)]
struct MapSettingsScroll {
    id: ElementId,
    content: AnyElement,
}

impl RenderOnce for MapSettingsScroll {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        scroll_view(self.id).list_shell_scroll_under_chrome(cx).bg(theme.bg._100).child(self.content)
    }
}

fn section_rows(
    id: &'static str,
    sections: &[MapSettingsSection],
    nav_store: &Option<Entity<NavigationStore>>,
) -> GroupedBtns {
    let items = sections
        .iter()
        .map(|section| {
            let section = *section;
            let nav_store = nav_store.clone();
            section.row(move |_, cx| push_section(&nav_store, section, cx))
        })
        .collect();
    GroupedBtns::new(id, items)
}

pub struct MapSettingsPage {}

impl MapSettingsPage {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn present_leave_confirmation(nav_store: Option<Entity<NavigationStore>>, cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("map_leave_title"),
            &tr("map_leave_message"),
            Some(tr("map_leave_action")),
            None,
            false,
            None,
            None,
            move |cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.leave_map(move |cx| pop(&nav_store, cx), cx);
                });
            },
            cx,
        );
    }
}

impl Render for MapSettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let back_store = nav_store.clone();
        let leave_store = nav_store.clone();

        let content = vstack(22.0)
            .w_full()
            .pt(px(14.0))
            .child(section_rows("map-settings-recording", &MapSettingsSection::RECORDING_SECTIONS, &nav_store))
            .child(section_rows("map-settings-sharing", &MapSettingsSection::SHARING_SECTIONS, &nav_store))
            .child(GroupedBtns::new(
                "map-settings-leave",
                vec![
                    GroupedBtn::new("map_leave", tr("map_leave_action"))
                        .icon("rectangle.portrait.and.arrow.right")
                        .icon_background(SettingsIconPalette::logout())
                        .destructive(true)
                        .show_chevron(false)
                        .callback(move |_, cx| Self::present_leave_confirmation(leave_store.clone(), cx)),
                ],
            ))
            .child(spacer_min_h(60.0));

        SettingsWrapperUi::new(tr("map_settings_title"), move |_, cx| pop(&back_store, cx))
            .content(MapSettingsScroll { id: "map-settings-scroll".into(), content: content.into_any_element() })
            .on_appear("map-settings-page", |_, cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.settings_appeared(cx));
            })
    }
}

/// privacy here is the map's own
/// its audience is friends on the map which the rest of the app has no word for
pub struct MapSettingsSectionPage {
    section: MapSettingsSection,
    edited_list: Option<MapPrivacyListTarget>,
    is_privacy_presented: bool,
    focus_handle: FocusHandle,
    focus_to_restore: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl MapSettingsSectionPage {
    pub fn new(section: MapSettingsSection, cx: &mut Context<Self>) -> Self {
        let presentation = RiseMapPresentation::shared(cx);
        let (state, friends, tracks, privacy) = {
            let presentation = presentation.read(cx);
            (
                presentation.state.clone(),
                presentation.friends.clone(),
                presentation.tracks.clone(),
                presentation.privacy.clone(),
            )
        };
        let sandbox = GuideSandbox::shared(cx);
        let interactions = MapInteractionsStore::shared(cx);
        let merge_preview_signal = interactions.read(cx).merge_preview_signal.clone();
        let notify_services = NotifyServicesStore::shared(cx);
        Self {
            section,
            edited_list: None,
            is_privacy_presented: false,
            focus_handle: cx.focus_handle(),
            focus_to_restore: None,
            _subscriptions: vec![
                cx.observe(&state, |_, _, cx| cx.notify()),
                cx.observe(&friends, |_, _, cx| cx.notify()),
                cx.observe(&tracks, |_, _, cx| cx.notify()),
                cx.observe(&privacy, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
                cx.observe(&interactions, |_, _, cx| cx.notify()),
                cx.observe(&merge_preview_signal, |_, _, cx| cx.notify()),
                cx.observe(&notify_services, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn element_id(&self, name: &str) -> ElementId {
        ElementId::Name(format!("map-settings-section-{}-{name}", self.section.raw_value()).into())
    }

    fn sharing_group(&self, settings: &MapSettings) -> AnyElement {
        GroupedBtns::new(
            self.element_id("sharing"),
            vec![
                Self::toggle_row(
                    "map_sharing",
                    "location.fill",
                    SettingsIconPalette::privacy(),
                    tr("map_settings_sharing"),
                    settings.sharing_enabled,
                    |next, cx| update_settings(MapSettingsPatch { sharing_enabled: Some(next), ..MapSettingsPatch::default() }, cx),
                ),
                Self::toggle_row(
                    "map_battery",
                    "battery.75",
                    SettingsIconPalette::memory(),
                    tr("map_settings_battery"),
                    settings.battery_visible,
                    |next, cx| update_settings(MapSettingsPatch { battery_visible: Some(next), ..MapSettingsPatch::default() }, cx),
                ),
            ],
        )
        .title(tr("map_settings_position_section").to_uppercase())
        .end_group_title(tr("map_settings_sharing_hint"))
        .into_any_element()
    }

    fn location_access_group(&self) -> AnyElement {
        GroupedBtns::new(
            self.element_id("location-access"),
            vec![
                GroupedBtn::new("map_location_access", tr("map_onboarding_location_title"))
                    .icon("iphone")
                    .icon_background(SettingsIconPalette::privacy())
                    .subtitle(tr("map_location_from_phone"))
                    .show_chevron(false),
            ],
        )
        .into_any_element()
    }

    fn tracks_group(&self, settings: &MapSettings) -> AnyElement {
        GroupedBtns::new(
            self.element_id("tracks"),
            vec![Self::toggle_row(
                "map_tracks",
                "point.topleft.down.to.point.bottomright.curvepath.fill",
                SettingsIconPalette::goals_plans(),
                tr("map_settings_tracks"),
                settings.tracks_enabled,
                |next, cx| update_settings(MapSettingsPatch { tracks_enabled: Some(next), ..MapSettingsPatch::default() }, cx),
            )],
        )
        .title(tr("map_tracks_title").to_uppercase())
        .end_group_title(tr("map_settings_tracks_hint"))
        .into_any_element()
    }

    fn precise_recording_group(&self, settings: &MapSettings) -> AnyElement {
        GroupedBtns::new(
            self.element_id("precise-recording"),
            vec![Self::toggle_row(
                "map_precise_recording",
                "scope",
                SettingsIconPalette::privacy_goals(),
                tr("map_precise_recording"),
                settings.track_precise_recording,
                |next, cx| {
                    update_settings(MapSettingsPatch { track_precise_recording: Some(next), ..MapSettingsPatch::default() }, cx);
                },
            )],
        )
        .end_group_title(tr("map_precise_recording_hint"))
        .into_any_element()
    }

    fn privacy_group(
        &self,
        kind: MapPrivacyKind,
        snapshot: &RiseMapPrivacySnapshot,
        page: &Entity<Self>,
        theme: &AppTheme,
    ) -> AnyElement {
        let privacy = snapshot.privacy.as_ref();
        let current = privacy.map(|privacy| privacy.rule(kind));
        let mut items: Vec<GroupedBtn> = kind
            .rules()
            .into_iter()
            .map(|rule| {
                GroupedBtn::new(format!("map_rule_{}_{}", kind.raw_value(), rule.wire()), tr(Self::rule_key(rule)))
                    .disabled(privacy.is_none())
                    .accessibility_selected(current == Some(rule))
                    .show_chevron(false)
                    .trailing_slot(div().opacity(if current == Some(rule) { 1.0 } else { 0.0 }).children(IconUi::sized(
                        "checkmark",
                        px(14.0),
                        theme.primary._100,
                        false,
                    )))
                    .callback(move |_, cx| {
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_privacy_rule(rule, kind, cx));
                    })
            })
            .collect();

        let allow_count = privacy.map_or(0, |privacy| privacy.users(kind, MapPrivacyAction::Allow).len());
        let deny_count = privacy.map_or(0, |privacy| privacy.users(kind, MapPrivacyAction::Deny).len());
        let edit_page = page.clone();
        items.push(
            GroupedBtn::new(
                format!("map_allow_{}", kind.raw_value()),
                tr(if current == Some(MapPrivacyRule::Selected) {
                    "map_privacy_selected_people"
                } else {
                    "map_privacy_always_show"
                }),
            )
            .trailing(if allow_count > 0 { allow_count.to_string() } else { tr("map_privacy_list_empty") })
            .disabled(privacy.is_none())
            .callback(move |_, cx| {
                edit_page.update(cx, |page, cx| {
                    page.edited_list = Some(MapPrivacyListTarget { kind, action: MapPrivacyAction::Allow });
                    cx.notify();
                });
            }),
        );
        let edit_page = page.clone();
        items.push(
            GroupedBtn::new(format!("map_deny_{}", kind.raw_value()), tr("map_privacy_never_show"))
                .trailing(if deny_count > 0 { deny_count.to_string() } else { tr("map_privacy_list_empty") })
                .disabled(privacy.is_none())
                .callback(move |_, cx| {
                    edit_page.update(cx, |page, cx| {
                        page.edited_list = Some(MapPrivacyListTarget { kind, action: MapPrivacyAction::Deny });
                        cx.notify();
                    });
                }),
        );

        let title_key = match kind {
            MapPrivacyKind::Presence => "map_privacy_presence_title",
            MapPrivacyKind::Tracks => "map_privacy_tracks_title",
            MapPrivacyKind::Media => "map_media_grids",
            MapPrivacyKind::Friends => "map_friends_title",
        };
        GroupedBtns::new(self.element_id("privacy"), items)
            .title(tr(title_key).to_uppercase())
            .end_group_title(tr("map_privacy_hint"))
            .into_any_element()
    }

    fn cooling_group(&self, settings: &MapSettings) -> AnyElement {
        GroupedBtns::new(
            self.element_id("cooling"),
            vec![
                GroupedBtn::new("map_cooling", "").guide_anchor(RiseGuideAnchor::MapCoolingCard).content_slot(
                    MapCoolingCard::new(
                        settings.track_cooling_days,
                        settings.track_heat_walks,
                        |next, _, cx| {
                            update_settings(MapSettingsPatch { track_cooling_days: Some(next), ..MapSettingsPatch::default() }, cx);
                        },
                        |next, _, cx| {
                            update_settings(MapSettingsPatch { track_heat_walks: Some(next), ..MapSettingsPatch::default() }, cx);
                        },
                    ),
                ),
            ],
        )
        .title(tr("map_settings_cooling_section").to_uppercase())
        .end_group_title(tr("map_settings_cooling_hint"))
        .into_any_element()
    }

    fn notifications_group(&self, cx: &mut App) -> AnyElement {
        let enabled = NotifyServicesStore::shared(cx)
            .update(cx, |services, cx| services.notification_settings(cx))
            .value(NotificationCategory::MapReactions);
        GroupedBtns::new(
            self.element_id("notifications"),
            vec![Self::toggle_row(
                "map_reaction_pushes",
                "heart.fill",
                SettingsIconPalette::delete_account(),
                tr("map_settings_reaction_pushes"),
                enabled,
                |next, cx| {
                    NotifyInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.set_notification_category(NotificationCategory::MapReactions, next, cx);
                    });
                },
            )],
        )
        .title(tr("map_settings_notifications_section").to_uppercase())
        .end_group_title(tr("map_settings_reaction_pushes_hint"))
        .into_any_element()
    }

    fn home_group(&self, home: Option<&MapPlace>, is_setting_home: bool) -> AnyElement {
        let subtitle = match home {
            Some(home) => tr(if home.is_inferred { "map_home_inferred" } else { "map_home_manual" }),
            None => tr("map_home_none"),
        };
        let mut items = vec![
            GroupedBtn::new("map_home_set", tr("map_home_set_here"))
                .icon("house.fill")
                .icon_background(SettingsIconPalette::profile())
                .subtitle(subtitle)
                .disabled(is_setting_home)
                .guide_anchor(RiseGuideAnchor::MapHomeSetHere)
                .show_chevron(false)
                .callback(|_, cx| {
                    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.set_home_here(cx));
                }),
        ];
        if home.is_some() {
            items.push(
                GroupedBtn::new("map_home_remove", tr("map_home_remove"))
                    .icon("minus.circle.fill")
                    .icon_background(SettingsIconPalette::delete_account())
                    .destructive(true)
                    .show_chevron(false)
                    .callback(|_, cx| {
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.remove_home(cx));
                    }),
            );
        }
        GroupedBtns::new(self.element_id("home"), items)
            .title(tr("map_home_title").to_uppercase())
            .end_group_title(tr("map_home_hint"))
            .into_any_element()
    }

    fn home_radius_group(&self, settings: &MapSettings) -> AnyElement {
        GroupedBtns::new(
            self.element_id("home-radius"),
            vec![
                GroupedBtn::new("map_home_track_radius", "").guide_anchor(RiseGuideAnchor::MapHomeRadius).content_slot(
                    MapHomeTrackRadiusControl::new(settings.home_track_exclusion_radius_m, |next, cx| {
                        update_settings(
                            MapSettingsPatch { home_track_exclusion_radius_m: Some(next), ..MapSettingsPatch::default() },
                            cx,
                        );
                    }),
                ),
            ],
        )
        .title(tr("map_tracks_title").to_uppercase())
        .end_group_title(tr("map_home_track_radius_hint"))
        .into_any_element()
    }

    fn delete_tracks_group(&self, is_deleting_tracks: bool) -> AnyElement {
        GroupedBtns::new(
            self.element_id("delete-tracks"),
            vec![
                GroupedBtn::new("map_delete_tracks", tr("map_tracks_delete_action"))
                    .icon("trash.fill")
                    .icon_background(SettingsIconPalette::delete_account())
                    .destructive(true)
                    .disabled(is_deleting_tracks)
                    .show_chevron(false)
                    .callback(|_, cx| Self::present_delete_tracks_confirmation(cx)),
            ],
        )
        .into_any_element()
    }

    fn present_delete_tracks_confirmation(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("map_tracks_delete_title"),
            &tr("map_tracks_delete_message"),
            Some(tr("map_tracks_delete_action")),
            None,
            false,
            None,
            None,
            |cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.delete_all_tracks(cx));
            },
            cx,
        );
    }

    fn retry_group(&self, error: Option<&str>) -> AnyElement {
        GroupedBtns::new(
            self.element_id("retry"),
            vec![
                GroupedBtn::new("map_settings_retry", tr("retry"))
                    .icon("arrow.clockwise")
                    .icon_background(SettingsIconPalette::reset())
                    .show_chevron(false)
                    .callback(|_, cx| {
                        MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.settings_appeared(cx));
                    }),
            ],
        )
        .end_group_title(error.map_or_else(|| tr("map_load_failed"), str::to_owned))
        .into_any_element()
    }

    fn skeleton(&self) -> AnyElement {
        let heights: Vec<f32> = match self.section {
            MapSettingsSection::Location => vec![72.0, 112.0],
            MapSettingsSection::Tracks => vec![56.0, 56.0, 196.0, 56.0],
            MapSettingsSection::Merge => vec![56.0, 112.0, 112.0, 420.0],
            MapSettingsSection::Home => vec![72.0, 132.0],
            MapSettingsSection::Notifications => vec![56.0],
            MapSettingsSection::Privacy => vec![224.0],
            MapSettingsSection::PresencePrivacy
            | MapSettingsSection::TracksPrivacy
            | MapSettingsSection::MediaPrivacy
            | MapSettingsSection::FriendsPrivacy => {
                vec![(self.section.privacy_kind().map_or(0, |kind| kind.rules().len()) + 2) as f32 * 56.0]
            }
        };
        vstack(22.0)
            .w_full()
            .children(heights.into_iter().map(|height| {
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(div().px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(SkeletonLine::new(130.0, 12.0)))
                    .child(div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(SkeletonBlock::new(
                        None::<f32>,
                        height,
                        GroupedCardMetrics::CORNER_RADIUS,
                    )))
                    .child(div().px(px(GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)).child(SkeletonLine::new(240.0, 12.0)))
            }))
            .into_any_element()
    }

    fn toggle_row(
        id: &'static str,
        icon: &'static str,
        background: Hsla,
        title: String,
        value: bool,
        on_change: impl Fn(bool, &mut App) + 'static,
    ) -> GroupedBtn {
        let on_change = Rc::new(on_change);
        let switch_change = on_change.clone();
        GroupedBtn::new(id, title)
            .icon(icon)
            .icon_background(background)
            .show_chevron(false)
            .trailing_slot(SwitchUi::new(
                ElementId::Name(format!("{id}-switch").into()),
                Binding::new(move |_| value, move |next, _, cx| switch_change(next, cx)),
            ))
            .callback(move |_, cx| on_change(!value, cx))
    }

    fn rule_key(rule: MapPrivacyRule) -> &'static str {
        match rule {
            MapPrivacyRule::MapFriends => "map_privacy_rule_friends",
            MapPrivacyRule::Selected => "map_privacy_rule_selected",
            MapPrivacyRule::Contacts => "map_privacy_rule_contacts",
            MapPrivacyRule::Nobody => "map_privacy_rule_nobody",
        }
    }

    fn body(
        &self,
        nav_store: &Option<Entity<NavigationStore>>,
        privacy: &RiseMapPrivacySnapshot,
        page: &Entity<Self>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let section = self.section;
        if section == MapSettingsSection::Privacy {
            return vec![
                section_rows("map-settings-privacy-sections", &MapSettingsSection::PRIVACY_SECTIONS, nav_store).into_any_element(),
            ];
        }
        if let Some(kind) = section.privacy_kind() {
            if privacy.privacy.is_some() {
                return vec![self.privacy_group(kind, privacy, page, theme)];
            }
            if privacy.load_state == RiseMapLoadState::Failed {
                return vec![self.retry_group(privacy.error_text.as_deref())];
            }
            return vec![self.skeleton()];
        }
        let state = MapServicesStore::state(cx);
        let Some(settings) = state.state.as_ref().map(|state| &state.settings) else {
            if state.load_state == RiseMapLoadState::Failed {
                return vec![self.retry_group(state.error_text.as_deref())];
            }
            return vec![self.skeleton()];
        };
        match section {
            MapSettingsSection::Location => vec![self.location_access_group(), self.sharing_group(settings)],
            MapSettingsSection::Tracks => {
                let is_deleting_tracks = MapInteractionsStore::shared(cx).read(cx).is_deleting_tracks;
                vec![
                    self.tracks_group(settings),
                    self.precise_recording_group(settings),
                    self.cooling_group(settings),
                    self.delete_tracks_group(is_deleting_tracks),
                ]
            }
            MapSettingsSection::Merge => vec![MapMergeSettingsSection::new(settings.clone()).into_any_element()],
            MapSettingsSection::Home => {
                let tracks = MapServicesStore::tracks(cx);
                let is_setting_home = MapInteractionsStore::shared(cx).read(cx).is_setting_home;
                vec![self.home_group(tracks.home.as_ref(), is_setting_home), self.home_radius_group(settings)]
            }
            MapSettingsSection::Notifications => vec![self.notifications_group(cx)],
            MapSettingsSection::PresencePrivacy
            | MapSettingsSection::TracksPrivacy
            | MapSettingsSection::MediaPrivacy
            | MapSettingsSection::FriendsPrivacy
            | MapSettingsSection::Privacy => Vec::new(),
        }
    }
}

impl Render for MapSettingsSectionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let section = self.section;
        let page = cx.entity();
        let nav_store = env::<NavigationStoreKey>(cx);
        let privacy = MapServicesStore::privacy(cx);

        let home_is_set = section == MapSettingsSection::Home
            && MapServicesStore::tracks(cx).home.as_ref().is_some_and(|home| !home.is_inferred);
        on_change(self.element_id("home-is-set"), home_is_set, true, window, cx, |_, set, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapHomeSetManually, *set, cx);
        });
        let presented_page = page.clone();
        on_change(self.element_id("edited-list"), self.edited_list, false, window, cx, move |_, target, _, cx| {
            if target.is_some() {
                presented_page.update(cx, |page, cx| {
                    page.is_privacy_presented = true;
                    cx.notify();
                });
            }
        });

        let focus_page = page.clone();
        on_change(self.element_id("privacy-focus"), self.is_privacy_presented, false, window, cx, move |_, presented, window, cx| {
            let presented = *presented;
            focus_page.update(cx, |page, cx| {
                if presented {
                    page.focus_to_restore = window.focused(cx);
                    window.focus(&page.focus_handle, cx);
                } else if page.focus_handle.is_focused(window) {
                    match page.focus_to_restore.take() {
                        Some(previous) => window.focus(&previous, cx),
                        None => window.blur(),
                    }
                }
            });
        });

        let body = self.body(&nav_store, &privacy, &page, &theme, cx);
        let content = vstack(22.0).w_full().pt(px(14.0)).children(body).child(spacer_min_h(60.0));

        let sheet = self.edited_list.map(|target| {
            let friends = MapServicesStore::friends(cx);
            let current_privacy = privacy.privacy.as_ref();
            let dismiss_page = page.clone();
            MapPrivacyUsersSheet {
                is_presented: Binding::entity(
                    page.clone(),
                    |page: &Self| page.is_privacy_presented,
                    |page: &mut Self, value, _| page.is_privacy_presented = value,
                ),
                target,
                rule: current_privacy.map_or(MapPrivacyRule::MapFriends, |privacy| privacy.rule(target.kind)),
                selected: current_privacy.map_or_else(Vec::new, |privacy| privacy.users(target.kind, target.action).to_vec()),
                candidates: friends.friends.iter().map(|friend| friend.user.clone()).collect(),
                has_more_candidates: friends.has_more_friends,
                on_load_more: Rc::new(|cx: &mut App| {
                    MapInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.load_more(RiseMapFriendsRepositoryList::Friends, cx));
                }),
                on_save: Rc::new(move |users: Vec<MapUser>, cx: &mut App| {
                    MapInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.save_privacy_users(users, target.kind, target.action, cx));
                }),
                on_did_dismiss: Rc::new(move |cx: &mut App| {
                    dismiss_page.update(cx, |page, cx| {
                        if !page.is_privacy_presented {
                            page.edited_list = None;
                            cx.notify();
                        }
                    });
                }),
            }
        });

        let back_store = nav_store.clone();
        let sheet_focus = self.focus_handle.clone();
        let escape_page = page.clone();
        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr(section.title_key()), move |_, cx| pop(&back_store, cx))
                    .content(MapSettingsScroll { id: self.element_id("scroll"), content: content.into_any_element() }),
            )
            .children(sheet.map(|sheet| {
                div()
                    .absolute()
                    .inset_0()
                    .track_focus(&sheet_focus)
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key != "escape" {
                            return;
                        }
                        cx.stop_propagation();
                        escape_page.update(cx, |page, cx| {
                            page.is_privacy_presented = false;
                            cx.notify();
                        });
                    })
                    .child(sheet)
            }))
            .on_appear(self.element_id("appear"), move |_, cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.settings_appeared(cx));
                if section == MapSettingsSection::Notifications {
                    NotifyInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.ensure_notification_settings_loaded(cx));
                }
            })
            .and_on_disappear(move |_, cx| {
                if section == MapSettingsSection::Notifications {
                    NotifyInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.flush_notification_settings(cx));
                }
                if section == MapSettingsSection::Home {
                    GuideFacts::set(RiseGuideSignal::MapHomeSetManually, false, cx);
                }
            })
    }
}

#[derive(Clone)]
enum MapPrivacyUsersSheetRow {
    Title,
    NoFriends,
    User(MapUser),
    MoreCandidates,
    BottomInset,
}

impl MapPrivacyUsersSheetRow {
    fn key(&self) -> String {
        match self {
            Self::Title => "title".to_owned(),
            Self::NoFriends => "no-friends".to_owned(),
            Self::User(user) => format!("user-{}", user.id),
            Self::MoreCandidates => "more".to_owned(),
            Self::BottomInset => "bottom".to_owned(),
        }
    }
}

/// state of the sheet, recreated with the list it edits
struct MapPrivacyUsersSheetState {
    chosen: HashSet<String>,
    list: ListUi,
    row_keys: Vec<String>,
}

impl MapPrivacyUsersSheetState {
    fn sync_list(&mut self, rows: &[MapPrivacyUsersSheetRow]) {
        let keys: Vec<String> = rows.iter().map(MapPrivacyUsersSheetRow::key).collect();
        if keys != self.row_keys {
            let new_index: HashMap<&str, usize> =
                keys.iter().enumerate().map(|(index, key)| (key.as_str(), index)).collect();
            let old_keys = &self.row_keys;
            self.list.rebuilt(keys.len(), |old| old_keys.get(old).and_then(|key| new_index.get(key.as_str()).copied()));
            self.row_keys = keys;
        }
    }
}

/// picks the people of one exception list
/// candidates are friends on the map only
/// a stranger can never be shown so offering one would give a switch that does nothing
#[derive(IntoElement)]
struct MapPrivacyUsersSheet {
    is_presented: Binding<bool>,
    target: MapPrivacyListTarget,
    rule: MapPrivacyRule,
    selected: Vec<MapUser>,
    candidates: Vec<MapUser>,
    has_more_candidates: bool,
    on_load_more: Rc<dyn Fn(&mut App)>,
    on_save: Rc<dyn Fn(Vec<MapUser>, &mut App)>,
    on_did_dismiss: Rc<dyn Fn(&mut App)>,
}

impl MapPrivacyUsersSheet {
    fn merged_people(&self) -> Vec<MapUser> {
        let mut seen = HashSet::new();
        self.selected.iter().chain(self.candidates.iter()).filter(|user| seen.insert(user.id.clone())).cloned().collect()
    }

    fn title_key(&self) -> &'static str {
        match self.target.action {
            MapPrivacyAction::Allow => {
                if self.rule == MapPrivacyRule::Selected {
                    "map_privacy_selected_people"
                } else {
                    "map_privacy_always_show"
                }
            }
            MapPrivacyAction::Deny => "map_privacy_never_show",
        }
    }

    fn row(
        row: &MapPrivacyUsersSheetRow,
        title: &str,
        state: &Entity<MapPrivacyUsersSheetState>,
        on_load_more: &Rc<dyn Fn(&mut App)>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        match row {
            MapPrivacyUsersSheetRow::Title => div()
                .w_full()
                .pt(px(14.0))
                .pb(px(18.0))
                .px(px(22.0))
                .child(
                    MainText::new(title.to_owned())
                        .px(18.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .into_any_element(),
            MapPrivacyUsersSheetRow::NoFriends => div()
                .w_full()
                .min_h(px(180.0))
                .px(px(28.0))
                .flex()
                .flex_col()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .child(MainText::new(tr("map_privacy_no_friends")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary)),
                )
                .into_any_element(),
            MapPrivacyUsersSheetRow::User(user) => Self::user_row(user, state, theme, cx),
            MapPrivacyUsersSheetRow::MoreCandidates => {
                let on_load_more = on_load_more.clone();
                hstack(12.0)
                    .w_full()
                    .h(px(58.0))
                    .px(px(16.0))
                    .items_center()
                    .child(SkeletonCircle::new(42.0))
                    .child(SkeletonLine::new(150.0, 13.0))
                    .child(spacer_min_w(0.0))
                    .on_appear("map-privacy-users-more", move |_, cx| on_load_more(cx))
                    .into_any_element()
            }
            MapPrivacyUsersSheetRow::BottomInset => div().w_full().h(px(112.0)).into_any_element(),
        }
    }

    fn user_row(user: &MapUser, state: &Entity<MapPrivacyUsersSheetState>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let is_on = state.read(cx).chosen.contains(&user.id);
        let toggle_state = state.clone();
        let user_id = user.id.clone();
        let display_name = SharedString::from(user.display_name().to_owned());
        plain(ElementId::Name(format!("map-privacy-user-{}", user.id).into()))
            .w_full()
            .px(px(16.0))
            .on_press(move |_, cx| {
                toggle_state.update(cx, |state, cx| {
                    if is_on {
                        state.chosen.remove(&user_id);
                    } else {
                        state.chosen.insert(user_id.clone());
                    }
                    cx.notify();
                });
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .h(px(58.0))
                    .items_center()
                    .child(
                        pass_through().flex_none().child(
                            UserLogoUi::new(ElementId::Name(format!("map-privacy-user-logo-{}", user.id).into()))
                                .source(SharedString::from(user.logo.clone()))
                                .name(display_name.clone())
                                .user_id(SharedString::from(user.id.clone()))
                                .size(42.0),
                        ),
                    )
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .min_w_0()
                            .child(MainText::new(display_name).px(16.0).font_weight(FontWeight::MEDIUM).number_of_lines(1))
                            .child(MainText::new(format!("@{}", user.tag)).px(13.0).color(theme.text.secondary).number_of_lines(1)),
                    )
                    .child(spacer_min_w(8.0))
                    .children(IconUi::sized(
                        if is_on { "checkmark.circle.fill" } else { "circle" },
                        px(22.0),
                        if is_on { theme.primary._100 } else { theme.text.secondary.opacity(0.5) },
                        false,
                    )),
            )
            .into_any_element()
    }
}

impl RenderOnce for MapPrivacyUsersSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let people = self.merged_people();
        let selected_ids: HashSet<String> = self.selected.iter().map(|user| user.id.clone()).collect();
        let list_theme = theme.clone();
        let initially_chosen = selected_ids.clone();
        let state = window.use_keyed_state(
            ElementId::Name(format!("map-privacy-users-sheet-{}", self.target.id()).into()),
            cx,
            move |_, _| MapPrivacyUsersSheetState {
                chosen: initially_chosen,
                list: ListUi::new(&list_theme, 0, ListAlignment::Top),
                row_keys: Vec::new(),
            },
        );

        let mut rows = vec![MapPrivacyUsersSheetRow::Title];
        if people.is_empty() {
            rows.push(MapPrivacyUsersSheetRow::NoFriends);
        } else {
            rows.extend(people.iter().cloned().map(MapPrivacyUsersSheetRow::User));
            if self.has_more_candidates {
                rows.push(MapPrivacyUsersSheetRow::MoreCandidates);
            }
        }
        rows.push(MapPrivacyUsersSheetRow::BottomInset);
        state.update(cx, |state, _| state.sync_list(&rows));
        let list_state: ListState = state.read(cx).list.state().clone();
        let is_dirty = state.read(cx).chosen != selected_ids;

        let rows = Rc::new(rows);
        let title = tr(self.title_key());
        let content_state = state.clone();
        let content_theme = theme.clone();
        let cancel_presented = self.is_presented.clone();
        let save_presented = self.is_presented.clone();
        let footer_theme = theme.clone();
        let on_save = self.on_save;
        let people = Rc::new(people);
        let save_state = state;
        let dismiss = self.on_did_dismiss;
        let refused_dismiss = dismiss.clone();
        let refused_presented = self.is_presented.clone();
        let on_load_more = self.on_load_more;

        BottomSheetUi::new(
            ElementId::Name(format!("map-privacy-users-{}", self.target.id()).into()),
            self.is_presented,
            move |_, _| {
                let rows = rows.clone();
                let title = title.clone();
                let state = content_state.clone();
                let theme = content_theme.clone();
                let on_load_more = on_load_more.clone();
                list(list_state.clone(), move |index, _, cx| {
                    let Some(row) = rows.get(index) else {
                        return Empty.into_any_element();
                    };
                    MapPrivacyUsersSheet::row(row, &title, &state, &on_load_more, &theme, cx)
                })
                .size_full()
                .into_any_element()
            },
            move |_, _| {
                let cancel_presented = cancel_presented.clone();
                let save_presented = save_presented.clone();
                let on_save = on_save.clone();
                let people = people.clone();
                let save_state = save_state.clone();
                hstack(10.0)
                    .w_full()
                    .pb(px(12.0))
                    .child(
                        div().flex_1().min_w_0().child(
                            ButtonUi::new("map-privacy-users-cancel")
                                .background_color(footer_theme.text.secondary.opacity(0.18))
                                .height(50.0)
                                .on_press(move |window, cx| cancel_presented.set(false, window, cx))
                                .child(
                                    MainText::new(tr("cancel"))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(footer_theme.text.primary),
                                ),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            ButtonUi::new("map-privacy-users-done")
                                .disabled(!is_dirty)
                                .height(50.0)
                                .on_press(move |window, cx| {
                                    let chosen = save_state.read(cx).chosen.clone();
                                    on_save(people.iter().filter(|user| chosen.contains(&user.id)).cloned().collect(), cx);
                                    save_presented.set(false, window, cx);
                                })
                                .child(MainText::new(tr("done")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white())),
                        ),
                    )
                    .into_any_element()
            },
        )
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(14.0))
        .footer_top_padding(Some(8.0))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| dismiss(cx))
        .on_presentation_refused(move |window, cx| {
            refused_presented.set(false, window, cx);
            refused_dismiss(cx);
        })
    }
}

#[derive(Default)]
struct MapHomeTrackRadiusControlState {
    draft: Option<i64>,
}

#[derive(IntoElement)]
struct MapHomeTrackRadiusControl {
    radius: i64,
    on_commit: Rc<dyn Fn(i64, &mut App)>,
}

impl MapHomeTrackRadiusControl {
    fn new(radius: i64, on_commit: impl Fn(i64, &mut App) + 'static) -> Self {
        Self { radius, on_commit: Rc::new(on_commit) }
    }

    fn meters(value: i64) -> String {
        tr_args("map_home_track_radius_value", &[FormatArg::Int(value)])
    }
}

impl RenderOnce for MapHomeTrackRadiusControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let radius = self.radius;
        let state = window.use_keyed_state("map-home-track-radius-control", cx, |_, _| MapHomeTrackRadiusControlState::default());
        let change_state = state.clone();
        on_change("map-home-track-radius-value", radius, false, window, cx, move |_, _, _, cx| {
            change_state.update(cx, |state, cx| {
                state.draft = None;
                cx.notify();
            });
        });
        let shown = state.read(cx).draft.unwrap_or(radius);
        let draft_state = state.clone();
        let commit_state = state;
        let on_commit = self.on_commit;

        vstack(14.0)
            .w_full()
            .child(
                hstack(None)
                    .w_full()
                    .items_baseline()
                    .child(MainText::new(tr("map_home_track_radius_title")).px(16.0))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(Self::meters(shown))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .primary(true)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(
                SteppedSliderUi::new(
                    "map-home-track-radius-rail",
                    shown as f64,
                    (MapSettings::minimum_home_track_radius() as f64)..=(MapSettings::maximum_home_track_radius() as f64),
                    1.0,
                    move |value, _, cx| {
                        draft_state.update(cx, |state, cx| {
                            state.draft = Some(value.round() as i64);
                            cx.notify();
                        });
                    },
                    move |value, _, cx| {
                        commit_state.update(cx, |state, cx| {
                            state.draft = None;
                            cx.notify();
                        });
                        let clamped = MapSettings::clamped_home_track_radius(value.round() as i64);
                        if clamped != radius {
                            on_commit(clamped, cx);
                        }
                    },
                )
                .shows_step_ticks(false),
            )
            .child(
                hstack(None)
                    .w_full()
                    .child(MainText::new(Self::meters(MapSettings::minimum_home_track_radius())).px(12.0))
                    .child(spacer())
                    .child(MainText::new(Self::meters(MapSettings::maximum_home_track_radius())).px(12.0)),
            )
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
    }
}
