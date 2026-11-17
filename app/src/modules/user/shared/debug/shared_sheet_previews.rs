#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, AsyncApp, Context, Entity, Render, Subscription, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore};
use crate::core::widgets::sheets::report_sheet::ReportSheet;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::components::bot_ui_components::BotOneTimeTokenSheet;
use crate::modules::chat::components::folders::chat_folder_editor_store::ChatFolderEditorStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{ReportReason, ReportReasonExt};
use crate::modules::user::pages::profile::settings::privacy_settings::abuse_appeal_sheet::AbuseAppealSheet;
use crate::modules::vacancy::components::resume_form_ui::{GeoCityPickerSheet, GeoCountryPickerSheet};

/// closes the preview, the one action every preview button shares
pub(super) fn debug_close(open: Binding<bool>) -> impl Fn(&mut Window, &mut App) + 'static {
    move |window: &mut Window, cx: &mut App| open.set(false, window, cx)
}

pub enum SharedSheetPreviews {}

impl SharedSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![
            BottomSheetDebugRegistration::new(
                "other.report.content",
                BottomSheetDebugRegistrationModule::Other,
                "report_sheet_title",
                "ReportSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    ReportSheet::new()
                        .presentation_override(Some(open))
                        .reasons_override(Some(ReportReason::content_reasons().to_vec()))
                },
            ),
            BottomSheetDebugRegistration::new(
                "other.report.user",
                BottomSheetDebugRegistrationModule::Other,
                "report_sheet_title",
                "ReportSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    ReportSheet::new()
                        .presentation_override(Some(open))
                        .reasons_override(Some(ReportReason::user_reasons().to_vec()))
                },
            )
            .with_variant(Some("navbtn_profile".to_owned())),
            Self::global(
                "other.people",
                BottomSheetDebugRegistrationModule::Other,
                "people_picker_search_placeholder",
                "PeoplePickerSheet",
                PeoplePickerStore::shared,
                PeoplePickerStore::is_presented,
                |cx: &mut App| {
                    PeoplePickerStore::shared(cx).update(cx, |store, cx| {
                        store.present(
                            PeoplePickerMode::SearchAll,
                            10,
                            Vec::new(),
                            HashSet::new(),
                            tr("people_picker_search_placeholder"),
                            None,
                            PeoplePickerHostContext::Root,
                            |_, _: &mut App| {},
                            cx,
                        );
                    });
                },
                |cx: &mut App| PeoplePickerStore::shared(cx).update(cx, |store, cx| store.dismiss(cx)),
            ),
            Self::global(
                "other.share",
                BottomSheetDebugRegistrationModule::Other,
                "ctx_share",
                "ShareSheet",
                ShareSheetStore::shared,
                ShareSheetStore::is_visible,
                |cx: &mut App| {
                    let auth = AuthServicesStore::shared(cx);
                    let profile = auth.read(cx).profile().cloned();
                    if let Some(profile) = profile {
                        ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_profile(profile, cx));
                    }
                },
                |cx: &mut App| ShareSheetStore::shared(cx).update(cx, |store, cx| store.dismiss(cx)),
            ),
            Self::global(
                "chats.folder",
                BottomSheetDebugRegistrationModule::Chats,
                "folder_create",
                "ChatFolderEditSheet",
                ChatFolderEditorStore::shared,
                ChatFolderEditorStore::is_presented,
                |cx: &mut App| ChatFolderEditorStore::shared(cx).update(cx, |store, cx| store.present_create(cx)),
                |cx: &mut App| ChatFolderEditorStore::shared(cx).update(cx, |store, cx| store.close(cx)),
            ),
            BottomSheetDebugRegistration::new(
                "jobs.country",
                BottomSheetDebugRegistrationModule::Jobs,
                "vacancy_resume_country",
                "GeoCountryPickerSheet",
                |open: Binding<bool>, _: &mut Window, cx: &mut App| {
                    let close = open.clone();
                    GeoCountryPickerSheet::new("jobs.country.sheet", move |_, window: &mut Window, cx: &mut App| {
                        close.set(false, window, cx);
                    })
                    .presented("jobs.country", open, cx)
                },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live),
            BottomSheetDebugRegistration::new(
                "jobs.city",
                BottomSheetDebugRegistrationModule::Jobs,
                "vacancy_field_city",
                "GeoCityPickerSheet",
                |open: Binding<bool>, _: &mut Window, cx: &mut App| {
                    let close = open.clone();
                    GeoCityPickerSheet::new(
                        "jobs.city.sheet",
                        "Kazakhstan",
                        move |_, window: &mut Window, cx: &mut App| close.set(false, window, cx),
                    )
                    .presented("jobs.city", open, cx)
                },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live),
            BottomSheetDebugRegistration::native(
                "bots.token",
                BottomSheetDebugRegistrationModule::Bots,
                "bot_token_created_title",
                "BotOneTimeTokenSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    BotOneTimeTokenSheet::new(
                        "bots.token.sheet",
                        "Design preview bot",
                        "preview-token-not-valid-for-authentication",
                        debug_close(open),
                    )
                },
            ),
            BottomSheetDebugRegistration::native(
                "profile.abuse_appeal",
                BottomSheetDebugRegistrationModule::Profile,
                "abuse_appeal_title",
                "AbuseAppealSheet",
                |_: Binding<bool>, _: &mut Window, _: &mut App| AbuseAppealSheet::new(),
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn global<S: 'static>(
        id: &'static str,
        module: BottomSheetDebugRegistrationModule,
        title: &'static str,
        source: &'static str,
        store: fn(&mut App) -> Entity<S>,
        presented: fn(&S) -> bool,
        launch: impl Fn(&mut App) + 'static,
        cancel: impl Fn(&mut App) + 'static,
    ) -> BottomSheetDebugRegistration {
        let app: Rc<RefCell<Option<AsyncApp>>> = Rc::new(RefCell::new(None));
        let content_app = app.clone();
        let launch: Rc<dyn Fn(&mut App)> = Rc::new(launch);
        let cancel: Rc<dyn Fn(&mut App)> = Rc::new(cancel);
        BottomSheetDebugRegistration::new(
            id,
            module,
            title,
            source,
            move |open: Binding<bool>, window: &mut Window, cx: &mut App| {
                *content_app.borrow_mut() = Some(cx.to_async());
                let launch = launch.clone();
                let entity = store(cx);
                window.use_keyed_state(format!("{id}.launcher"), cx, move |_, cx| {
                    DebugGlobalSheetLauncher::new(id, open, entity, presented, launch, cx)
                })
            },
        )
        .with_mode(BottomSheetDebugRegistrationMode::Live)
        .on_cancel(move || {
            let Some(app) = app.borrow().clone() else {
                return;
            };
            let cancel = cancel.clone();
            app.spawn(async move |cx| {
                cx.update(|cx| cancel(cx));
            })
            .detach();
        })
    }
}

pub struct DebugGlobalSheetLauncher {
    id: &'static str,
    is_presented: Binding<bool>,
    presented: Rc<dyn Fn(&App) -> bool>,
    launch: Rc<dyn Fn(&mut App)>,
    _store_observer: Subscription,
}

impl DebugGlobalSheetLauncher {
    fn new<S: 'static>(
        id: &'static str,
        is_presented: Binding<bool>,
        store: Entity<S>,
        presented: fn(&S) -> bool,
        launch: Rc<dyn Fn(&mut App)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let store_observer = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            id,
            is_presented,
            presented: Rc::new(move |cx: &App| presented(store.read(cx))),
            launch,
            _store_observer: store_observer,
        }
    }
}

impl Render for DebugGlobalSheetLauncher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let launch = self.launch.clone();
        on_change(
            format!("{}.launcher.open", self.id),
            self.is_presented.get(cx),
            true,
            window,
            cx,
            move |_, open: &bool, _, cx| {
                if *open {
                    launch(cx);
                }
            },
        );
        let is_presented = self.is_presented.clone();
        on_change(
            format!("{}.launcher.presented", self.id),
            (self.presented)(cx),
            false,
            window,
            cx,
            move |_, open: &bool, window, cx| {
                if !*open {
                    is_presented.set(false, window, cx);
                }
            },
        );
        div().w(px(0.0)).h(px(0.0))
    }
}
