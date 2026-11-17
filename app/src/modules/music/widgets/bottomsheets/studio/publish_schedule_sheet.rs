use std::rc::Rc;

use gpui::{App, ElementId, Entity, FontWeight, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, fill_content_color};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::date_picker::DatePicker;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer, vstack};
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};

const SECONDS_PER_WEEK: f64 = 7.0 * 24.0 * 3600.0;
const MINIMUM_HOURS_AHEAD: i64 = 1;
const MAXIMUM_MONTHS_AHEAD: i64 = 6;
const DATE_PICKER_TOGGLE_DURATION: f64 = 0.2;
const DATE_PICKER_TRANSITION_SCALE: f32 = 0.96;
const BODY_SYMBOL_SIDE: f32 = 17.0;

/// snap heights of the publish sheet
/// opens at rest height, grows to the picker height while the calendar is open
/// so time field and schedule stay reachable
pub enum PublishScheduleSheetLayout {}

impl PublishScheduleSheetLayout {
    pub const RESTING_FRACTION: f32 = 0.50;
    pub const PICKER_FRACTION: f32 = 0.93;
    pub const SNAP_FRACTIONS: [f32; 2] = [Self::RESTING_FRACTION, Self::PICKER_FRACTION];
}

struct PublishScheduleSheetState {
    selected_date: Date,
    show_date_picker: bool,
    date_picker_presence: Presence<()>,
    min_date: Date,
    max_date: Date,
}

impl PublishScheduleSheetState {
    fn new() -> Self {
        let now = RiseClock::now();
        let calendar = Calendar::current();
        Self {
            selected_date: now.adding_time_interval(SECONDS_PER_WEEK),
            show_date_picker: false,
            date_picker_presence: Presence::new(None).with_transition(
                Transition::opacity().combined(Transition::scale_anchored(DATE_PICKER_TRANSITION_SCALE, UnitPoint::TOP)),
            ),
            min_date: calendar.date_by_adding(CalendarComponent::Hour, MINIMUM_HOURS_AHEAD, &now).unwrap_or(now),
            max_date: calendar.date_by_adding(CalendarComponent::Month, MAXIMUM_MONTHS_AHEAD, &now).unwrap_or(now),
        }
    }
}

/// a symbol in the ink the button puts on its content
#[derive(IntoElement)]
struct ButtonSymbol {
    name: &'static str,
}

impl RenderOnce for ButtonSymbol {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = fill_content_color(cx).unwrap_or_else(|| ThemeStore::shared(cx).read(cx).current_theme().text.primary);
        div().children(IconUi::sized(self.name, px(BODY_SYMBOL_SIDE), color, self.name.ends_with(".fill")))
    }
}

#[derive(IntoElement)]
pub struct PublishScheduleSheet {
    id: ElementId,
    on_publish_now: Rc<dyn Fn(&mut Window, &mut App)>,
    on_schedule: Rc<dyn Fn(Date, &mut Window, &mut App)>,
    on_dismiss: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    sheet_fraction: Option<Binding<f32>>,
}

impl PublishScheduleSheet {
    pub fn new(
        id: impl Into<ElementId>,
        on_publish_now: impl Fn(&mut Window, &mut App) + 'static,
        on_schedule: impl Fn(Date, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            on_publish_now: Rc::new(on_publish_now),
            on_schedule: Rc::new(on_schedule),
            on_dismiss: None,
            sheet_fraction: None,
        }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }

    pub fn sheet_fraction(mut self, sheet_fraction: Binding<f32>) -> Self {
        self.sheet_fraction = Some(sheet_fraction);
        self
    }

    fn sync_sheet_fraction(show_date_picker: bool, sheet_fraction: Option<&Binding<f32>>, window: &mut Window, cx: &mut App) {
        let target = if show_date_picker {
            PublishScheduleSheetLayout::PICKER_FRACTION
        } else {
            PublishScheduleSheetLayout::RESTING_FRACTION
        };
        let Some(sheet_fraction) = sheet_fraction else {
            return;
        };
        if sheet_fraction.get(cx) == target {
            return;
        }
        sheet_fraction.set(target, window, cx);
    }

    fn format_date(date: &Date) -> String {
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_format("d MMM yyyy, HH:mm");
        formatter.string_from(date)
    }

    fn toggle_date_picker(state: &Entity<PublishScheduleSheetState>, cx: &mut App) -> bool {
        state.update(cx, |state, cx| {
            state.show_date_picker = !state.show_date_picker;
            let shown = state.show_date_picker.then_some(());
            state.date_picker_presence.set_in(shown, ease_in_out(DATE_PICKER_TOGGLE_DURATION), RiseClock::monotonic(), cx);
            cx.notify();
            state.show_date_picker
        })
    }
}

impl RenderOnce for PublishScheduleSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| PublishScheduleSheetState::new());
        let sheet = state.read(cx);
        let selected_date = sheet.selected_date;
        let show_date_picker = sheet.show_date_picker;
        let now = RiseClock::monotonic();
        let date_picker = sheet.date_picker_presence.render_transitioned(now, window, |_| {
            let selection = Binding::entity(
                state.clone(),
                |state: &PublishScheduleSheetState| state.selected_date,
                |state: &mut PublishScheduleSheetState, value, _| state.selected_date = value,
            );
            div().w_full().child(
                DatePicker::new((self.id.clone(), "date-picker"), selection)
                    .range(sheet.min_date..=sheet.max_date)
                    .tint(theme.primary._100),
            )
        });

        let publish_dismiss = self.on_dismiss.clone();
        let on_publish_now = Rc::clone(&self.on_publish_now);
        let schedule_dismiss = self.on_dismiss.clone();
        let on_schedule = Rc::clone(&self.on_schedule);
        let toggle_state = state.clone();
        let toggle_fraction = self.sheet_fraction.clone();
        let appear_fraction = self.sheet_fraction;

        let body = vstack(20.0)
            .w_full()
            .justify_start()
            .px(px(20.0))
            .pt(px(8.0))
            .pb(px(32.0))
            .bg(theme.bg._100)
            .child(MainText::new(tr("music_publish")).px(18.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
            .child(
                div().w_full().child(
                    ButtonUi::new((self.id.clone(), "publish-now"))
                        .on_press(move |window, cx| {
                            if let Some(on_dismiss) = publish_dismiss.as_ref() {
                                on_dismiss(window, cx);
                            }
                            on_publish_now(window, cx);
                        })
                        .child(
                            hstack(8.0)
                                .child(ButtonSymbol { name: "bolt.fill" })
                                .child(MainText::new(tr("music_publish_now")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
                        ),
                ),
            )
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .child(div().flex_1().h(px(0.5)).bg(theme.border._100))
                    .child(div().px(px(12.0)).child(SecondaryText::new(tr("music_publish_or")).px(13.0)))
                    .child(div().flex_1().h(px(0.5)).bg(theme.border._100)),
            )
            .child(
                vstack(12.0)
                    .w_full()
                    .child(
                        hstack(12.0)
                            .w_full()
                            .p(px(14.0))
                            .bg(theme.bg._200)
                            .rounded_rect(12.0)
                            .children(IconUi::sized("calendar.badge.clock", px(18.0), theme.primary._100, false))
                            .child(
                                vstack(2.0)
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        MainText::new(tr("music_publish_schedule"))
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.text.primary),
                                    )
                                    .child(SecondaryText::new(Self::format_date(&selected_date)).px(12.0).color(theme.primary._100)),
                            )
                            .child(spacer())
                            .child(
                                plain((self.id.clone(), "toggle-date-picker"))
                                    .on_press(move |window, cx| {
                                        let shown = Self::toggle_date_picker(&toggle_state, cx);
                                        Self::sync_sheet_fraction(shown, toggle_fraction.as_ref(), window, cx);
                                    })
                                    .children(IconUi::sized(
                                        if show_date_picker { "chevron.up" } else { "chevron.down" },
                                        px(13.0),
                                        theme.text.secondary,
                                        false,
                                    )),
                            ),
                    )
                    .children(date_picker)
                    .child(
                        div().w_full().child(
                            ButtonUi::new((self.id.clone(), "schedule"))
                                .disabled(selected_date <= RiseClock::now())
                                .on_press(move |window, cx| {
                                    if let Some(on_dismiss) = schedule_dismiss.as_ref() {
                                        on_dismiss(window, cx);
                                    }
                                    on_schedule(selected_date, window, cx);
                                })
                                .child(
                                    hstack(8.0)
                                        .child(ButtonSymbol { name: "calendar" })
                                        .child(MainText::new(tr("music_publish_schedule")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
                                ),
                        ),
                    ),
            );
        let scroll_id = (self.id.clone(), "scroll");
        div()
            .size_full()
            .bg(theme.bg._100)
            .child(scroll_view(scroll_id).child(body))
            .on_appear((self.id, "appear"), move |window, cx| {
                Self::sync_sheet_fraction(show_date_picker, appear_fraction.as_ref(), window, cx);
            })
    }
}
