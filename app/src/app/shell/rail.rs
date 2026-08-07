use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, ElementId, IntoElement, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_navigation::RootTab;
use rise_platform::materials::Material;
use rise_theme::AppTheme;
use rise_ui::{IconSize, IconUi};
use rise_widgets::GlassPanel;

/// One section of the rail: what the phone puts in its tab bar.
///
/// The SF Symbol and the string key are the reference's own —
/// `MainTabs.swift` — so the rail and the phone's tab bar cannot drift apart
/// without the drift being visible in a diff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RailSection {
    pub tab: RootTab,
    pub sf_symbol: &'static str,
    pub title_key: &'static str,
}

impl RailSection {
    pub const ALL: [Self; 5] = [
        Self {
            tab: RootTab::Feed,
            sf_symbol: "doc.text",
            title_key: "navbtn_posts",
        },
        Self {
            tab: RootTab::Search,
            sf_symbol: "magnifyingglass",
            title_key: "navbtn_search",
        },
        Self {
            tab: RootTab::Chats,
            sf_symbol: "bubble.left.and.bubble.right",
            title_key: "navbtn_chats",
        },
        Self {
            tab: RootTab::Shorts,
            sf_symbol: "play.rectangle.on.rectangle",
            title_key: "navbtn_shorts",
        },
        Self {
            tab: RootTab::Profile,
            sf_symbol: "ellipsis.circle",
            title_key: "more_tab_title",
        },
    ];

    pub fn title(&self) -> String {
        tr(self.title_key)
    }
}

/// A chat folder, stacked under the sections the way Telegram for macOS does it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RailFolder {
    pub id: String,
    pub title: SharedString,
    pub unread: u32,
}

/// What a click on the rail addresses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RailTarget {
    Section(RootTab),
    Folder(String),
}

pub struct RailState<'a> {
    pub selected: RootTab,
    pub folders: &'a [RailFolder],
    pub selected_folder: Option<&'a str>,
}

type Activate<V> = Rc<dyn Fn(&mut V, RailTarget, &mut Window, &mut Context<V>)>;
type Contextual<V> = Rc<dyn Fn(&mut V, RailTarget, Point<Pixels>, &mut Window, &mut Context<V>)>;

pub struct RailHandlers<V: 'static> {
    pub activate: Activate<V>,
    pub context_menu: Contextual<V>,
}

/// The vertical strip on the left.
///
/// This is the only part of the design that deliberately departs from the phone.
/// It is `Material::Chrome`, which on a recent macOS is a real system material
/// and everywhere else is paint — the rail itself never learns which.
pub struct Rail;

impl Rail {
    pub const REGION_KEY: &'static str = "shell.rail";

    /// The tab-index the sections start at.
    ///
    /// The rail is the first thing Tab reaches, before the columns beside it,
    /// because it is the first thing the eye reaches.
    const FIRST_TAB_INDEX: isize = 0;

    pub fn render<V: 'static>(
        state: RailState<'_>,
        handlers: &RailHandlers<V>,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let theme: AppTheme = rise_ui::theme(cx as &App).clone();
        let metrics = theme.shell;

        let mut rail = GlassPanel::surface(Self::REGION_KEY, Material::Chrome, cx)
            .w(metrics.rail_width)
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(metrics.rail_item_gap)
            .p(metrics.rail_padding)
            .tab_group();

        for (index, section) in RailSection::ALL.iter().enumerate() {
            rail = rail.child(Self::item(
                &theme,
                ElementId::Name(SharedString::new_static(section.tab.screen_id())),
                Self::FIRST_TAB_INDEX + index as isize,
                section.sf_symbol,
                Some(section.title()),
                section.tab == state.selected,
                None,
                RailTarget::Section(section.tab),
                handlers,
                cx,
            ));
        }

        if !state.folders.is_empty() {
            rail = rail.child(Self::divider(&theme));
        }

        let folders_start = Self::FIRST_TAB_INDEX + RailSection::ALL.len() as isize;
        for (index, folder) in state.folders.iter().enumerate() {
            rail = rail.child(Self::item(
                &theme,
                ElementId::Name(SharedString::from(format!("folder.{}", folder.id))),
                folders_start + index as isize,
                "folder",
                Some(folder.title.to_string()),
                state.selected_folder == Some(folder.id.as_str()),
                Some(folder.unread),
                RailTarget::Folder(folder.id.clone()),
                handlers,
                cx,
            ));
        }

        rail
    }

    fn divider(theme: &AppTheme) -> impl IntoElement {
        div()
            .w(theme.shell.rail_item_size)
            .h(theme.shell.rail_item_gap / 3.0)
            .my(theme.shell.rail_item_gap)
            .rounded(theme.radius._300)
            .bg(theme.border._200)
    }

    /// Icon over a caption, the way the phone's tab bar reads, rather than an
    /// icon alone with a tooltip: gpui ships no tooltip and nothing may leave
    /// the window rectangle, so a hover-only label would be ours to build and
    /// would still be unreachable from the keyboard.
    #[allow(clippy::too_many_arguments)]
    fn item<V: 'static>(
        theme: &AppTheme,
        id: ElementId,
        tab_index: isize,
        sf_symbol: &'static str,
        title: Option<String>,
        is_selected: bool,
        unread: Option<u32>,
        target: RailTarget,
        handlers: &RailHandlers<V>,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let tint = if is_selected {
            theme.primary._100
        } else {
            theme.text.secondary
        };
        let caption = theme.typography.caption_small();

        let mut glyph = div()
            .relative()
            .size(theme.shell.rail_item_size)
            .flex()
            .items_center()
            .justify_center();

        if let Some(icon) = IconUi::render(theme, sf_symbol, IconSize::Large, tint) {
            glyph = glyph.child(icon);
        }

        if let Some(count) = unread.filter(|count| *count > 0) {
            glyph = glyph.child(Self::badge(theme, count));
        }

        let activate = handlers.activate.clone();
        let on_click_target = target.clone();
        let open_menu = handlers.context_menu.clone();

        let mut item = div()
            .id(id)
            .tab_index(tab_index)
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .py(theme.shell.rail_item_gap / 2.0)
            .rounded(theme.radius._300)
            .when(is_selected, |item| item.bg(theme.bg._400))
            .hover(|item| item.bg(theme.bg._300))
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                activate(view, on_click_target.clone(), window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    open_menu(view, target.clone(), event.position, window, cx);
                }),
            )
            .child(glyph);

        if let Some(title) = title {
            item = item.child(
                div()
                    .w_full()
                    .text_size(caption.size)
                    .font(caption.font)
                    .text_color(tint)
                    .text_center()
                    .truncate()
                    .child(title),
            );
        }

        item
    }

    /// Capped at 99+ so a folder with four thousand unread messages cannot widen
    /// the rail and shift every column beside it.
    fn badge(theme: &AppTheme, count: u32) -> impl IntoElement {
        let label = if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        };
        let caption = theme.typography.caption_small();

        div()
            .absolute()
            .top_0()
            .right_0()
            .px(theme.shell.rail_item_gap / 2.0)
            .h(theme.icon.small)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.icon.small)
            .bg(theme.primary._200)
            .text_size(caption.size)
            .font(caption.font)
            .text_color(theme.bg._000)
            .child(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_presents_every_tab_in_the_reference_order() {
        let presented: Vec<RootTab> = RailSection::ALL.iter().map(|s| s.tab).collect();

        assert_eq!(
            presented,
            RootTab::ALL.to_vec(),
            "the rail is a presentation of RootTab::ALL; a different order moves \
             the sections under the user's Cmd+1..5 as well"
        );
        assert_eq!(
            presented,
            vec![
                RootTab::Feed,
                RootTab::Search,
                RootTab::Chats,
                RootTab::Shorts,
                RootTab::Profile
            ],
            "MainTabs.swift declares posts, search, chats, shorts, more"
        );
    }

    #[test]
    fn every_section_names_an_icon_the_bundle_actually_carries() {
        for section in RailSection::ALL {
            assert!(
                IconUi::asset_path(section.sf_symbol).is_some(),
                "{} has no Lucide mapping, so the rail would draw a gap",
                section.sf_symbol
            );
        }
    }

    #[test]
    fn the_folder_icon_is_mapped_too() {
        assert!(IconUi::asset_path("folder").is_some());
    }

    #[test]
    fn every_section_carries_a_string_key_rather_than_literal_copy() {
        for section in RailSection::ALL {
            assert!(!section.title_key.is_empty());
            assert!(
                section.title_key.is_ascii() && !section.title_key.contains(' '),
                "{} looks like copy, not a key",
                section.title_key
            );
        }
    }

    #[test]
    fn the_section_keys_are_the_ones_the_reference_uses() {
        let keys: Vec<&str> = RailSection::ALL.iter().map(|s| s.title_key).collect();
        assert_eq!(
            keys,
            vec![
                "navbtn_posts",
                "navbtn_search",
                "navbtn_chats",
                "navbtn_shorts",
                "more_tab_title"
            ]
        );
    }

    #[test]
    fn every_tab_resolves_to_exactly_one_section() {
        for tab in RootTab::ALL {
            let matches = RailSection::ALL
                .iter()
                .filter(|section| section.tab == tab)
                .count();
            assert_eq!(matches, 1, "{tab:?} is presented {matches} times");
        }
    }
}
