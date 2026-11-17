use std::rc::Rc;

use gpui::{App, Context, Entity, Subscription, Window};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, RootRoute};
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
use crate::modules::camera::components::feed_media_picker_reveal::{CreatePostMediaHostActions, create_post_media_host};
use crate::modules::media_picker::api::media_picker_configuration::{MediaPickerConfiguration, MediaPickerSourceTab};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerResult, MediaPickerTrackSelection};
use crate::modules::media_picker::editor::media_editor_screen::MediaEditorOutcome;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowPresenter;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::ui::media_picker_grid_view::MEDIA_PICKER_REFRESH_CELL_FRAMES;
use crate::modules::media_picker::ui::media_picker_screen::{MediaPickerLayout, MediaPickerScreen};
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;
use crate::modules::user::pages::profile::create_post::create_post::CreatePostScreenState;

const CONFIRM_BUTTON_SYSTEM_IMAGE: &str = "arrow.right";

/// single shot handoff of the exported picker selection to the details step
/// a route payload would hash blobs on every path change and other entry points must not get
/// stale media
pub struct CreatePostMediaHandoff {
    pending: Option<(CreatePostIntent, MediaPickerResult)>,
}

impl CreatePostMediaHandoff {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { pending: None })
    }

    pub fn set(&mut self, intent: CreatePostIntent, result: MediaPickerResult) {
        self.pending = Some((intent, result));
    }

    pub fn has_pending(&self, intent: CreatePostIntent) -> bool {
        self.pending.as_ref().is_some_and(|(pending, _)| *pending == intent)
    }

    pub fn take(&mut self, intent: CreatePostIntent) -> Option<MediaPickerResult> {
        if !self.has_pending(intent) {
            return None;
        }
        self.pending.take().map(|(_, result)| result)
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }

    pub fn reset_account_state(&mut self) {
        self.pending = None;
    }
}

pub struct CreatePostMediaPage {
    intent: CreatePostIntent,
    track_seed: Option<MediaPickerTrackSelection>,
    journey_id: Option<String>,
    session: Option<Entity<MediaPickerSession>>,
    pushed_details: bool,
    allows_cancel_pop: bool,
    nav_store: Option<Entity<NavigationStore>>,
    host: Option<CreatePostMediaHostActions>,
    editor_was_presented: bool,
    error_alert_is_up: bool,
    _session_observations: Vec<Subscription>,
}

impl CreatePostMediaPage {
    pub fn new(
        intent: CreatePostIntent,
        track_seed: Option<MediaPickerTrackSelection>,
        journey_id: Option<String>,
    ) -> Self {
        Self {
            intent,
            track_seed,
            journey_id,
            session: None,
            pushed_details: false,
            allows_cancel_pop: true,
            nav_store: None,
            host: None,
            editor_was_presented: false,
            error_alert_is_up: false,
            _session_observations: Vec::new(),
        }
    }

    fn configuration(cx: &App) -> MediaPickerConfiguration {
        let mut configuration =
            MediaPickerConfiguration::post_media_for_tier(CreatePostScreenState::author_is_premium(cx));
        configuration.confirm_button_system_image = CONFIRM_BUTTON_SYSTEM_IMAGE.to_owned();
        configuration
    }

    fn on_appear(&mut self, cx: &mut Context<Self>) {
        self.pushed_details = false;
        self.allows_cancel_pop = true;
        CreatePostMediaHandoff::shared(cx).update(cx, |handoff, _| handoff.clear());
        self.boot_session_if_needed(cx);
        self.sync_editor_window(cx);
        cx.notify();
    }

    fn on_disappear(&mut self, cx: &mut Context<Self>) {
        if self.pushed_details {
            if let Some(session) = self.session.clone() {
                MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| presenter.teardown(&session, cx));
            }
        } else {
            self.tear_down_session(cx);
        }
    }

    fn boot_session_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.session.is_some() {
            return;
        }
        let configuration = Self::configuration(cx);
        let theme_store = ThemeStore::shared(cx);
        let track_seed = self.track_seed.clone();
        let weak = cx.weak_entity();
        let on_complete: Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)> =
            Rc::new(move |result: MediaPickerResult, window: &mut Window, cx: &mut App| {
                let weak = weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |page, cx| page.handle_export_complete(result, window, cx));
                });
            });
        let session = cx.new(|cx| {
            MediaPickerSession::new(configuration, theme_store, None, None, track_seed, on_complete, cx)
        });
        let editor_presentation = session.read(cx).editor_presentation.clone();
        self._session_observations = vec![
            cx.observe(&session, |page, _, cx| {
                page.present_error_if_needed(cx);
                cx.notify();
            }),
            cx.observe(&editor_presentation, |page, presentation, cx| {
                let is_presented = presentation.read(cx).is_presented;
                if is_presented != page.editor_was_presented {
                    page.editor_was_presented = is_presented;
                    page.sync_editor_window(cx);
                }
            }),
        ];
        self.session = Some(session);
        RiseAppEvents::default_center().post(MEDIA_PICKER_REFRESH_CELL_FRAMES);
    }

    fn present_error_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(message) = session.read(cx).error_text.clone() else {
            return;
        };
        if self.error_alert_is_up {
            return;
        }
        self.error_alert_is_up = true;
        let weak = cx.weak_entity();
        NativeConfirmAlert::present_notice(
            &tr("error"),
            &message,
            None,
            None,
            move |cx| {
                session.update(cx, |session, cx| {
                    session.error_text = None;
                    cx.notify();
                });
                let _ = weak.update(cx, |page, _| page.error_alert_is_up = false);
            },
            cx,
        );
    }

    fn handle_export_complete(&mut self, result: MediaPickerResult, window: &mut Window, cx: &mut Context<Self>) {
        if result.was_cancelled || result.items.is_empty() {
            if let Some(host) = self.host.clone() {
                (host.on_dismiss)(window, cx);
            } else if self.allows_cancel_pop
                && let Some(nav_store) = self.nav_store.clone()
            {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
            return;
        }
        let intent = self.intent;
        CreatePostMediaHandoff::shared(cx).update(cx, |handoff, _| handoff.set(intent, result));
        self.pushed_details = true;
        if let Some(nav_store) = self.nav_store.clone() {
            let route = RootRoute::CreatePost { intent, journey_id: self.journey_id.clone() };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
        if let Some(host) = self.host.clone() {
            (host.on_details_pushed)(window, cx);
        }
        if let Some(session) = self.session.clone() {
            session.update(cx, |session, cx| session.reset_completion(cx));
        }
    }

    fn tear_down_session(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.allows_cancel_pop = false;
        let editor_presentation = session.read(cx).editor_presentation.clone();
        if editor_presentation.read(cx).is_presented {
            editor_presentation.update(cx, |presentation, cx| presentation.complete_flyback_handoff(cx));
        }
        MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| presenter.teardown(&session, cx));
        if !session.read(cx).did_finish() {
            session.update(cx, |session, cx| session.cancel(cx));
        }
        self._session_observations.clear();
        self.session = None;
        self.editor_was_presented = false;
    }

    fn sync_editor_window(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let configuration = session.read(cx).configuration.clone();
        let theme_store = ThemeStore::shared(cx);
        let outcome_session = session.clone();
        MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.sync_presentation(
                session,
                configuration,
                theme_store,
                Rc::new(move |outcome: MediaEditorOutcome, asset: MediaAsset, _: &mut Window, cx: &mut App| {
                    outcome_session.update(cx, |session, cx| session.handle_editor_outcome(outcome, asset, cx));
                }),
                cx,
            );
        });
    }

    fn screen(&self, session: &Entity<MediaPickerSession>, cx: &App) -> MediaPickerScreen {
        let read = session.read(cx);
        let on_open_camera: Option<Rc<dyn Fn(&mut Window, &mut App)>> =
            (read.offers_camera_tile() && self.track_seed.is_none()).then(|| {
                Rc::new(|_: &mut Window, cx: &mut App| {
                    RiseCameraPresenter::shared(cx).update(cx, |presenter, cx| presenter.present_from_picker(cx));
                }) as Rc<dyn Fn(&mut Window, &mut App)>
            });
        let (cancel, outcome, send, document) = (session.clone(), session.clone(), session.clone(), session.clone());
        MediaPickerScreen::new(
            read.configuration.clone(),
            read.theme_store.clone(),
            read.selection.clone(),
            read.caption.clone(),
            read.fetcher.clone(),
            read.editor_presentation.clone(),
            move |_, cx| cancel.update(cx, |session, cx| session.cancel(cx)),
            move |editor_outcome, asset, _, cx| {
                outcome.update(cx, |session, cx| session.handle_editor_outcome(editor_outcome, asset, cx));
            },
            move |send_as_file, _, cx| send.update(cx, |session, cx| session.export_selection(send_as_file, cx)),
            move |_, cx| {
                document.update(cx, |session, cx| {
                    session.present_document_picker(MediaPickerSourceTab::File, cx);
                });
            },
        )
        .layout(MediaPickerLayout::FullScreen)
        .import_session(session.downgrade())
        .on_open_camera(on_open_camera)
    }
}

impl Render for CreatePostMediaPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        self.host = create_post_media_host(cx);
        let theme = rise_ui::theme(cx).clone();
        let appear = cx.weak_entity();
        let disappear = cx.weak_entity();
        div()
            .id("create_post_media_page")
            .size_full()
            .relative()
            .bg(theme.bg._000)
            .children(self.session.as_ref().map(|session| self.screen(session, cx)))
            .on_appear("create_post_media_page_appear", move |_, cx| {
                let _ = appear.update(cx, |page, cx| page.on_appear(cx));
            })
            .on_disappear("create_post_media_page_disappear", move |_, cx| {
                let _ = disappear.update(cx, |page, cx| page.on_disappear(cx));
            })
    }
}
