use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, Div, ElementId, FontWeight, Hsla, IntoElement, Pixels,
    SharedString, Window, div, prelude::*,
};
use rise_theme::AppTheme;
use rise_ui::IconUi;

use super::grouped_chrome::{
    GroupColors, GroupSurface, IconChrome, RowChrome, RowPosition, RowState, card_background,
    draws_separator_after, end_separator_color, group_padding_x, icon_chrome, row_chrome,
    separator_color,
};

const END_ACTION_SYMBOL: &str = "trash";

const CHEVRON_SYMBOL: &str = "chevron.right";

/// Lucide has no filled variants, so the fill intent is read off the SF name.
const SF_FILLED_SUFFIX: &str = ".fill";

/// What happens when a row is activated, addressed by the row's stable id.
pub type GroupedActivate<V> = Rc<dyn Fn(&mut V, SharedString, &mut Window, &mut Context<V>)>;

/// One row of a settings group. `id` is the stable action id an activation
/// carries and the row's [`ElementId`] is built from — never a position, since
/// gpui keys element state off it.
pub struct GroupedBtn {
    pub id: SharedString,
    sf_symbol: Option<&'static str>,
    icon_background: Option<Hsla>,
    icon_foreground: Option<Hsla>,
    leading: Option<AnyElement>,
    pretitle: Option<SharedString>,
    text: SharedString,
    subtitle: Option<SharedString>,
    text_line_limit: usize,
    subtitle_line_limit: usize,
    trailing: Option<SharedString>,
    trailing_slot: Option<AnyElement>,
    content: Option<AnyElement>,
    expanded: Option<AnyElement>,
    destructive: bool,
    disabled: bool,
    selected: bool,
    show_chevron: bool,
    show_chevron_when_trailing_slot: bool,
}

impl GroupedBtn {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            sf_symbol: None,
            icon_background: None,
            icon_foreground: None,
            leading: None,
            pretitle: None,
            text: text.into(),
            subtitle: None,
            text_line_limit: 0,
            subtitle_line_limit: 0,
            trailing: None,
            trailing_slot: None,
            content: None,
            expanded: None,
            destructive: false,
            disabled: false,
            selected: false,
            show_chevron: true,
            show_chevron_when_trailing_slot: false,
        }
    }

    /// An SF Symbol on a coloured chip; [`rise_theme::SettingsIconPalette`]
    /// carries the palette.
    pub fn icon(mut self, sf_symbol: &'static str, background: Hsla) -> Self {
        self.sf_symbol = Some(sf_symbol);
        self.icon_background = Some(background);
        self
    }

    /// The same chip, at the accent the theme picks rather than a named colour.
    pub fn themed_icon(mut self, sf_symbol: &'static str) -> Self {
        self.sf_symbol = Some(sf_symbol);
        self
    }

    pub fn icon_foreground(mut self, color: Hsla) -> Self {
        self.icon_foreground = Some(color);
        self
    }

    /// Anything at all in place of the chip: an avatar, a flag, a swatch.
    pub fn leading(mut self, slot: impl IntoElement) -> Self {
        self.leading = Some(slot.into_any_element());
        self
    }

    /// A small line above the title — a category, a role, a group name.
    pub fn pretitle(mut self, pretitle: impl Into<SharedString>) -> Self {
        self.pretitle = Some(pretitle.into());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// `0` means "as many lines as it takes", not "none".
    pub fn text_line_limit(mut self, lines: usize) -> Self {
        self.text_line_limit = lines;
        self
    }

    pub fn subtitle_line_limit(mut self, lines: usize) -> Self {
        self.subtitle_line_limit = lines;
        self
    }

    /// The current value, on the right: "Русский", "4", "On".
    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    /// A control on the right instead of a value — a switch, an emoji, a badge.
    pub fn trailing_slot(mut self, slot: impl IntoElement) -> Self {
        self.trailing_slot = Some(slot.into_any_element());
        self
    }

    /// Keeps the chevron beside a trailing control. Off by default.
    pub fn show_chevron_when_trailing_slot(mut self) -> Self {
        self.show_chevron_when_trailing_slot = true;
        self
    }

    pub fn hide_chevron(mut self) -> Self {
        self.show_chevron = false;
        self
    }

    /// Replaces the whole row. It keeps the group's background and its disabled
    /// state and gets nothing else — no padding, no click, no chevron.
    pub fn content(mut self, slot: impl IntoElement) -> Self {
        self.content = Some(slot.into_any_element());
        self
    }

    /// Drawn under the row and inside the same group: a slider, a picker, a
    /// nested list.
    pub fn expanded(mut self, slot: impl IntoElement) -> Self {
        self.expanded = Some(slot.into_any_element());
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// The row the screen is currently showing.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the row ends in a chevron. Derived, so it does not depend on the
    /// order the builder was called in.
    pub fn shows_chevron(&self) -> bool {
        if self.trailing_slot.is_some() {
            return self.show_chevron_when_trailing_slot;
        }
        self.show_chevron
    }

    /// Whether the pointer may activate this row at all. A content slot owns its
    /// own interior, so the row around it takes no clicks.
    pub fn is_activatable(&self) -> bool {
        !self.disabled && self.content.is_none()
    }

    pub fn state(&self, surface: GroupSurface, position: RowPosition) -> RowState {
        RowState {
            surface,
            position,
            selected: self.selected,
            destructive: self.destructive,
            disabled: self.disabled,
        }
    }
}

/// The destructive action a group can end with — "Delete chat", "Leave". It
/// activates through the same handler as every other row.
pub struct GroupedEndAction {
    pub id: SharedString,
    pub title: SharedString,
    pub disabled: bool,
}

impl GroupedEndAction {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// A group of settings rows.
///
/// Draws a card by default — rows on a lifted rectangle with hairlines between
/// them; [`GroupedBtns::without_wrapper`] drops it and lets the rows run edge to
/// edge. Stateless: the screen above it owns which row is selected.
pub struct GroupedBtns {
    id: ElementId,
    items: Vec<GroupedBtn>,
    title: Option<SharedString>,
    right_top_text: Option<SharedString>,
    end_group_title: Option<SharedString>,
    end_group_title_size: Option<f32>,
    end_action: Option<GroupedEndAction>,
    surface: GroupSurface,
    colors: GroupColors,
}

impl GroupedBtns {
    /// `id` scopes every row's [`ElementId`], so two groups on one screen may
    /// repeat an item id — gpui drops a duplicate id silently in release.
    pub fn new(id: impl Into<ElementId>, items: Vec<GroupedBtn>) -> Self {
        Self {
            id: id.into(),
            items,
            title: None,
            right_top_text: None,
            end_group_title: None,
            end_group_title_size: None,
            end_action: None,
            surface: GroupSurface::Card,
            colors: GroupColors::default(),
        }
    }

    /// The heading above the group. Already translated — this component never
    /// calls `tr` itself.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A value on the heading's right — a count, a state.
    pub fn right_top_text(mut self, text: impl Into<SharedString>) -> Self {
        self.right_top_text = Some(text.into());
        self
    }

    /// The footnote under the group, explaining what the rows above it do.
    pub fn end_group_title(mut self, title: impl Into<SharedString>) -> Self {
        self.end_group_title = Some(title.into());
        self
    }

    pub fn end_group_title_size(mut self, size_pt: f32) -> Self {
        self.end_group_title_size = Some(size_pt);
        self
    }

    pub fn end_action(mut self, action: GroupedEndAction) -> Self {
        self.end_action = Some(action);
        self
    }

    /// Drops the card: no fill, no corner, no hairlines, and the rows run to the
    /// edge of whatever holds them.
    pub fn without_wrapper(mut self) -> Self {
        self.surface = GroupSurface::Bare;
        self
    }

    pub fn surface_color(mut self, color: Hsla) -> Self {
        self.colors.surface = Some(color);
        self
    }

    pub fn primary_text_color(mut self, color: Hsla) -> Self {
        self.colors.primary_text = Some(color);
        self
    }

    pub fn secondary_text_color(mut self, color: Hsla) -> Self {
        self.colors.secondary_text = Some(color);
        self
    }

    pub fn separator_color(mut self, color: Hsla) -> Self {
        self.colors.separator = Some(color);
        self
    }

    pub fn surface(&self) -> GroupSurface {
        self.surface
    }

    pub fn items(&self) -> &[GroupedBtn] {
        &self.items
    }

    pub fn render<V: 'static>(
        self,
        theme: &AppTheme,
        activate: &GroupedActivate<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.grouped;
        let surface = self.surface;
        let colors = self.colors;
        let group_id = self.id;
        let items = self.items;
        let count = items.len();
        let has_end_action = self.end_action.is_some();

        let mut card =
            div()
                .id(group_id.clone())
                .w_full()
                .flex()
                .flex_col()
                .when(surface.is_card(), |card| {
                    card.bg(card_background(theme, colors))
                        .rounded(metrics.card_radius)
                        // A gpui ContentMask is a Bounds with no corner radius, so
                        // this clips the rectangle only; rows round their own ends.
                        .overflow_hidden()
                });

        for (index, item) in items.into_iter().enumerate() {
            let position = RowPosition::of(index, count, has_end_action);
            let state = item.state(surface, position);
            let chrome = row_chrome(theme, colors, state);
            let row_id = ElementId::from((group_id.clone(), item.id.clone()));

            card = card.child(Self::row(theme, item, chrome, state, row_id, activate, cx));

            if draws_separator_after(surface, index, count) {
                card = card.child(Self::separator(
                    theme,
                    separator_color(theme, colors),
                    metrics.separator_leading_padding,
                ));
            }
        }

        if let Some(end) = self.end_action {
            card = card
                .child(Self::separator(
                    theme,
                    end_separator_color(theme, colors),
                    Pixels::ZERO,
                ))
                .child(Self::end_action_row(
                    theme, end, colors, surface, &group_id, activate, cx,
                ));
        }

        let mut group = div().w_full().flex().flex_col().gap(metrics.header_gap);

        if let Some(heading) = Self::heading(theme, self.title, self.right_top_text, colors) {
            group = group.child(heading);
        }

        group = group.child(
            div()
                .w_full()
                .px(group_padding_x(&metrics, surface))
                .child(card),
        );

        if let Some(footnote) = self.end_group_title {
            let token = theme.typography.style(
                self.end_group_title_size.unwrap_or(metrics.footer_size),
                FontWeight::NORMAL,
            );

            group = group.child(
                div()
                    .w_full()
                    .pl(metrics.footer_padding_left)
                    .pt(metrics.footer_padding_top)
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(colors.secondary_text.unwrap_or(theme.text.secondary))
                    .child(footnote),
            );
        }

        group
    }

    fn heading(
        theme: &AppTheme,
        title: Option<SharedString>,
        right_top_text: Option<SharedString>,
        colors: GroupColors,
    ) -> Option<Div> {
        let title = title?;
        let metrics = theme.grouped;
        let token = theme
            .typography
            .style(metrics.title_size, FontWeight::NORMAL);

        let mut row = div()
            .w_full()
            .px(metrics.header_padding_x)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .text_size(token.size)
            .line_height(token.line_height)
            .font(token.font)
            .text_color(colors.secondary_text.unwrap_or(theme.text.secondary))
            .child(div().child(title));

        if let Some(right) = right_top_text {
            row = row.child(div().child(right));
        }

        Some(row)
    }

    fn separator(theme: &AppTheme, color: Hsla, leading: Pixels) -> Div {
        div()
            .w_full()
            .pl(leading)
            .child(div().w_full().h(theme.grouped.separator_height).bg(color))
    }

    fn row<V: 'static>(
        theme: &AppTheme,
        mut item: GroupedBtn,
        chrome: RowChrome,
        state: RowState,
        row_id: ElementId,
        activate: &GroupedActivate<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let mut stack = div().w_full().flex().flex_col();

        if let Some(content) = item.content.take() {
            stack = stack.child(
                div()
                    .w_full()
                    .when_some(chrome.background, |slot, background| slot.bg(background))
                    .opacity(chrome.opacity)
                    .child(content),
            );
        } else {
            stack = stack.child(Self::button(
                theme, &mut item, chrome, state, row_id, activate, cx,
            ));
        }

        if let Some(expanded) = item.expanded.take() {
            stack = stack.child(
                div()
                    .w_full()
                    .when_some(chrome.background, |slot, background| slot.bg(background))
                    .opacity(chrome.opacity)
                    .child(expanded),
            );
        }

        stack
    }

    fn button<V: 'static>(
        theme: &AppTheme,
        item: &mut GroupedBtn,
        chrome: RowChrome,
        state: RowState,
        row_id: ElementId,
        activate: &GroupedActivate<V>,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let metrics = theme.grouped;
        let shows_chevron = item.shows_chevron();
        let is_activatable = !item.disabled;

        let mut row = div()
            .id(row_id)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.row_gap)
            .px(metrics.row_padding_x)
            .py(metrics.row_padding_y)
            .rounded_t(chrome.corners.top)
            .rounded_b(chrome.corners.bottom)
            .opacity(chrome.opacity)
            .when_some(chrome.background, |row, background| row.bg(background))
            .when_some(chrome.hover_background, |row, hovered| {
                row.hover(move |row| row.bg(hovered))
            })
            .child(Self::leading(theme, item, state))
            .child(Self::text_column(theme, item, chrome));

        if let Some(slot) = item.trailing_slot.take() {
            row = row.child(div().flex_shrink_0().child(slot));
        } else if let Some(trailing) = item.trailing.take() {
            let token = theme
                .typography
                .style(metrics.trailing_size, FontWeight::NORMAL);

            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(chrome.secondary)
                    .child(trailing),
            );
        }

        if shows_chevron
            && let Some(chevron) =
                IconUi::sized(CHEVRON_SYMBOL, metrics.chevron, chrome.chevron, false)
        {
            row = row.child(div().flex_shrink_0().child(chevron));
        }

        if is_activatable {
            let activate = Rc::clone(activate);
            let id = item.id.clone();

            row = row.on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                activate(view, id.clone(), window, cx);
            }));
        }

        row
    }

    /// The empty case still reserves the chip's height, so an iconless group
    /// keeps the same row height as one with icons.
    fn leading(theme: &AppTheme, item: &mut GroupedBtn, state: RowState) -> Div {
        let metrics = theme.grouped;

        if let Some(slot) = item.leading.take() {
            return div().flex_shrink_0().child(slot);
        }

        let Some(symbol) = item.sf_symbol else {
            return div().h(metrics.icon_side).flex_shrink_0();
        };

        let IconChrome { background, glyph } =
            icon_chrome(theme, state, item.icon_background, item.icon_foreground);

        let mut chip = div()
            .size(metrics.icon_side)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(metrics.icon_radius)
            .bg(background);

        let filled = symbol.ends_with(SF_FILLED_SUFFIX);
        if let Some(icon) = IconUi::sized(symbol, metrics.icon_glyph, glyph, filled) {
            chip = chip.child(icon);
        }

        chip
    }

    fn text_column(theme: &AppTheme, item: &mut GroupedBtn, chrome: RowChrome) -> Div {
        let metrics = theme.grouped;

        // flex_1's zero basis is what makes a long label wrap instead of pushing
        // the trailing value off the row.
        let mut column = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(metrics.text_gap)
            .overflow_hidden();

        if let Some(pretitle) = item.pretitle.take() {
            let token = theme
                .typography
                .style(metrics.pretitle_size, FontWeight::NORMAL);

            column = column.child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(chrome.secondary)
                    .child(pretitle),
            );
        }

        let token = theme
            .typography
            .style(metrics.text_size, FontWeight::NORMAL);
        column = column.child(
            div()
                .text_size(token.size)
                .line_height(token.line_height)
                .font(token.font)
                .text_color(chrome.label)
                .when(item.text_line_limit > 0, |label| {
                    label.line_clamp(item.text_line_limit)
                })
                .child(item.text.clone()),
        );

        if let Some(subtitle) = item.subtitle.take() {
            let token = theme
                .typography
                .style(metrics.subtitle_size, FontWeight::NORMAL);

            column = column.child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(chrome.secondary)
                    .when(item.subtitle_line_limit > 0, |label| {
                        label.line_clamp(item.subtitle_line_limit)
                    })
                    .child(subtitle),
            );
        }

        column
    }

    fn end_action_row<V: 'static>(
        theme: &AppTheme,
        end: GroupedEndAction,
        colors: GroupColors,
        surface: GroupSurface,
        group_id: &ElementId,
        activate: &GroupedActivate<V>,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let metrics = theme.grouped;
        let state = RowState {
            surface,
            position: RowPosition::Last,
            selected: false,
            destructive: true,
            disabled: end.disabled,
        };
        let chrome = row_chrome(theme, colors, state);
        let token = theme
            .typography
            .style(metrics.end_title_size, FontWeight::SEMIBOLD);

        let mut row = div()
            .id(ElementId::from((group_id.clone(), end.id.clone())))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.end_gap)
            .px(metrics.row_padding_x)
            .py(metrics.end_padding_y)
            .rounded_t(chrome.corners.top)
            .rounded_b(chrome.corners.bottom)
            .opacity(chrome.opacity)
            .when_some(chrome.background, |row, background| row.bg(background))
            .when_some(chrome.hover_background, |row, hovered| {
                row.hover(move |row| row.bg(hovered))
            });

        if let Some(icon) = IconUi::sized(
            END_ACTION_SYMBOL,
            metrics.end_icon,
            theme.semantic.error_100,
            false,
        ) {
            row = row.child(div().flex_shrink_0().child(icon));
        }

        row = row.child(
            div()
                .flex_1()
                .text_size(token.size)
                .line_height(token.line_height)
                .font(token.font)
                .text_color(theme.semantic.error_100)
                .child(end.title),
        );

        if !end.disabled {
            let activate = Rc::clone(activate);

            row = row.on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                activate(view, end.id.clone(), window, cx);
            }));
        }

        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_theme::SettingsIconPalette;

    fn item() -> GroupedBtn {
        GroupedBtn::new("profile", "Мой профиль")
    }

    #[test]
    fn a_plain_row_ends_in_a_chevron() {
        assert!(item().shows_chevron());
    }

    #[test]
    fn a_trailing_control_takes_the_chevrons_place_unless_it_is_asked_not_to() {
        assert!(
            !item().trailing_slot(div()).shows_chevron(),
            "a switch on the right means the row toggles rather than navigates"
        );
        assert!(
            item()
                .trailing_slot(div())
                .show_chevron_when_trailing_slot()
                .shows_chevron()
        );
    }

    #[test]
    fn the_chevron_rule_does_not_depend_on_the_order_it_was_built_in() {
        let slot_last = item()
            .show_chevron_when_trailing_slot()
            .trailing_slot(div());
        let slot_first = item()
            .trailing_slot(div())
            .show_chevron_when_trailing_slot();

        assert_eq!(slot_last.shows_chevron(), slot_first.shows_chevron());
        assert!(slot_last.shows_chevron());
    }

    #[test]
    fn a_trailing_value_is_not_a_trailing_control() {
        assert!(
            item().trailing("Русский").shows_chevron(),
            "a value on the right still points at a screen behind it"
        );
    }

    #[test]
    fn a_hidden_chevron_stays_hidden() {
        assert!(!item().hide_chevron().shows_chevron());
    }

    #[test]
    fn a_disabled_row_and_a_content_row_take_no_clicks() {
        assert!(item().is_activatable());
        assert!(!item().disabled().is_activatable());
        assert!(
            !item().content(div()).is_activatable(),
            "a content slot owns its own interior"
        );
    }

    #[test]
    fn a_rows_state_carries_the_groups_surface_with_it() {
        let selected = item().selected(true).destructive().disabled();

        let card = selected.state(GroupSurface::Card, RowPosition::First);
        assert_eq!(card.surface, GroupSurface::Card);
        assert_eq!(card.position, RowPosition::First);
        assert!(card.selected && card.destructive && card.disabled);

        assert_eq!(
            selected
                .state(GroupSurface::Bare, RowPosition::Last)
                .surface,
            GroupSurface::Bare
        );
    }

    #[test]
    fn a_group_draws_its_card_until_the_wrapper_is_taken_away() {
        let group = GroupedBtns::new("settings.profile", vec![item()]);
        assert_eq!(group.surface(), GroupSurface::Card);

        let bare = GroupedBtns::new("settings.profile", vec![item()]).without_wrapper();
        assert_eq!(bare.surface(), GroupSurface::Bare);
    }

    #[test]
    fn selection_survives_taking_the_wrapper_away_and_putting_it_back() {
        let group = GroupedBtns::new(
            "settings.security",
            vec![item().selected(true), GroupedBtn::new("wallet", "Кошелёк")],
        );

        assert!(group.items()[0].selected);
        assert!(!group.items()[1].selected);

        let bare = GroupedBtns::new(
            "settings.security",
            vec![item().selected(true), GroupedBtn::new("wallet", "Кошелёк")],
        )
        .without_wrapper();

        assert!(
            bare.items()[0].selected,
            "the two modes share one selection; they are not two components"
        );
    }

    #[test]
    fn a_row_id_is_scoped_by_its_group_so_two_groups_may_repeat_one() {
        let first = ElementId::from((ElementId::Name("settings.a".into()), "profile"));
        let second = ElementId::from((ElementId::Name("settings.b".into()), "profile"));

        assert_ne!(
            first, second,
            "an unscoped duplicate is dropped silently in release"
        );
    }

    #[test]
    fn an_end_action_is_enabled_until_it_is_not() {
        let end = GroupedEndAction::new("delete_chat", "Удалить чат");
        assert!(!end.disabled);
        assert!(
            GroupedEndAction::new("delete_chat", "Удалить чат")
                .disabled()
                .disabled
        );
    }

    #[test]
    fn the_fill_a_row_draws_comes_from_the_symbols_own_name() {
        assert!("person.fill".ends_with(SF_FILLED_SUFFIX));
        assert!(!"globe".ends_with(SF_FILLED_SUFFIX));
        assert!(!CHEVRON_SYMBOL.ends_with(SF_FILLED_SUFFIX));
        assert!(!END_ACTION_SYMBOL.ends_with(SF_FILLED_SUFFIX));
    }

    #[test]
    fn every_symbol_this_component_names_itself_is_one_the_bundle_carries() {
        for symbol in [CHEVRON_SYMBOL, END_ACTION_SYMBOL] {
            assert!(
                IconUi::asset_path(symbol).is_some(),
                "{symbol} would draw as a gap in every group"
            );
        }
    }

    #[test]
    fn an_icon_keeps_the_colour_it_was_handed() {
        let row = item().icon("person.fill", SettingsIconPalette::profile());

        assert_eq!(row.sf_symbol, Some("person.fill"));
        assert_eq!(row.icon_background, Some(SettingsIconPalette::profile()));
    }
}
