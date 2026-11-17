use gpui::{
    AnyElement, App, Context, Div, Entity, FontWeight, IntoElement, Render, SharedString, WeakEntity,
    Window, div, prelude::*,
};
use rise_i18n::{tr, tr_plural};
use rise_theme::{AppTheme, Appearance, Density, Material, ThemePalette};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::flag_icon_ui::FlagIconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedEndAction};
use rise_ui::input_ui::{InputMode, InputUiState};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::{ButtonUi, IconSize, IconUi, MainText};

pub const LAUNCH_ARGUMENT: &str = "-riseStorybook";

pub fn requested_by(arguments: impl IntoIterator<Item = String>) -> bool {
    arguments
        .into_iter()
        .any(|argument| argument == LAUNCH_ARGUMENT)
}

pub struct Storybook {
    single_line: Entity<InputUiState>,
    multi_line: Entity<InputUiState>,
    secure: Entity<InputUiState>,
    selected_setting: SharedString,
}

impl Storybook {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let single_line = cx.new(|cx| InputUiState::new(InputMode::SingleLine, cx));
        let multi_line = cx.new(|cx| InputUiState::new(InputMode::MultiLine, cx));
        let secure = cx.new(|cx| InputUiState::new(InputMode::SingleLine, cx));

        single_line.update(cx, |state, cx| state.set_placeholder("Search", cx));
        multi_line.update(cx, |state, cx| state.set_placeholder("Message", cx));
        secure.update(cx, |state, cx| {
            state.set_placeholder("Password", cx);
            state.set_secure(true, cx);
        });

        Self {
            single_line,
            multi_line,
            secure,
            selected_setting: Self::SETTINGS_ROWS[2].0.into(),
        }
    }

    fn appearances() -> [(Appearance, ThemePalette); 2] {
        [
            (Appearance::Dark, ThemePalette::default_dark()),
            (Appearance::Light, ThemePalette::default_light()),
        ]
    }

    pub fn densities() -> [(&'static str, Density); 3] {
        [
            ("compact", Density::COMPACT),
            ("normal", Density::NORMAL),
            ("comfortable", Density::COMFORTABLE),
        ]
    }

    fn icon_sample() -> [&'static str; 8] {
        [
            "doc.text",
            "magnifyingglass",
            "bubble.left.and.bubble.right",
            "play.rectangle.on.rectangle",
            "ellipsis.circle",
            "heart.fill",
            "gearshape.fill",
            "paperplane.fill",
        ]
    }

    const SETTINGS_ROWS: [(&'static str, &'static str, &'static str); 5] = [
        ("profile", "person.fill", "more_tab_profile"),
        ("sessions", "iphone", "settings_sessions_title"),
        ("general", "gearshape.fill", "notification_settings_music_general"),
        ("privacy", "lock.fill", "settings_privacy_title"),
        ("chats", "bubble.left.and.bubble.right", "chats_title"),
    ];

    fn settings_chip(index: usize) -> gpui::Hsla {
        match index {
            0 => SettingsIconPalette::profile(),
            1 => SettingsIconPalette::sessions(),
            2 => SettingsIconPalette::customization(),
            3 => SettingsIconPalette::privacy(),
            _ => SettingsIconPalette::chat_folders(),
        }
    }

    fn select_row(
        storybook: &WeakEntity<Self>,
        id: &'static str,
    ) -> impl Fn(&mut Window, &mut App) + 'static + use<> {
        let storybook = storybook.clone();
        move |_window, cx| {
            storybook
                .update(cx, |storybook, cx| {
                    storybook.selected_setting = id.into();
                    cx.notify();
                })
                .ok();
        }
    }

    fn themed_group(group: GroupedBtns, theme: &AppTheme) -> GroupedBtns {
        group
            .surface_color(theme.bg._200)
            .primary_text_color(theme.text.primary)
            .secondary_text_color(theme.text.secondary)
            .separator_color(theme.border._100)
    }

    fn grouped(
        theme: &AppTheme,
        key: &SharedString,
        selected: &SharedString,
        storybook: &WeakEntity<Self>,
    ) -> Div {
        let card = GroupedBtns::new(
            SharedString::from(format!("{key}.card")),
            vec![
                GroupedBtn::new("profile", tr("more_tab_profile"))
                    .icon("person.fill")
                    .icon_background(SettingsIconPalette::profile())
                    .callback(Self::select_row(storybook, "profile")),
                GroupedBtn::new("sessions", tr("settings_sessions_title"))
                    .icon("iphone")
                    .icon_background(SettingsIconPalette::sessions())
                    .trailing("4")
                    .callback(Self::select_row(storybook, "sessions")),
                GroupedBtn::new("music", tr("more_tab_music"))
                    .icon("music.note")
                    .icon_background(SettingsIconPalette::memory_audio())
                    .subtitle(tr_plural("music_track_count", 128))
                    .callback(Self::select_row(storybook, "music")),
                GroupedBtn::new("blocked", tr("privacy_blocked_users_title"))
                    .icon("hand.raised.fill")
                    .icon_background(SettingsIconPalette::blocks())
                    .disabled(true)
                    .callback(Self::select_row(storybook, "blocked")),
            ],
        )
        .title(tr("settings_privacy_title").to_uppercase())
        .right_top_text("4")
        .end_group_title(tr("achievements_owner_only_badge"))
        .end_action(GroupedEndAction::new(
            tr("settings_delete_account_title"),
            Self::select_row(storybook, "delete_account"),
        ));
        let card = Self::themed_group(card, theme);

        let rows = Self::SETTINGS_ROWS
            .into_iter()
            .enumerate()
            .map(|(index, (id, symbol, label_key))| {
                let is_selected = id == selected.as_ref();
                let row = GroupedBtn::new(id, tr(label_key))
                    .icon(symbol)
                    .icon_background(Self::settings_chip(index))
                    .accessibility_selected(is_selected)
                    .callback(Self::select_row(storybook, id));
                if is_selected { row.trailing("selected") } else { row }
            })
            .collect();

        let panel = Self::themed_group(
            GroupedBtns::new(SharedString::from(format!("{key}.panel")), rows),
            theme,
        );

        Self::section(
            theme,
            "GroupedBtns — card, and selectable rows on a panel",
            vec![
                div()
                    .w(theme.shell.sidebar_width)
                    .child(card)
                    .into_any_element(),
                div()
                    .w(theme.shell.sidebar_width)
                    .py(theme.spacing._400)
                    .bg(theme.bg._100)
                    .rounded(theme.radius._300)
                    .child(panel)
                    .into_any_element(),
            ],
        )
    }

    fn section(theme: &AppTheme, title: &'static str, body: Vec<AnyElement>) -> Div {
        let heading = theme.typography.style(11.0, FontWeight::SEMIBOLD);

        div()
            .flex()
            .flex_col()
            .gap(theme.shell.rail_item_gap)
            .child(
                div()
                    .text_size(heading.size)
                    .font(heading.font)
                    .text_color(theme.text.secondary)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(theme.shell.rail_item_gap)
                    .children(body),
            )
    }

    fn buttons(theme: &AppTheme) -> Div {
        let fills = [None, Some(theme.button.bg_300), Some(theme.primary._300)];
        let mut row = Vec::new();
        for (index, fill) in fills.into_iter().enumerate() {
            for (state, (disabled, is_pending)) in [(false, false), (true, false), (false, true)].into_iter().enumerate() {
                let button = ButtonUi::new(("storybook.button", index * 3 + state))
                    .fit_content(true)
                    .padding_horizontal(f32::from(theme.spacing._900))
                    .disabled(disabled)
                    .is_pending(is_pending);
                let button = match fill {
                    Some(fill) => button.background_color(fill),
                    None => button,
                };
                row.push(button.child(MainText::new("Button")).into_any_element());
            }
        }
        Self::section(theme, "ButtonUi", row)
    }

    fn text(theme: &AppTheme) -> Div {
        Self::section(
            theme,
            "MainText / Typography",
            vec![
                MainText::new("Title").px(f32::from(theme.typography.large_title().size)).into_any_element(),
                MainText::new("Body primary").px(f32::from(theme.typography.body().size))
                    .into_any_element(),
                MainText::new("Body secondary").px(f32::from(theme.typography.body().size)).secondary(true)
                    .into_any_element(),
                Self::type_step(theme, "headline", theme.typography.headline()),
                Self::type_step(theme, "body_strong", theme.typography.body_strong()),
                Self::type_step(theme, "caption", theme.typography.caption()),
            ],
        )
    }

    fn type_step(
        theme: &AppTheme,
        name: &'static str,
        token: rise_theme::TextStyleToken,
    ) -> AnyElement {
        div()
            .text_size(token.size)
            .font(token.font)
            .text_color(theme.text.primary)
            .child(name)
            .into_any_element()
    }

    fn icons(theme: &AppTheme) -> Div {
        let mut row = Vec::new();
        for symbol in Self::icon_sample() {
            for size in [IconSize::Small, IconSize::Regular, IconSize::Large] {
                if let Some(icon) = IconUi::primary(theme, symbol, size) {
                    row.push(icon.into_any_element());
                }
            }
        }
        Self::section(theme, "IconUi (SF name -> Lucide)", row)
    }

    fn flags(theme: &AppTheme) -> Div {
        let row = ["\u{1F1F7}\u{1F1FA}", "\u{1F1FA}\u{1F1F8}", "\u{1F1E9}\u{1F1EA}", "\u{1F1E7}\u{1F1F7}", "\u{1F1F0}\u{1F1FF}", "\u{1F1F9}\u{1F1FC}"]
            .into_iter()
            .map(|flag| FlagIconUi::new(flag).into_any_element())
            .collect();
        Self::section(theme, "FlagIconUi", row)
    }

    fn surfaces(theme: &AppTheme) -> Div {
        let mut row = Vec::new();
        for material in Material::ALL {
            let painted = theme.painted_material(material);
            row.push(
                div()
                    .w(theme.shell.aside_width / 3.0)
                    .h(theme.button.height_100)
                    .bg(painted.fill)
                    .border_1()
                    .border_color(painted.border)
                    .rounded(painted.corner_radius)
                    .into_any_element(),
            );
        }
        row.push(
            div()
                .bg(theme.bg._200)
                .border_1()
                .border_color(theme.border._100)
                .rounded(theme.radius._300)
                .w(theme.shell.aside_width / 3.0)
                .h(theme.button.height_100)
                .into_any_element(),
        );
        Self::section(theme, "Materials (painted tier) + surface", row)
    }

    fn skeletons(theme: &AppTheme) -> Div {
        let shapes = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing._400)
            .child(SkeletonCircle::new(f32::from(theme.button.height_300)))
            .child(SkeletonLine::new(f32::from(theme.shell.aside_width / 2.0), f32::from(theme.spacing._400)))
            .child(SkeletonBlock::new(
                f32::from(theme.shell.aside_width / 3.0),
                f32::from(theme.button.height_100),
                f32::from(theme.radius._300),
            ))
            .skeleton_shimmer(None);

        Self::section(theme, "SkeletonUi", vec![shapes.into_any_element()])
    }

    fn inputs(theme: &AppTheme, storybook: &Self) -> Div {
        Self::section(
            theme,
            "InputUi — installed theme only, and actually typeable",
            vec![
                div()
                    .w(theme.shell.sidebar_width)
                    .child(storybook.single_line.clone())
                    .into_any_element(),
                div()
                    .w(theme.shell.sidebar_width)
                    .child(storybook.secure.clone())
                    .into_any_element(),
                div()
                    .w(theme.shell.sidebar_width)
                    .h(theme.button.height_100 * 2.0)
                    .child(storybook.multi_line.clone())
                    .into_any_element(),
            ],
        )
    }

    fn column(
        appearance: Appearance,
        palette: &ThemePalette,
        density: Density,
        selected: &SharedString,
        storybook: &WeakEntity<Self>,
    ) -> Div {
        let theme = AppTheme::new(palette, appearance, density);
        let label = theme.typography.style(11.0, FontWeight::BOLD);
        let name = Self::densities()
            .into_iter()
            .find(|(_, d)| *d == density)
            .map(|(name, _)| name)
            .unwrap_or("custom");
        let key = SharedString::from(format!("storybook.grouped.{appearance:?}.{name}"));

        div()
            .flex()
            .flex_col()
            .gap(theme.button.height_600 / 2.0)
            .p(theme.button.height_600 / 2.0)
            .bg(theme.bg._000)
            .rounded(theme.radius._300)
            .border_1()
            .border_color(theme.border._100)
            .child(
                div()
                    .text_size(label.size)
                    .font(label.font)
                    .text_color(theme.text.primary)
                    .child(SharedString::from(format!("{appearance:?} · {name}"))),
            )
            .child(Self::text(&theme))
            .child(Self::buttons(&theme))
            .child(Self::icons(&theme))
            .child(Self::flags(&theme))
            .child(Self::surfaces(&theme))
            .child(Self::skeletons(&theme))
            .child(Self::grouped(&theme, &key, selected, storybook))
    }
}

impl Render for Storybook {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let installed = rise_ui::theme(cx as &gpui::App).clone();
        let gap = installed.button.height_600 / 2.0;

        let mut grid = div()
            .size_full()
            .text_color(installed.text.primary)
            .bg(installed.bg._100)
            .id("storybook")
            .overflow_scroll()
            .flex()
            .flex_col()
            .gap(gap)
            .p(gap)
            .child(Self::inputs(&installed, self));

        let selected = self.selected_setting.clone();
        let storybook = cx.weak_entity();

        for (appearance, palette) in Self::appearances() {
            let mut row = div().flex().flex_row().items_start().gap(gap);
            for (_, density) in Self::densities() {
                row = row.child(Self::column(
                    appearance, &palette, density, &selected, &storybook,
                ));
            }
            grid = grid.child(row);
        }

        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_platform::HostOs;

    const FRIENDLY_FLAG: &str = "--design";

    fn posix_shell_available() -> bool {
        HostOs::current() != HostOs::Windows
    }

    fn translate(arguments: &str) -> Vec<String> {
        let script =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/dev-args.sh");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "set -- {arguments}; . \"{}\"; for a in \"$@\"; do printf '%s\\n' \"$a\"; done",
                script.display()
            ))
            .output()
            .unwrap_or_else(|error| panic!("{}: {error}", script.display()));

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_friendly_flag_translates_to_the_argument_this_module_reads() {
        if !posix_shell_available() {
            return;
        }
        assert_eq!(translate(FRIENDLY_FLAG), vec![LAUNCH_ARGUMENT.to_owned()]);
    }

    #[test]
    fn every_other_argument_reaches_the_app_untouched() {
        if !posix_shell_available() {
            return;
        }
        assert!(
            translate("").is_empty(),
            "a plain `cargo make dev-macos` must launch with no arguments at all"
        );
        assert_eq!(
            translate("--design -riseUITestReset"),
            vec![LAUNCH_ARGUMENT.to_owned(), "-riseUITestReset".to_owned()],
            "a flag with no friendly name yet still has to pass through"
        );
        assert_eq!(
            translate("'a b'"),
            vec!["a b".to_owned()],
            "the rewrite must not split an argument on its spaces"
        );
    }

    #[test]
    fn the_storybook_opens_only_when_it_is_asked_for() {
        assert!(requested_by([LAUNCH_ARGUMENT.to_owned()]));
        assert!(requested_by([
            "--other".to_owned(),
            LAUNCH_ARGUMENT.to_owned()
        ]));
        assert!(!requested_by(["-riseStorybookish".to_owned()]));
        assert!(!requested_by(Vec::new()));
    }

    #[test]
    fn the_gate_asks_for_three_densities_and_both_appearances() {
        assert_eq!(
            Storybook::appearances().len() * Storybook::densities().len(),
            6
        );
    }

    #[test]
    fn the_densities_are_distinct_and_ordered() {
        let multipliers: Vec<f32> = Storybook::densities()
            .iter()
            .map(|(_, d)| d.multiplier())
            .collect();

        assert_eq!(multipliers.len(), 3);
        assert!(multipliers[0] < multipliers[1]);
        assert!(multipliers[1] < multipliers[2]);
    }

    #[test]
    fn every_sampled_icon_is_one_the_bundle_carries() {
        for symbol in Storybook::icon_sample() {
            assert!(
                IconUi::asset_path(symbol).is_some(),
                "{symbol} would render as a gap in the storybook"
            );
        }
    }
}
