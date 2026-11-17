use gpui::{AsyncApp, Context};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_media::map_view::LatLng;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::engine::rise_map_engine_models::RiseMapMutationOutcome;
use crate::modules::map::engine::rise_map_tracks_repository::MapPlaceSaveResult;
use crate::modules::map::stores::map_actions::map_actions::MapActionsStore;
use crate::modules::map::stores::map_actions::map_place_types::{MapPlaceSheetSession, MapPlaceSheetSessionPage};
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlaceKind;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;

impl MapInteractionsStore {
    pub fn hold_map(&mut self, coordinate: LatLng, cx: &mut Context<Self>) {
        let is_valid_coordinate = (-90.0..=90.0).contains(&coordinate.latitude)
            && (-180.0..=180.0).contains(&coordinate.longitude);
        if !MapServicesStore::is_member(cx)
            || !is_valid_coordinate
            || self.place_sheet.as_ref().is_some_and(|sheet| sheet.is_saving)
        {
            return;
        }
        self.select_friend(None, cx);
        self.select_grid(None, cx);
        self.set_place_sheet(Some(MapPlaceSheetSession::with_coordinate(coordinate)), cx);
    }

    pub fn select_place(&mut self, id: &str, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) || self.place_sheet.as_ref().is_some_and(|sheet| sheet.is_saving) {
            return;
        }
        let Some(place) = MapServicesStore::tracks(cx).places.into_iter().find(|place| place.id == id) else {
            return;
        };
        if place.kind != MapPlaceKind::Custom {
            return;
        }
        self.select_friend(None, cx);
        self.select_grid(None, cx);
        self.set_place_sheet(Some(MapPlaceSheetSession::new(place)), cx);
    }

    pub fn dismiss_place(&mut self, cx: &mut Context<Self>) {
        if self.place_sheet.as_ref().is_some_and(|sheet| sheet.is_saving) {
            return;
        }
        self.set_place_sheet(None, cx);
    }

    pub fn save_place(&mut self, session: &MapPlaceSheetSession, cx: &mut Context<Self>) {
        if !session.is_current() || !self.open_place_sheet_matches(session.id) || session.is_saving || !session.is_valid() {
            return;
        }
        let creating = session.is_creating;
        let draft = session.draft.clone();
        let session_id = session.id;
        if GuideSandbox::shared(cx).read(cx).owns_map_scene() {
            let saved = GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.save_map_place(&draft, cx));
            if let Some(sheet) = self.place_sheet.as_mut() {
                sheet.saved = Some(saved.clone());
                sheet.draft = saved;
                sheet.is_creating = false;
                sheet.page = MapPlaceSheetSessionPage::Details;
            }
            if creating {
                self.set_place_sheet(None, cx);
            }
            cx.notify();
            return;
        }
        if let Some(sheet) = self.place_sheet.as_mut() {
            sheet.is_saving = true;
            sheet.error_text = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let is_open = this
                .read_with(cx, |store, _| store.open_place_sheet_matches(session_id))
                .unwrap_or(false);
            if !is_open {
                return;
            }
            let result = cx
                .update(|cx| {
                    MapActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.save_place_action(draft, creating, cx))
                })
                .await;
            let _ = this.update(cx, |store, cx| {
                if !store.open_place_sheet_matches(session_id) {
                    return;
                }
                let Some(sheet) = store.place_sheet.as_mut() else {
                    return;
                };
                sheet.is_saving = false;
                match result {
                    MapPlaceSaveResult::Saved(confirmed) => {
                        sheet.saved = Some(confirmed.clone());
                        sheet.draft = confirmed;
                        sheet.is_creating = false;
                        sheet.page = MapPlaceSheetSessionPage::Details;
                        if creating {
                            store.set_place_sheet(None, cx);
                        }
                        GuideSignalBus::emit(RiseGuideSignal::MapPlaceSaved);
                    }
                    MapPlaceSaveResult::Failed(text) => {
                        sheet.error_text = Some(text.unwrap_or_else(|| tr("map_action_failed")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn delete_place(&mut self, session: &MapPlaceSheetSession, cx: &mut Context<Self>) {
        if !session.is_current() || !self.open_place_sheet_matches(session.id) || session.is_saving || session.is_creating {
            return;
        }
        if GuideSandbox::shared(cx).read(cx).owns_map_place(&session.draft.id) {
            let id = session.draft.id.clone();
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_map_place(&id, cx));
            self.set_place_sheet(None, cx);
            return;
        }
        let session_id = session.id;
        let draft = session.draft.clone();
        if let Some(sheet) = self.place_sheet.as_mut() {
            sheet.is_saving = true;
            sheet.error_text = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let is_open = this
                .read_with(cx, |store, _| store.open_place_sheet_matches(session_id))
                .unwrap_or(false);
            if !is_open {
                return;
            }
            let result = cx
                .update(|cx| {
                    MapActionsStore::shared(cx).update(cx, |actions, cx| actions.delete_place_action(draft, cx))
                })
                .await;
            let _ = this.update(cx, |store, cx| {
                if !store.open_place_sheet_matches(session_id) {
                    return;
                }
                match result {
                    RiseMapMutationOutcome::Done => store.set_place_sheet(None, cx),
                    RiseMapMutationOutcome::Failed(text) => {
                        if let Some(sheet) = store.place_sheet.as_mut() {
                            sheet.is_saving = false;
                            sheet.error_text = Some(text.unwrap_or_else(|| tr("map_action_failed")));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_place_sheet_matches(&self, session_id: Uuid) -> bool {
        self.place_sheet.as_ref().is_some_and(|sheet| sheet.id == session_id && sheet.is_current())
    }
}
