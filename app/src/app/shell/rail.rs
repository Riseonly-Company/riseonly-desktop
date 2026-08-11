use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, ElementId, IntoElement, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Window, div, prelude::*,
};
use rise_i18n::tr;
use rise_navigation::RootTab;
use rise_theme::AppTheme;
use rise_ui::{IconSize, IconUi};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RailSection {
    pub tab: RootTab,
    pub sf_symbol: &'static str,
    pub title_key: &'static str,
}

impl RailSection {
    pub const ALL: [Self; 7] = [
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
            tab: RootTab::Vacancies,
            sf_symbol: "briefcase",
            title_key: "navbtn_vacancies",
        },
        Self {
            tab: RootTab::Music,
            sf_symbol: "music.note",
            title_key: "navbtn_music",
        },
        Self {
            tab: RootTab::Profile,
            sf_symbol: "person.crop.circle",
            title_key: "navbtn_profile",
        },
    ];

    pub fn primary() -> &'static [Self] {
        &Self::ALL[..Self::ALL.len() - 1]
    }

    pub fn footer() -> &'static Self {
        &Self::ALL[Self::ALL.len() - 1]
    }

    pub fn title(&self) -> String {
        tr(self.title_key)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RailFolder {
    pub id: String,
    pub title: SharedString,
    pub unread: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RailTarget {
    Section(RootTab),
    Folder(String),
    Settings,
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

pub struct Rail;

impl Rail {
    const SETTINGS_SYMBOL: &'static str = "gearshape";

    const FIRST_TAB_INDEX: isize = 0;

    pub fn render<V: 'static>(
        state: RailState<'_>,
        corner_radius: Pixels,
        handlers: &RailHandlers<V>,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let theme: AppTheme = rise_ui::theme(cx as &App).clone();
        let metrics = theme.shell;

        let mut top = div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .gap(metrics.rail_item_gap);

        for (index, section) in RailSection::primary().iter().enumerate() {
            top = top.child(Self::item(
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
            top = top.child(Self::divider(&theme));
        }

        let folders_start = Self::FIRST_TAB_INDEX + RailSection::primary().len() as isize;
        for (index, folder) in state.folders.iter().enumerate() {
            top = top.child(Self::item(
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

        let profile = RailSection::footer();
        let footer_index = folders_start + state.folders.len() as isize;

        let bottom = div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .gap(metrics.rail_item_gap)
            .child(Self::divider(&theme))
            .child(Self::item(
                &theme,
                ElementId::Name(SharedString::new_static("rail.settings")),
                footer_index,
                Self::SETTINGS_SYMBOL,
                Some(tr("settings_page_title")),
                false,
                None,
                RailTarget::Settings,
                handlers,
                cx,
            ))
            .child(Self::item(
                &theme,
                ElementId::Name(SharedString::new_static(profile.tab.screen_id())),
                footer_index + 1,
                profile.sf_symbol,
                Some(profile.title()),
                profile.tab == state.selected,
                None,
                RailTarget::Section(profile.tab),
                handlers,
                cx,
            ));

        // This is the one inset surface in the shell. Content columns remain
        // edge-to-edge; only the Telegram-style navigation rail floats five
        // points inside the native window.
        div()
            .w(metrics.rail_width)
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .pt(metrics.window_drag_height)
            .pb(metrics.rail_padding)
            .px(metrics.rail_padding)
            .rounded(corner_radius)
            .border_1()
            .border_color(theme.border._100)
            .bg(theme.bg._200)
            .overflow_hidden()
            .tab_group()
            .child(div().flex_1().min_h_0().overflow_hidden().child(top))
            .child(bottom)
    }

    fn divider(theme: &AppTheme) -> impl IntoElement {
        div()
            .w(theme.shell.rail_item_size)
            .h(theme.shell.rail_item_gap / 3.0)
            .my(theme.shell.rail_item_gap)
            .rounded(theme.radius._300)
            .bg(theme.border._200)
    }

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
    fn the_rail_presents_every_tab_in_the_navigation_order() {
        let presented: Vec<RootTab> = RailSection::ALL.iter().map(|s| s.tab).collect();

        assert_eq!(
            presented,
            RootTab::ALL.to_vec(),
            "the rail is a presentation of RootTab::ALL; a different order moves \
             the sections under the user's Cmd+1..n as well"
        );
    }

    #[test]
    fn the_phones_five_are_still_the_first_four_plus_the_profile() {
        let first_four: Vec<RootTab> = RailSection::ALL.iter().take(4).map(|s| s.tab).collect();
        assert_eq!(
            first_four,
            vec![
                RootTab::Feed,
                RootTab::Search,
                RootTab::Chats,
                RootTab::Shorts
            ]
        );
        assert_eq!(RailSection::footer().tab, RootTab::Profile);
    }

    #[test]
    fn the_two_unpacked_sections_are_the_references_own_more_entries() {
        let keys: Vec<&str> = RailSection::ALL
            .iter()
            .skip(4)
            .take(2)
            .map(|s| s.title_key)
            .collect();
        assert_eq!(
            keys,
            vec!["navbtn_vacancies", "navbtn_music"],
            "both keys exist in the reference's own catalogue; neither is invented here"
        );
    }

    #[test]
    fn the_profile_is_the_only_section_pinned_to_the_bottom() {
        assert_eq!(RailSection::primary().len(), RailSection::ALL.len() - 1);
        assert!(
            !RailSection::primary()
                .iter()
                .any(|section| section.tab == RootTab::Profile)
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
        assert!(IconUi::asset_path(Rail::SETTINGS_SYMBOL).is_some());
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
    fn every_tab_resolves_to_exactly_one_section() {
        for tab in RootTab::ALL {
            let matches = RailSection::ALL
                .iter()
                .filter(|section| section.tab == tab)
                .count();
            assert_eq!(matches, 1, "{tab:?} is presented {matches} times");
        }
    }

    #[test]
    fn settings_is_a_target_the_rail_can_address_without_being_a_tab() {
        assert_ne!(RailTarget::Settings, RailTarget::Section(RootTab::Profile));
    }
}
