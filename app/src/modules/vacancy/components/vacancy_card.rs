use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Window, rgb};
use rise_core::foundation::foundation_text::characters;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::modules::post::components::moderation_hidden_banner::ModerationHiddenBanner;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::VacancyModel;

#[derive(IntoElement)]
pub struct VacancyCard {
    id: ElementId,
    vacancy: VacancyModel,
    is_viewed: bool,
    is_applied: bool,
    on_tap: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl VacancyCard {
    pub fn new(id: impl Into<ElementId>, vacancy: VacancyModel) -> Self {
        Self { id: id.into(), vacancy, is_viewed: false, is_applied: false, on_tap: Rc::new(|_, _| {}) }
    }

    pub fn is_viewed(mut self, is_viewed: bool) -> Self {
        self.is_viewed = is_viewed;
        self
    }

    pub fn is_applied(mut self, is_applied: bool) -> Self {
        self.is_applied = is_applied;
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Rc::new(on_tap);
        self
    }

    fn company_logo(&self, theme: &AppTheme) -> Div {
        let initial = SharedString::from(characters(&self.vacancy.company).next().unwrap_or("").to_owned());
        let (fill, ink) = if self.is_viewed {
            (theme.bg._300, theme.text.secondary)
        } else {
            (theme.primary._100.opacity(0.15), theme.primary._100)
        };
        div()
            .flex_none()
            .size(px(48.0))
            .bg(fill)
            .continuous_corners(12.0)
            .flex()
            .items_center()
            .justify_center()
            .child(div().swiftui_font(Font::system_weight(20.0, FontWeight::BOLD)).text_color(ink).child(initial))
    }

    fn applied_badge(&self) -> Div {
        let green: Hsla = rgb(0x00C781).into();
        hstack(3.0)
            .padding_edges(EdgeSet::HORIZONTAL, 6.0)
            .padding_edges(EdgeSet::VERTICAL, 3.0)
            .bg(green.opacity(0.12))
            .capsule()
            .children(IconUi::sized("checkmark.circle.fill", px(9.0), green, true))
            .child(
                div()
                    .swiftui_font(Font::system_weight(10.0, FontWeight::MEDIUM))
                    .text_color(green)
                    .child(SharedString::from(tr("vacancy_applied_badge"))),
            )
    }

    fn viewed_badge(&self, theme: &AppTheme) -> Div {
        Self::capsule_label(tr("vacancy_viewed_badge"), 10.0, 6.0, 3.0, theme.text.secondary, theme.bg._300)
    }

    fn tag_chip(&self, text: String, theme: &AppTheme) -> Div {
        Self::capsule_label(text, 11.0, 8.0, 4.0, theme.text.secondary, theme.bg._300)
    }

    fn skill_chip(&self, text: String, theme: &AppTheme) -> Div {
        let ink = if self.is_viewed { theme.text.secondary } else { theme.primary._100 };
        Self::capsule_label(text, 11.0, 8.0, 4.0, ink, ink.opacity(0.12))
    }

    fn capsule_label(text: String, size: f32, horizontal: f32, vertical: f32, ink: Hsla, fill: Hsla) -> Div {
        div()
            .flex_none()
            .padding_edges(EdgeSet::HORIZONTAL, horizontal)
            .padding_edges(EdgeSet::VERTICAL, vertical)
            .bg(fill)
            .capsule()
            .swiftui_font(Font::system_weight(size, FontWeight::MEDIUM))
            .text_color(ink)
            .whitespace_nowrap()
            .child(SharedString::from(text))
    }
}

impl RenderOnce for VacancyCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let vacancy = &self.vacancy;
        let is_viewed = self.is_viewed;

        let mut trailing = vstack(4.0)
            .items_end()
            .flex_none()
            .child(MainText::new(vacancy.published_at()).px(11.0).color(theme.text.secondary));
        if self.is_applied {
            trailing = trailing.child(self.applied_badge());
        } else if is_viewed {
            trailing = trailing.child(self.viewed_badge(&theme));
        }

        let head = hstack(12.0)
            .w_full()
            .child(self.company_logo(&theme))
            .child(
                vstack(2.0)
                    .items_start()
                    .min_w_0()
                    .child(MainText::new(vacancy.company.clone()).px(13.0).color(theme.text.secondary).line_limit(1))
                    .child(MainText::new(vacancy.city.clone()).px(12.0).color(theme.text.secondary).line_limit(1)),
            )
            .child(spacer_min_w(0.0))
            .child(trailing);

        let title_color = if is_viewed { theme.text.secondary } else { theme.text.primary };
        let salary_color = if is_viewed { theme.text.secondary } else { theme.primary._100 };

        let tags = hstack(6.0)
            .child(self.tag_chip(vacancy.schedule_display(), &theme))
            .child(self.tag_chip(vacancy.job_format_display(), &theme))
            .child(self.tag_chip(vacancy.experience_display(), &theme));

        let skills = (!vacancy.skills.is_empty()).then(|| {
            let scroll_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("skills"));
            div()
                .id(scroll_id)
                .w_full()
                .flex()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .child(
                    hstack(6.0)
                        .flex_none()
                        .children(vacancy.skills.iter().take(5).map(|skill| self.skill_chip(skill.clone(), &theme))),
                )
        });

        let banner = vacancy
            .is_moderation_disabled()
            .then(|| ModerationHiddenBanner::new(tr("vacancy_disabled_by_moderation")));

        let body = vstack(12.0)
            .w_full()
            .items_start()
            .p(px(16.0))
            .bg(if is_viewed { theme.bg._200.opacity(0.6) } else { theme.bg._200 })
            .rounded(theme.radius._300)
            .stroke_border(theme.border._100, 1.0)
            .opacity(if is_viewed { 0.75 } else { 1.0 })
            .child(head)
            .children(banner)
            .child(
                MainText::new(vacancy.title.clone())
                    .px(16.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(title_color)
                    .line_limit(2),
            )
            .child(
                MainText::new(vacancy.formatted_salary())
                    .px(15.0)
                    .font_weight(FontWeight::BOLD)
                    .color(salary_color),
            )
            .child(tags)
            .children(skills);

        let on_tap = self.on_tap.clone();
        plain(self.id.clone()).w_full().on_press(move |window, cx| on_tap(window, cx)).child(body)
    }
}
