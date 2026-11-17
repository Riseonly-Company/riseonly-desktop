use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::i18n::tr;
use rise_theme::{AppTheme, ThemeStore};

use crate::IconUi;
use crate::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use crate::main_text::{MainText, TextAlignT};
use crate::swiftui::button_style::plain;
use crate::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use crate::swiftui::shapes::{Shape, SwiftUiShapes};
use crate::swiftui::stacks::{Alignment, HorizontalAlignment, StackAlign, hstack, spacer, spacer_min_w, vstack};

const END_ACTION_SYMBOL: &str = "trash";
const DISABLED_OPACITY: f32 = 0.42;

/// settings card geometry shared by GroupedBtns and blocks stacked in the same column
/// without one source the same list gets two corner radii and two side insets
pub struct GroupedCardMetrics;

impl GroupedCardMetrics {
    pub const CORNER_RADIUS: f32 = 20.0;
    pub const HORIZONTAL_PADDING: f32 = 12.0;
    pub const TITLE_HORIZONTAL_PADDING: f32 = 16.0;

    pub const ROW_HORIZONTAL_PADDING: f32 = 16.0;
    pub const ROW_VERTICAL_PADDING: f32 = 12.0;
    pub const ICON_SIDE: f32 = 32.0;
    pub const ICON_SPACING: f32 = 12.0;

    /// where row text starts and so where the separator must start
    /// two separate numbers shifted rows against their separators
    pub const fn content_leading_inset() -> f32 {
        Self::ROW_HORIZONTAL_PADDING + Self::ICON_SIDE + Self::ICON_SPACING
    }

    pub const SHAPE: Shape = Shape::continuous(Self::CORNER_RADIUS);
}

pub struct GroupedBtn {
    pub id: SharedString,
    pub icon: Option<SharedString>,
    pub icon_background: Option<Hsla>,
    pub icon_foreground: Option<Hsla>,
    pub leading_slot: Option<AnyElement>,
    pub pretitle: Option<SharedString>,
    pub text: SharedString,
    pub subtitle: Option<SharedString>,
    pub text_line_limit: usize,
    pub subtitle_line_limit: usize,
    pub trailing: Option<SharedString>,
    pub destructive: bool,
    pub disabled: bool,
    pub accessibility_value: Option<SharedString>,
    pub accessibility_selected: bool,
    pub selected: bool,
    pub guide_anchor: Option<RiseGuideAnchor>,
    show_chevron: bool,
    pub trailing_slot: Option<AnyElement>,
    pub show_chevron_when_trailing_slot: bool,
    pub trailing_keeps_width: bool,
    pub content_slot: Option<AnyElement>,
    pub expanded_slot: Option<AnyElement>,
    pub callback: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl GroupedBtn {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            icon_background: None,
            icon_foreground: None,
            leading_slot: None,
            pretitle: None,
            text: text.into(),
            subtitle: None,
            text_line_limit: 0,
            subtitle_line_limit: 0,
            trailing: None,
            destructive: false,
            disabled: false,
            accessibility_value: None,
            accessibility_selected: false,
            selected: false,
            guide_anchor: None,
            show_chevron: true,
            trailing_slot: None,
            show_chevron_when_trailing_slot: false,
            trailing_keeps_width: false,
            content_slot: None,
            expanded_slot: None,
            callback: Rc::new(|_, _| {}),
        }
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn icon_background(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.icon_background = color.into();
        self
    }

    pub fn icon_foreground(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.icon_foreground = color.into();
        self
    }

    pub fn leading_slot(mut self, slot: impl IntoElement) -> Self {
        self.leading_slot = Some(slot.into_any_element());
        self
    }

    pub fn pretitle(mut self, pretitle: impl Into<SharedString>) -> Self {
        self.pretitle = Some(pretitle.into());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn text_line_limit(mut self, limit: usize) -> Self {
        self.text_line_limit = limit;
        self
    }

    pub fn subtitle_line_limit(mut self, limit: usize) -> Self {
        self.subtitle_line_limit = limit;
        self
    }

    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn accessibility_value(mut self, value: impl Into<SharedString>) -> Self {
        self.accessibility_value = Some(value.into());
        self
    }

    pub fn accessibility_selected(mut self, selected: bool) -> Self {
        self.accessibility_selected = selected;
        self
    }

    /// marks the row as the one open beside the list, also reported selected to accessibility
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self.accessibility_selected = self.accessibility_selected || selected;
        self
    }

    pub fn guide_anchor(mut self, anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = anchor.into();
        self
    }

    pub fn show_chevron(mut self, show: bool) -> Self {
        self.show_chevron = show;
        self
    }

    pub fn trailing_slot(mut self, slot: impl IntoElement) -> Self {
        self.trailing_slot = Some(slot.into_any_element());
        self
    }

    /// keeps the trailing slot whole (a value that is the row state) and lets the title truncate
    pub fn trailing_keeps_width(mut self, keeps: bool) -> Self {
        self.trailing_keeps_width = keeps;
        self
    }

    pub fn show_chevron_when_trailing_slot(mut self, show: bool) -> Self {
        self.show_chevron_when_trailing_slot = show;
        self
    }

    pub fn content_slot(mut self, slot: impl IntoElement) -> Self {
        self.content_slot = Some(slot.into_any_element());
        self
    }

    pub fn expanded_slot(mut self, slot: impl IntoElement) -> Self {
        self.expanded_slot = Some(slot.into_any_element());
        self
    }

    pub fn callback(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.callback = Rc::new(callback);
        self
    }

    /// a plain one line title beside a trailing value, the title is never squeezed and the value gives way
    /// rows with pretitle or subtitle keep the shared split
    fn title_leads_trailing(&self) -> bool {
        !self.trailing_keeps_width
            && (self.trailing_slot.is_some() || self.trailing.is_some())
            && self.pretitle.is_none()
            && self.subtitle.is_none()
    }

    pub fn should_show_chevron(&self) -> bool {
        if self.trailing_slot.is_some() {
            return self.show_chevron_when_trailing_slot;
        }
        self.show_chevron
    }
}

/// the red action closing a card, under its own separator
pub struct GroupedEndAction {
    pub title: SharedString,
    pub disabled: bool,
    pub callback: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl GroupedEndAction {
    pub fn new(
        title: impl Into<SharedString>,
        callback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            disabled: false,
            callback: Rc::new(callback),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// a settings card with optional title row, rows with separators
/// optional end action and optional caption under it
#[derive(IntoElement)]
pub struct GroupedBtns {
    id: ElementId,
    items: Vec<GroupedBtn>,
    title: Option<SharedString>,
    right_top_text: Option<SharedString>,
    end_group_title: Option<SharedString>,
    end_group_title_px: f32,
    end_action: Option<GroupedEndAction>,
    corner_radius: f32,
    horizontal_padding: f32,
    separator_leading_padding: f32,
    surface_color: Option<Hsla>,
    primary_text_color: Option<Hsla>,
    secondary_text_color: Option<Hsla>,
    separator_color: Option<Hsla>,
    uses_lazy_rows: bool,
}

impl GroupedBtns {
    pub fn new(id: impl Into<ElementId>, items: Vec<GroupedBtn>) -> Self {
        Self {
            id: id.into(),
            items,
            title: None,
            right_top_text: None,
            end_group_title: None,
            end_group_title_px: 12.0,
            end_action: None,
            corner_radius: GroupedCardMetrics::CORNER_RADIUS,
            horizontal_padding: GroupedCardMetrics::HORIZONTAL_PADDING,
            separator_leading_padding: GroupedCardMetrics::content_leading_inset(),
            surface_color: None,
            primary_text_color: None,
            secondary_text_color: None,
            separator_color: None,
            uses_lazy_rows: true,
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn right_top_text(mut self, text: impl Into<SharedString>) -> Self {
        self.right_top_text = Some(text.into());
        self
    }

    pub fn end_group_title(mut self, title: impl Into<SharedString>) -> Self {
        self.end_group_title = Some(title.into());
        self
    }

    pub fn end_group_title_px(mut self, px: f32) -> Self {
        self.end_group_title_px = px;
        self
    }

    pub fn end_action(mut self, action: GroupedEndAction) -> Self {
        self.end_action = Some(action);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn horizontal_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = padding;
        self
    }

    pub fn separator_leading_padding(mut self, padding: f32) -> Self {
        self.separator_leading_padding = padding;
        self
    }

    pub fn surface_color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.surface_color = color.into();
        self
    }

    pub fn primary_text_color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.primary_text_color = color.into();
        self
    }

    pub fn secondary_text_color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.secondary_text_color = color.into();
        self
    }

    pub fn separator_color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.separator_color = color.into();
        self
    }

    pub fn uses_lazy_rows(mut self, lazy: bool) -> Self {
        self.uses_lazy_rows = lazy;
        self
    }

    pub fn rows_are_lazy(&self) -> bool {
        self.uses_lazy_rows
    }
}

fn disabled_opacity(disabled: bool) -> f32 {
    if disabled { DISABLED_OPACITY } else { 1.0 }
}

fn leading_content(item: &mut GroupedBtn, theme: &AppTheme) -> AnyElement {
    if let Some(leading_slot) = item.leading_slot.take() {
        return leading_slot;
    }
    if let Some(name) = item.icon.as_ref() {
        let bg = item.icon_background.unwrap_or(if item.destructive {
            theme.semantic.error_100.opacity(0.45)
        } else {
            theme.primary._100.opacity(0.88)
        });
        let fg = item.icon_foreground.unwrap_or(gpui::white());
        return div()
            .size(px(GroupedCardMetrics::ICON_SIDE))
            .flex_none()
            .bg(bg)
            .continuous_corners(11.0)
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(name, px(17.0), fg, false))
            .into_any_element();
    }
    div()
        .w(px(0.0))
        .h(px(GroupedCardMetrics::ICON_SIDE))
        .into_any_element()
}

impl RenderOnce for GroupedBtns {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().clone()
        };
        let surface = self.surface_color.unwrap_or(theme.bg._200);
        let primary_text = self.primary_text_color.unwrap_or(theme.text.primary);
        let secondary_text = self.secondary_text_color.unwrap_or(theme.text.secondary);
        let separator = self.separator_color.unwrap_or(theme.border._200.opacity(0.55));
        let error = theme.semantic.error_100;
        let selection_fill = theme.primary._100.opacity(0.16);

        let header = (self.title.is_some() || self.right_top_text.is_some()).then(|| {
            hstack(None)
                .w_full()
                .children(
                    self.title
                        .clone()
                        .map(|title| MainText::new(title).px(12.0).color(secondary_text)),
                )
                .child(spacer())
                .children(
                    self.right_top_text
                        .clone()
                        .map(|text| MainText::new(text).px(12.0).color(secondary_text)),
                )
                .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)
        });

        let count = self.items.len();
        let has_end_action = self.end_action.is_some();
        let selection_radius = px(self.corner_radius);
        let mut rows = vstack(0.0).w_full();
        for (i, mut item) in self.items.into_iter().enumerate() {
            let opacity = disabled_opacity(item.disabled);
            let anchor = item.guide_anchor;
            if let Some(content) = item.content_slot.take() {
                rows = rows.child(
                    div()
                        .w_full()
                        .opacity(opacity)
                        .child(content)
                        .frame_max_width(Alignment::Leading)
                        .guide_anchor(anchor),
                );
            } else {
                let leading = leading_content(&mut item, &theme);
                let title_leads_trailing = item.title_leads_trailing();
                let trailing_is_slot = item.trailing_slot.is_some();
                let mut text_column = vstack(2.0).align(HorizontalAlignment::Leading).min_w(px(0.0));
                text_column = match (title_leads_trailing, trailing_is_slot) {
                    (true, false) => text_column.flex_none(),
                    (true, true) => text_column.flex_shrink(1.0),
                    (false, _) => text_column.flex_1(),
                };
                if let Some(pre) = item.pretitle.clone() {
                    text_column = text_column.child(MainText::new(pre).px(11.0).color(secondary_text));
                }
                text_column = text_column.child(
                    MainText::new(item.text.clone())
                        .px(15.0)
                        .color(if item.destructive { error } else { primary_text })
                        .number_of_lines(item.text_line_limit),
                );
                if let Some(subtitle) = item.subtitle.clone() {
                    text_column = text_column.child(
                        MainText::new(subtitle)
                            .px(12.0)
                            .color(secondary_text)
                            .number_of_lines(item.subtitle_line_limit),
                    );
                }
                let show_chevron = item.should_show_chevron();
                let trailing: Option<AnyElement> = match item.trailing_slot.take() {
                    Some(slot) => Some(slot),
                    None => item
                        .trailing
                        .clone()
                        .map(|text| {
                            let value = MainText::new(text).px(14.0).color(secondary_text);
                            if item.trailing_keeps_width {
                                div().whitespace_nowrap().child(value).into_any_element()
                            } else {
                                value.number_of_lines(1).into_any_element()
                            }
                        }),
                };
                let trailing = trailing.map(|trailing| {
                    if item.trailing_keeps_width || trailing_is_slot {
                        div().flex_none().child(trailing).into_any_element()
                    } else if title_leads_trailing {
                        div().min_w(px(0.0)).flex_shrink(1.0).child(trailing).into_any_element()
                    } else {
                        trailing
                    }
                });
                let accessibility_value = [
                    item.accessibility_selected.then(|| SharedString::from(tr("accessibility_state_selected"))),
                    item.accessibility_value.clone(),
                ]
                .into_iter()
                .flatten()
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<String>>()
                .join(", ");
                let chevron = show_chevron
                    .then(|| IconUi::sized(IconUi::trailing_chevron(), px(12.0), secondary_text.opacity(0.75), false))
                    .flatten()
                    .map(|chevron| div().flex_none().child(chevron));
                let callback = item.callback.clone();
                let mut row_value = div().id(SharedString::from(format!("{}-row-{}-value", self.id, item.id))).w_full();
                if item.selected {
                    row_value = row_value.bg(selection_fill);
                    if i == 0 {
                        row_value = row_value.rounded_tl(selection_radius).rounded_tr(selection_radius);
                    }
                    if i + 1 == count && !has_end_action && item.expanded_slot.is_none() {
                        row_value = row_value.rounded_bl(selection_radius).rounded_br(selection_radius);
                    }
                }
                rows = rows.child(
                    plain(SharedString::from(format!("{}-row-{}", self.id, item.id)))
                        .on_press(move |window, cx| callback(window, cx))
                        .disabled(item.disabled)
                        .w_full()
                        .opacity(opacity)
                        .child(
                            row_value
                                .aria_value(accessibility_value)
                                .child(
                                    hstack(GroupedCardMetrics::ICON_SPACING)
                                        .w_full()
                                        .child(leading)
                                        .child(text_column)
                                        .children(title_leads_trailing.then(|| spacer_min_w(0.0)))
                                        .children(trailing)
                                        .children(chevron)
                                        .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::ROW_HORIZONTAL_PADDING)
                                        .padding_edges(EdgeSet::VERTICAL, GroupedCardMetrics::ROW_VERTICAL_PADDING),
                                ),
                        )
                        .guide_anchor(anchor),
                );
            }

            if let Some(expanded) = item.expanded_slot.take() {
                rows = rows.child(
                    div()
                        .w_full()
                        .opacity(opacity)
                        .child(expanded.frame_max_width(Alignment::Center)),
                );
            }

            if i + 1 < count {
                rows = rows.child(
                    div()
                        .w_full()
                        .padding_edges(EdgeSet::LEADING, self.separator_leading_padding)
                        .child(div().w_full().h(px(0.5)).bg(separator)),
                );
            }
        }

        if let Some(end) = self.end_action {
            let callback = end.callback.clone();
            rows = rows
                .child(div().w_full().h(px(0.5)).bg(separator.opacity(0.82)))
                .child(
                    plain(SharedString::from(format!("{}-end-action", self.id)))
                        .on_press(move |window, cx| callback(window, cx))
                        .disabled(end.disabled)
                        .w_full()
                        .opacity(disabled_opacity(end.disabled))
                        .child(
                            hstack(10.0)
                                .w_full()
                                .children(IconUi::sized(END_ACTION_SYMBOL, px(20.0), error, false))
                                .child(
                                    MainText::new(end.title)
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(error),
                                )
                                .child(spacer_min_w(0.0))
                                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                                .padding_edges(EdgeSet::VERTICAL, 13.0),
                        ),
                );
        }

        let card = div()
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, self.horizontal_padding)
            .child(
                rows.bg(surface)
                    .clip_shape(Shape::continuous(self.corner_radius)),
            );

        let end_title = self.end_group_title.map(|title| {
            MainText::new(title)
                .px(self.end_group_title_px)
                .tac(TextAlignT::Left)
                .color(secondary_text)
                .min_w_0()
                .frame_max_width(Alignment::Leading)
                .padding_edges(EdgeSet::HORIZONTAL, GroupedCardMetrics::TITLE_HORIZONTAL_PADDING)
                .padding_edges(EdgeSet::TOP, 3.0)
        });

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .children(header)
            .child(card)
            .children(end_title)
    }
}
