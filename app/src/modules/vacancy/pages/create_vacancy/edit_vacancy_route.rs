use gpui::{App, Subscription, Window};
use rise_ui::swiftui::prelude::*;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::vacancy::pages::create_vacancy::create_vacancy::CreateVacancy;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_actions::VacancyActionsStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::VacancyModel;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::WireVacancyDTOExt;
use crate::modules::vacancy::stores::vacancy_interactions::vacancy_interactions::VacancyInteractionsStore;

struct EditVacancyRouteObservation {
    _subscription: Subscription,
}

#[derive(IntoElement)]
pub struct EditVacancyRoute {
    vacancy_id: i64,
}

impl EditVacancyRoute {
    pub fn new(vacancy_id: i64) -> Self {
        Self { vacancy_id }
    }

    fn prefill(vacancy_id: i64, cx: &mut App) -> Option<VacancyModel> {
        if let Some(practice) =
            GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_vacancy(vacancy_id))
        {
            return Some(practice);
        }
        let store = VacancyActionsStore::shared(cx);
        let store = store.read(cx);
        store
            .my_vacancies_sai
            .data()
            .and_then(|data| data.list.iter().find(|vacancy| vacancy.id == vacancy_id).map(WireVacancyDTOExt::to_model))
            .or_else(|| {
                store
                    .get_vacancy_sai
                    .data()
                    .and_then(|data| data.vacancy)
                    .filter(|vacancy| vacancy.id == vacancy_id)
                    .map(|vacancy| vacancy.to_model())
            })
    }
}

impl RenderOnce for EditVacancyRoute {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state("edit-vacancy-route-observation", cx, |_, cx| {
            let store = VacancyActionsStore::shared(cx);
            EditVacancyRouteObservation { _subscription: cx.observe(&store, |_, _, cx| cx.notify()) }
        });
        let vacancy_id = self.vacancy_id;
        match Self::prefill(vacancy_id, cx) {
            Some(prefill) => CreateVacancy::new(Some(prefill)).into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(progress_view())
                .on_appear("edit-vacancy-route-appear", move |_, cx| {
                    VacancyInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_vacancy_loaded(vacancy_id, cx));
                })
                .into_any_element(),
        }
    }
}
