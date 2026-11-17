use gpui::{App, ElementId, FontWeight, Hsla, Window};
use rise_i18n::tr;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::prelude::*;

use crate::modules::moderation::components::moderation_kit_ui::{
    ModerationMetrics, moderation_child_id, moderation_symbol,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModeratorCapability, ModeratorCapabilityExt, ModeratorPermissions, ModeratorScopeType, ReportTargetTypeExt,
};

const SEPARATOR_LEADING_PADDING: f32 = 16.0;
const STEPPER_SPACING: f32 = 10.0;
const STEPPER_BUTTON_SIDE: f32 = 30.0;
const STEPPER_ICON_SIDE: f32 = 12.0;
const STEPPER_VALUE_MIN_WIDTH: f32 = 28.0;
const UNCAPPED_OFF_DAYS: i64 = 7;

/// no accordions and no per switch icon, thirty toggles in thirty colors made grants unreadable
/// rows are not lazy since every change re-estimated the unmounted rows above the stepper
/// and the scroll offset jumped under the next tap
#[derive(IntoElement)]
pub struct ModeratorPermissionsEditor {
    id: ElementId,
    permissions: Binding<ModeratorPermissions>,
    show_appeals_toggle: bool,
}

impl ModeratorPermissionsEditor {
    pub fn new(id: impl Into<ElementId>, permissions: Binding<ModeratorPermissions>) -> Self {
        Self { id: id.into(), permissions, show_appeals_toggle: false }
    }

    pub fn show_appeals_toggle(mut self, show_appeals_toggle: bool) -> Self {
        self.show_appeals_toggle = show_appeals_toggle;
        self
    }

    fn scope_card(&self, scope: ModeratorScopeType, current: &ModeratorPermissions) -> GroupedBtns {
        let capabilities = ModeratorCapability::ALL;
        let granted = capabilities.iter().filter(|cap| current.scope(scope).has(**cap)).count();
        let items = capabilities
            .iter()
            .map(|cap| {
                self.toggle_item(
                    format!("{}-{}", scope.id(), cap.id()),
                    tr(cap.title_key()),
                    self.cap_binding(scope, *cap),
                )
            })
            .collect();
        GroupedBtns::new(moderation_child_id(&self.id, format!("scope-{}", scope.id())), items)
            .title(tr(scope.title_key()).to_uppercase())
            .right_top_text(format!("{}/{}", granted, capabilities.len()))
            .separator_leading_padding(SEPARATOR_LEADING_PADDING)
            .uses_lazy_rows(false)
    }

    fn cap_binding(&self, scope: ModeratorScopeType, cap: ModeratorCapability) -> Binding<bool> {
        let reader = self.permissions.clone();
        let writer = self.permissions.clone();
        Binding::new(
            move |cx| reader.get(cx).scope(scope).has(cap),
            move |value, window, cx| {
                let mut permissions = writer.get(cx);
                let mut scope_permissions = permissions.scope(scope);
                scope_permissions.set(cap, value);
                permissions.scopes.insert(scope.wire().to_owned(), scope_permissions);
                writer.set(permissions, window, cx);
            },
        )
    }

    fn global_card(&self, current: &ModeratorPermissions, cx: &App) -> GroupedBtns {
        let mut items: Vec<GroupedBtn> = Vec::new();

        if self.show_appeals_toggle {
            items.push(self.toggle_item("appeals", tr("moderation_can_resolve_appeals"), self.appeals_binding()));
        }

        items.push(self.toggle_item("uncapped", tr("moderation_uncap_blocks"), self.uncapped_binding()));

        if current.max_block_days.is_some() {
            items.push(
                GroupedBtn::new("maxDays", tr("moderation_max_block_days"))
                    .show_chevron(false)
                    .content_slot(self.max_days_row(current, cx)),
            );
        }

        GroupedBtns::new(moderation_child_id(&self.id, "global"), items)
            .title(tr("moderation_global_settings_label").to_uppercase())
            .separator_leading_padding(SEPARATOR_LEADING_PADDING)
            .uses_lazy_rows(false)
    }

    fn appeals_binding(&self) -> Binding<bool> {
        let reader = self.permissions.clone();
        let writer = self.permissions.clone();
        Binding::new(
            move |cx| reader.get(cx).can_resolve_appeals,
            move |value, window, cx| {
                let mut permissions = writer.get(cx);
                permissions.can_resolve_appeals = value;
                writer.set(permissions, window, cx);
            },
        )
    }

    fn max_days_row(&self, current: &ModeratorPermissions, cx: &App) -> impl IntoElement {
        let primary = rise_ui::theme(cx).text.primary;
        hstack(GroupedCardMetrics::ICON_SPACING)
            .w_full()
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(MainText::new(tr("moderation_max_block_days")).px(15.0).color(primary)),
            )
            .child(self.stepper(current, cx))
    }

    fn stepper(&self, current: &ModeratorPermissions, cx: &App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let (decrease_tint, increase_tint) = (theme.text.secondary, theme.primary._100);
        let surface = theme.bg._100;
        let decrease = self.permissions.clone();
        let increase = self.permissions.clone();
        hstack(STEPPER_SPACING)
            .child(self.stepper_button("minus", decrease_tint, surface, move |window, cx| {
                let mut permissions = decrease.get(cx);
                permissions.max_block_days = Some((permissions.max_block_days.unwrap_or(1) - 1).max(1));
                decrease.set(permissions, window, cx);
            }))
            .child(
                div().flex().justify_center().min_w(px(STEPPER_VALUE_MIN_WIDTH)).child(
                    MainText::new(current.max_block_days.unwrap_or(1).to_string())
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .monospaced_digits(true),
                ),
            )
            .child(self.stepper_button("plus", increase_tint, surface, move |window, cx| {
                let mut permissions = increase.get(cx);
                permissions.max_block_days = Some(permissions.max_block_days.unwrap_or(1) + 1);
                increase.set(permissions, window, cx);
            }))
    }

    fn stepper_button(
        &self,
        icon: &'static str,
        tint: Hsla,
        surface: Hsla,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        plain(moderation_child_id(&self.id, icon))
            .on_press(action)
            .size(px(STEPPER_BUTTON_SIDE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(surface)
            .children(moderation_symbol(icon, STEPPER_ICON_SIDE, tint))
    }

    fn uncapped_binding(&self) -> Binding<bool> {
        let reader = self.permissions.clone();
        let writer = self.permissions.clone();
        Binding::new(
            move |cx| reader.get(cx).max_block_days.is_none(),
            move |is_uncapped, window, cx| {
                let mut permissions = writer.get(cx);
                permissions.max_block_days = if is_uncapped { None } else { Some(UNCAPPED_OFF_DAYS) };
                writer.set(permissions, window, cx);
            },
        )
    }

    fn toggle_item(&self, id: impl Into<gpui::SharedString>, label: String, is_on: Binding<bool>) -> GroupedBtn {
        let id: gpui::SharedString = id.into();
        let toggled = is_on.clone();
        GroupedBtn::new(id.clone(), label)
            .show_chevron(false)
            .trailing_slot(SwitchUi::new(moderation_child_id(&self.id, format!("switch-{id}")), is_on))
            .callback(move |window, cx| toggled.toggle(window, cx))
    }
}

impl RenderOnce for ModeratorPermissionsEditor {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let current = self.permissions.get(cx);
        vstack(ModerationMetrics::SECTION_SPACING)
            .w_full()
            .children(
                ModeratorScopeType::moderator_scopes()
                    .iter()
                    .map(|scope| self.scope_card(*scope, &current))
                    .collect::<Vec<_>>(),
            )
            .child(self.global_card(&current, cx))
    }
}
