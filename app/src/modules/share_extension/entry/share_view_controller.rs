use std::rc::Rc;
use std::sync::Arc;

use futures::future::{FutureExt, LocalBoxFuture};
use gpui::{
    AnyView, App, AsyncApp, Context, Div, Entity, FocusHandle, FontWeight, Hsla, Render, Stateful,
    Subscription, Task, Window,
};
use rise_core::foundation::foundation_url::Url;
use rise_theme::ui_color_system::UIColor;
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::push_notifications::desktop_link_routing::handle_external_url;
use crate::modules::share_extension::entry::app_group_share_story_handoff::AppGroupShareStoryHandoff;
use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_dependencies::{
    ShareContainingAppOpening, ShareExtensionDependencyResolver, ShareStoryHandling,
};
use crate::modules::share_extension::entry::share_extension_models::{ShareExtensionError, SharePayload};
use crate::modules::share_extension::entry::share_extension_strings::{ShareErrorPresenter, ShareStrings};
use crate::modules::share_extension::entry::share_extension_view::{ShareExtensionView, ShareSurface};
use crate::modules::share_extension::entry::share_extension_view_model::ShareExtensionViewModel;
use crate::modules::share_extension::entry::share_item_loader::{ShareExtensionItem, ShareItemLoader};
use crate::modules::share_extension::entry::share_temporary_file_store::ShareTemporaryFileStore;
use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};

const SHEET_MAXIMUM_WIDTH: f32 = 620.0;
const SHEET_CORNER_RADIUS: f32 = 28.0;
const SHEET_OUTER_PADDING: f32 = 10.0;
const SHEET_INNER_PADDING: f32 = 24.0;
const SHEET_SPACING: f32 = 16.0;
const SCRIM_OPACITY: f32 = 0.42;
const LARGE_TITLE_ICON_SIDE: f32 = 34.0;

struct ShareViewControllerAppOpener;

impl ShareContainingAppOpening for ShareViewControllerAppOpener {
    fn open_containing_app(&self, url: Url, cx: AsyncApp) -> LocalBoxFuture<'_, bool> {
        async move {
            cx.update(|cx| handle_external_url(&url, cx));
            true
        }
        .boxed_local()
    }
}

pub struct ShareViewController {
    hosting_controller: Option<AnyView>,
    model: Option<Entity<ShareExtensionViewModel>>,
    temporary_file_store: Option<Arc<ShareTemporaryFileStore>>,
    did_resolve_request: bool,
    app_opener: Rc<dyn ShareContainingAppOpening>,
    input_items: Vec<ShareExtensionItem>,
    on_dismiss: Rc<dyn Fn(&mut App)>,
    _configuration_task: Option<Task<()>>,
    _release: Subscription,
}

impl ShareViewController {
    pub fn new(
        input_items: Vec<ShareExtensionItem>,
        on_dismiss: Rc<dyn Fn(&mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let release = cx.on_release(|this: &mut Self, cx: &mut App| this.view_did_disappear(cx));
        let mut this = Self {
            hosting_controller: None,
            model: None,
            temporary_file_store: None,
            did_resolve_request: false,
            app_opener: Rc::new(ShareViewControllerAppOpener),
            input_items,
            on_dismiss,
            _configuration_task: None,
            _release: release,
        };
        this.view_did_load(window, cx);
        this
    }

    fn view_did_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.configure_extension(window, cx);
    }

    fn view_did_disappear(&mut self, cx: &mut App) {
        if self.did_resolve_request {
            return;
        }
        if let Some(model) = self.model.take() {
            model.update(cx, |model, cx| model.cancel(cx));
        } else if let Some(temporary_file_store) = self.temporary_file_store.take() {
            cx.background_spawn(async move { temporary_file_store.remove_session_files() }).detach();
        }
    }

    fn configure_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let configuration = match ShareExtensionConfiguration::load() {
            Ok(configuration) => configuration,
            Err(error) => {
                self.show_failure(error.into(), window, cx);
                return;
            }
        };
        let temporary_file_store = cx.background_spawn(async { ShareTemporaryFileStore::new() });
        self._configuration_task = Some(cx.spawn_in(window, async move |this, cx| {
            let temporary_file_store = temporary_file_store.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_configuration(configuration, temporary_file_store, window, cx);
            });
        }));
    }

    fn finish_configuration(
        &mut self,
        configuration: ShareExtensionConfiguration,
        temporary_file_store: Result<ShareTemporaryFileStore, RiseShareContainerError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let temporary_file_store = match temporary_file_store {
            Ok(temporary_file_store) => Arc::new(temporary_file_store),
            Err(error) => {
                self.show_failure(error.into(), window, cx);
                return;
            }
        };
        self.temporary_file_store = Some(temporary_file_store.clone());

        let dependencies = match ShareExtensionDependencyResolver::resolve(
            configuration.clone(),
            &self.app_opener,
            temporary_file_store.clone(),
            cx,
        ) {
            Ok(dependencies) => dependencies,
            Err(error) => {
                let error = anyhow::Error::from(error);
                if Self::can_offer_story_without_session(&error) {
                    self.show_signed_out_story(
                        Self::is_signed_out(&error),
                        configuration,
                        temporary_file_store,
                        window,
                        cx,
                    );
                } else {
                    self.show_failure(error, window, cx);
                }
                return;
            }
        };

        let item_loader = ShareItemLoader::new(configuration.clone(), temporary_file_store.clone());
        let input_items = std::mem::take(&mut self.input_items);
        let finish_target = cx.weak_entity();
        let cancel_target = cx.weak_entity();
        let on_finish: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = finish_target.update(cx, |this, cx| this.complete_request(cx));
        });
        let on_cancel: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = cancel_target.update(cx, |this, cx| this.cancel_request(cx));
        });
        let model = cx.new(|_| {
            ShareExtensionViewModel::new(
                configuration,
                dependencies,
                item_loader,
                temporary_file_store,
                input_items,
                None,
                on_finish,
                on_cancel,
            )
        });
        self.model = Some(model.clone());
        let root_view = cx.new(|cx| ShareExtensionView::new(model, window, cx));
        self.install(AnyView::from(root_view), cx);
    }

    /// true when the error still allows offering a story without a session
    pub(crate) fn can_offer_story_without_session(error: &anyhow::Error) -> bool {
        Self::is_signed_out(error)
            || error.downcast_ref::<ShareExtensionError>() == Some(&ShareExtensionError::SnapshotUnavailable)
    }

    /// true when the error means signed out, no keychain branch since desktop reads the live session
    pub(crate) fn is_signed_out(error: &anyhow::Error) -> bool {
        error.downcast_ref::<ShareExtensionError>() == Some(&ShareExtensionError::AuthenticationUnavailable)
    }

    fn show_signed_out_story(
        &mut self,
        is_signed_out: bool,
        configuration: ShareExtensionConfiguration,
        temporary_file_store: Arc<ShareTemporaryFileStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item_loader = Rc::new(ShareItemLoader::new(configuration.clone(), temporary_file_store.clone()));
        let story_handoff = Rc::new(AppGroupShareStoryHandoff::new(
            configuration,
            RiseShareContainer::UNBOUND_STORY_ACCOUNT_ID.to_owned(),
            &self.app_opener,
            temporary_file_store,
        ));
        let input_items = std::mem::take(&mut self.input_items);
        let load_payload: ShareLoadPayload = Rc::new(move |cx: AsyncApp| {
            let item_loader = item_loader.clone();
            let input_items = input_items.clone();
            async move { item_loader.load(cx.background_executor().clone(), input_items).await }.boxed_local()
        });
        let story_target = cx.weak_entity();
        let add_to_story: ShareAddToStory = Rc::new(move |payload: SharePayload, cx: AsyncApp| {
            let story_handoff = story_handoff.clone();
            let story_target = story_target.clone();
            async move {
                story_handoff.handoff(payload, cx.clone()).await?;
                cx.update(|cx| {
                    let _ = story_target.update(cx, |this, cx| this.complete_request(cx));
                });
                Ok::<(), anyhow::Error>(())
            }
            .boxed_local()
        });
        let on_cancel = self.remove_files_and_cancel(cx);
        let root_view = cx.new(|cx| {
            ShareExtensionSignedOutView::new(is_signed_out, load_payload, add_to_story, on_cancel, window, cx)
        });
        self.install(AnyView::from(root_view), cx);
    }

    fn install(&mut self, root_view: AnyView, cx: &mut Context<Self>) {
        self.hosting_controller = Some(root_view);
        cx.notify();
    }

    fn show_failure(&mut self, error: anyhow::Error, window: &mut Window, cx: &mut Context<Self>) {
        let message = ShareErrorPresenter::message(&error);
        let on_cancel = self.remove_files_and_cancel(cx);
        let root_view = cx.new(|cx| ShareExtensionFailureView::new(message, on_cancel, window, cx));
        self.install(AnyView::from(root_view), cx);
    }

    fn remove_files_and_cancel(&self, cx: &mut Context<Self>) -> Rc<dyn Fn(&mut App)> {
        let this = cx.weak_entity();
        let temporary_file_store = self.temporary_file_store.clone();
        Rc::new(move |cx: &mut App| {
            let this = this.clone();
            let temporary_file_store = temporary_file_store.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                if let Some(temporary_file_store) = temporary_file_store {
                    cx.background_spawn(async move { temporary_file_store.remove_session_files() }).await;
                }
                cx.update(|cx| {
                    let _ = this.update(cx, |this, cx| this.cancel_request(cx));
                });
            })
            .detach();
        })
    }

    fn complete_request(&mut self, cx: &mut Context<Self>) {
        self.resolve_request(cx);
    }

    fn cancel_request(&mut self, cx: &mut Context<Self>) {
        self.resolve_request(cx);
    }

    fn resolve_request(&mut self, cx: &mut Context<Self>) {
        if self.did_resolve_request {
            return;
        }
        self.did_resolve_request = true;
        let on_dismiss = self.on_dismiss.clone();
        on_dismiss(cx);
    }
}

impl Render for ShareViewController {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().absolute().inset_0().children(self.hosting_controller.clone())
    }
}

/// scrim and bottom sheet, on desktop the sheet is an app modal and follows the app theme
fn share_sheet_layer(
    id: &'static str,
    focus_handle: &FocusHandle,
    fill: Hsla,
    content: impl IntoElement,
) -> Stateful<Div> {
    div()
        .id(id)
        .track_focus(focus_handle)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .flex_col()
        .items_center()
        .justify_end()
        .bg(Hsla { h: 0.0, s: 0.0, l: 0.0, a: SCRIM_OPACITY })
        .child(
            div().w_full().max_w(px(SHEET_MAXIMUM_WIDTH)).p(px(SHEET_OUTER_PADDING)).child(
                div()
                    .relative()
                    .w_full()
                    .bg(fill)
                    .continuous_corners(SHEET_CORNER_RADIUS)
                    .child(content)
                    .child(div().absolute().inset_0().child(centred_stroke(
                        ShareSurface::hairline(),
                        1.0,
                        Shape::RoundedRectangle {
                            corner_radius: SHEET_CORNER_RADIUS,
                            style: RoundedCornerStyle::Continuous,
                        },
                    ))),
            ),
        )
}

fn sheet_title(text: String) -> impl IntoElement {
    div()
        .swiftui_font(Font::title3().weight(FontWeight::BOLD))
        .text_color(ShareSurface::label())
        .text_center()
        .child(text)
}

fn sheet_message(text: String) -> impl IntoElement {
    div()
        .w_full()
        .swiftui_font(Font::subheadline())
        .text_color(ShareSurface::secondary_label())
        .text_center()
        .child(text)
}

struct ShareExtensionFailureView {
    message: String,
    on_cancel: Rc<dyn Fn(&mut App)>,
    focus_handle: FocusHandle,
}

impl ShareExtensionFailureView {
    fn new(message: String, on_cancel: Rc<dyn Fn(&mut App)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self { message, on_cancel, focus_handle }
    }
}

impl Render for ShareExtensionFailureView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_cancel = self.on_cancel.clone();
        let appearance = ShareSurface::capture(cx);
        let content = vstack(SHEET_SPACING)
            .w_full()
            .p(px(SHEET_INNER_PADDING))
            .children(IconUi::sized(
                "exclamationmark.triangle.fill",
                px(LARGE_TITLE_ICON_SIDE),
                UIColor::system_orange().resolved_color(appearance),
                false,
            ))
            .child(sheet_title(ShareStrings::error_title()))
            .child(sheet_message(self.message.clone()))
            .child(
                system_button("share-failure-cancel", BorderedStyle::Prominent)
                    .tint(ShareSurface::accent())
                    .control_size(ControlSize::Large)
                    .on_press(move |_, cx| on_cancel(cx))
                    .child(ShareStrings::cancel()),
            );
        let fill = ShareSurface::sheet();
        share_sheet_layer("share-failure-layer", &self.focus_handle, fill, content)
            .layer_keys(cancel_on_escape(self.on_cancel.clone()))
    }
}

fn cancel_on_escape(on_cancel: Rc<dyn Fn(&mut App)>) -> LayerCancel {
    LayerCancel::dismiss(move |_, cx| on_cancel(cx))
}

/// callback that loads the share payload for the signed out view
type ShareLoadPayload = Rc<dyn Fn(AsyncApp) -> LocalBoxFuture<'static, anyhow::Result<SharePayload>>>;
type ShareAddToStory = Rc<dyn Fn(SharePayload, AsyncApp) -> LocalBoxFuture<'static, anyhow::Result<()>>>;

struct ShareExtensionSignedOutView {
    is_signed_out: bool,
    add_to_story: ShareAddToStory,
    on_cancel: Rc<dyn Fn(&mut App)>,
    payload: Option<SharePayload>,
    is_loading: bool,
    is_opening: bool,
    error_message: Option<String>,
    focus_handle: FocusHandle,
    _load_task: Option<Task<()>>,
    _open_task: Option<Task<()>>,
}

impl ShareExtensionSignedOutView {
    fn new(
        is_signed_out: bool,
        load_payload: ShareLoadPayload,
        add_to_story: ShareAddToStory,
        on_cancel: Rc<dyn Fn(&mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let load_task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = load_payload(cx.clone()).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(payload) => this.payload = Some(payload),
                    Err(error) => this.error_message = Some(ShareErrorPresenter::message(&error)),
                }
                this.is_loading = false;
                cx.notify();
            });
        });
        Self {
            is_signed_out,
            add_to_story,
            on_cancel,
            payload: None,
            is_loading: true,
            is_opening: false,
            error_message: None,
            focus_handle,
            _load_task: Some(load_task),
            _open_task: None,
        }
    }

    fn can_add_to_story(&self) -> bool {
        self.is_loading || self.payload.as_ref().is_some_and(SharePayload::can_share_to_story)
    }

    fn message(&self) -> String {
        if !self.is_loading && !self.payload.as_ref().is_some_and(SharePayload::can_share_to_story) {
            return if self.is_signed_out {
                ShareStrings::authentication_unavailable()
            } else {
                ShareStrings::snapshot_unavailable()
            };
        }
        if self.is_signed_out {
            ShareStrings::signed_out_story_message()
        } else {
            ShareStrings::open_app_story_message()
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.clone() else {
            return;
        };
        if !payload.can_share_to_story() || self.is_opening {
            return;
        }
        self.is_opening = true;
        self.error_message = None;
        cx.notify();
        let add_to_story = self.add_to_story.clone();
        self._open_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            if let Err(error) = add_to_story(payload, cx.clone()).await {
                let _ = this.update(cx, |this, cx| {
                    this.error_message = Some(ShareErrorPresenter::message(&error));
                    this.is_opening = false;
                    cx.notify();
                });
            }
        }));
    }
}

impl Render for ShareExtensionSignedOutView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ShareSurface::capture(cx);
        let is_opening = self.is_opening;
        let title = if self.is_signed_out {
            ShareStrings::signed_out_title()
        } else {
            ShareStrings::open_app_title()
        };
        let message = self.error_message.clone().unwrap_or_else(|| self.message());
        let view = cx.entity().downgrade();
        let on_cancel = self.on_cancel.clone();

        let content = vstack(SHEET_SPACING)
            .w_full()
            .p(px(SHEET_INNER_PADDING))
            .children(IconUi::sized(
                "person.crop.circle.badge.checkmark",
                px(LARGE_TITLE_ICON_SIDE),
                ShareSurface::accent(),
                false,
            ))
            .child(sheet_title(title))
            .child(sheet_message(message))
            .when(self.can_add_to_story(), |stack| {
                stack.child(
                    div().w_full().child(
                        system_button("share-signed-out-story", BorderedStyle::Prominent)
                            .tint(ShareSurface::accent())
                            .control_size(ControlSize::Large)
                            .disabled(is_opening)
                            .on_press(move |_, cx| {
                                let _ = view.update(cx, |view, cx| view.open(cx));
                            })
                            .child(if is_opening {
                                progress_view().into_any_element()
                            } else {
                                ShareStrings::share_to_story().into_any_element()
                            }),
                    ),
                )
            })
            .child(
                system_button("share-signed-out-cancel", BorderedStyle::Bordered)
                    .tint(ShareSurface::accent())
                    .control_size(ControlSize::Large)
                    .disabled(is_opening)
                    .on_press(move |_, cx| on_cancel(cx))
                    .child(ShareStrings::cancel()),
            );

        let on_cancel = self.on_cancel.clone();
        let escape_view = cx.entity().downgrade();
        share_sheet_layer("share-signed-out-layer", &self.focus_handle, ShareSurface::sheet(), content).layer_keys(
            LayerCancel::dismiss(move |_, cx| {
                let can_cancel = escape_view.read_with(cx, |view, _| !view.is_opening).unwrap_or(false);
                if can_cancel {
                    on_cancel(cx);
                }
            }),
        )
    }
}
