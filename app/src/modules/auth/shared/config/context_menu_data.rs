use std::rc::Rc;

use gpui::{App, Window};
use rise_i18n::tr;

use crate::core::ui::context_menu_ui::ContextMenuItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenderT {
    Male,
    Female,
    None,
}

impl GenderT {
    const ALL: [GenderT; 3] = [GenderT::Male, GenderT::Female, GenderT::None];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Male => "Male",
            Self::Female => "Female",
            Self::None => "None",
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|gender| gender.raw_value() == raw_value)
    }
}

pub fn get_gender_context_menu_items(
    set_selected_gender: impl Fn(GenderT, &mut Window, &mut App) + 'static,
) -> Vec<ContextMenuItem> {
    let set_selected_gender = Rc::new(set_selected_gender);
    let male = Rc::clone(&set_selected_gender);
    let female = Rc::clone(&set_selected_gender);
    let none = set_selected_gender;
    vec![
        ContextMenuItem::new(
            1,
            tr("contextMenu_male"),
            Some("male".to_owned()),
            GenderT::Male.raw_value(),
            move |window, cx| male(GenderT::Male, window, cx),
        ),
        ContextMenuItem::new(
            2,
            tr("contextMenu_female"),
            Some("female".to_owned()),
            GenderT::Female.raw_value(),
            move |window, cx| female(GenderT::Female, window, cx),
        ),
        ContextMenuItem::new(
            3,
            tr("not_selected"),
            None,
            GenderT::None.raw_value(),
            move |window, cx| none(GenderT::None, window, cx),
        ),
    ]
}
