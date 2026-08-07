use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, Render, SharedString, Window, div,
    prelude::*,
};
use rise_theme::{AppTheme, Appearance, Density, Material, ThemePalette};
use rise_ui::animation::ShimmerBand;
use rise_ui::input_ui::{InputMode, InputUiState};
use rise_ui::{
    BoxUi, ButtonSize, ButtonTone, ButtonUi, FlagUi, IconSize, IconUi, MainText, SkeletonShape,
    SkeletonUi, TextTone,
};

/// The launch argument that opens the storybook instead of the app.
///
/// A launch argument and not a route: the reference gates its own developer
/// surfaces the same way (`-riseChatPerformanceTools`), and a `RootRoute`
/// variant would put a developer screen in the product's exhaustive route
/// match forever.
pub const LAUNCH_ARGUMENT: &str = "-riseStorybook";

pub fn requested_by(arguments: impl IntoIterator<Item = String>) -> bool {
    arguments
        .into_iter()
        .any(|argument| argument == LAUNCH_ARGUMENT)
}

/// Every component, at both appearances and all three densities.
///
/// This is the Phase 5 gate made visible. It builds its themes locally rather
/// than switching the installed global, so light and dark stand side by side in
/// one window and a difference between them is a difference you can see rather
/// than one you have to remember across a restart.
pub struct Storybook {
    single_line: Entity<InputUiState>,
    multi_line: Entity<InputUiState>,
    secure: Entity<InputUiState>,
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

    /// A sample rather than all 229 icons: the storybook is for judging the
    /// design, and a wall of glyphs is a stress test, not a review. The
    /// approximate ones are shown deliberately, because those are the ones a
    /// designer has to sign off.
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
        let mut row = Vec::new();
        for tone in [ButtonTone::Primary, ButtonTone::Neutral] {
            for size in [ButtonSize::Large, ButtonSize::Regular, ButtonSize::Small] {
                row.push(
                    ButtonUi::base(theme, tone, size)
                        .child("Button")
                        .into_any_element(),
                );
            }
        }
        Self::section(theme, "ButtonUi", row)
    }

    fn text(theme: &AppTheme) -> Div {
        Self::section(
            theme,
            "MainText / Typography",
            vec![
                MainText::title(theme).child("Title").into_any_element(),
                MainText::body(theme, TextTone::Primary)
                    .child("Body primary")
                    .into_any_element(),
                MainText::body(theme, TextTone::Secondary)
                    .child("Body secondary")
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
        let width = theme.icon.large;
        let mut row = Vec::new();
        for region in ["RU", "US", "DE", "BR", "KZ", "TW"] {
            row.push(FlagUi::render(theme, region, width));
        }
        // The fallback: a region with no shipped flag must read as its own two
        // letters, never as a blank or a pair of boxes.
        row.push(FlagUi::render(theme, "ZZ", width));
        Self::section(theme, "FlagUi", row)
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
            BoxUi::surface(theme)
                .w(theme.shell.aside_width / 3.0)
                .h(theme.button.height_100)
                .into_any_element(),
        );
        Self::section(theme, "Materials (painted tier) + BoxUi", row)
    }

    fn skeletons(theme: &AppTheme) -> Div {
        let band = ShimmerBand {
            leading: 0.2,
            trailing: 0.65,
        };

        Self::section(
            theme,
            "SkeletonUi",
            vec![
                SkeletonUi::circle(theme, band, theme.button.height_300).into_any_element(),
                SkeletonUi::line(theme, band, theme.shell.aside_width / 2.0).into_any_element(),
                SkeletonUi::shape(
                    theme,
                    band,
                    SkeletonShape::Rect {
                        width: theme.shell.aside_width / 3.0,
                        height: theme.button.height_100,
                        radius: theme.radius._300,
                    },
                )
                .into_any_element(),
            ],
        )
    }

    /// The inputs stand outside the six-column grid.
    ///
    /// `InputUiState` is a live entity that reads the INSTALLED theme, not one
    /// handed to it, so it cannot be shown six times at six different
    /// appearances the way a stateless component can. Showing one row of real,
    /// typeable fields at the installed theme is worth more than six dead
    /// screenshots of a field, and the storybook says which it is rather than
    /// letting the row look like part of the matrix.
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

    fn column(appearance: Appearance, palette: &ThemePalette, density: Density) -> Div {
        let theme = AppTheme::new(palette, appearance, density);
        let label = theme.typography.style(11.0, FontWeight::BOLD);
        let name = Self::densities()
            .into_iter()
            .find(|(_, d)| *d == density)
            .map(|(name, _)| name)
            .unwrap_or("custom");

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
    }
}

impl Render for Storybook {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let installed = rise_ui::theme(cx as &gpui::App).clone();
        let gap = installed.button.height_600 / 2.0;

        let mut grid = BoxUi::screen(&installed)
            .id("storybook")
            .overflow_scroll()
            .flex()
            .flex_col()
            .gap(gap)
            .p(gap)
            .child(Self::inputs(&installed, self));

        for (appearance, palette) in Self::appearances() {
            let mut row = div().flex().flex_row().items_start().gap(gap);
            for (_, density) in Self::densities() {
                row = row.child(Self::column(appearance, &palette, density));
            }
            grid = grid.child(row);
        }

        grid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_flag_sample_exercises_both_the_asset_and_the_fallback() {
        assert!(FlagUi::is_shipped("RU"));
        assert!(
            !FlagUi::is_shipped("ZZ"),
            "the storybook needs one region with no flag, or the fallback is never seen"
        );
    }
}
