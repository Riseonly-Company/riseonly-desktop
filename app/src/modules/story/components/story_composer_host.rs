use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, SharedString, Subscription, Window};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::prelude::*;

use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerResult, MediaPickerTrackSelection,
};
use crate::modules::story::components::story_composer_picker_hooks::story_composer_picker_hooks;
use crate::modules::story::components::story_create_sheet::StoryCreateSheet;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

/// called when the story composer host is dismissed
pub type StoryComposerHostDismissed = Rc<dyn Fn(&mut Window, &mut App)>;

/// composer request from the feed stories tray, a list row cant present so a screen level host does it
pub struct StoryFeedComposerPresenter {
    pub is_presented: bool,
}

impl StoryFeedComposerPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_presented: false }
    }

    pub fn present(&mut self, cx: &mut Context<Self>) {
        self.is_presented = true;
        cx.notify();
    }
}

/// observer entity of the composer mount, only this leaf observes the flag
struct StoryFeedComposerMountState {
    presenter: Entity<StoryFeedComposerPresenter>,
    _observation: Subscription,
}

/// screen level host for the story feed composer presenter
#[derive(IntoElement)]
pub struct StoryFeedComposerMount {
    id: ElementId,
}

impl StoryFeedComposerMount {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for StoryFeedComposerMount {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx: &mut Context<StoryFeedComposerMountState>| {
            let presenter = StoryFeedComposerPresenter::shared(cx);
            let observation = cx.observe(&presenter, |_, _, cx| cx.notify());
            StoryFeedComposerMountState { presenter, _observation: observation }
        });
        let presenter = state.read(cx).presenter.clone();
        StoryComposerHost::new(
            ElementId::NamedChild(Arc::new(self.id), SharedString::new_static("host")),
            Binding::entity(presenter, |presenter| presenter.is_presented, |presenter, value, _| presenter.is_presented = value),
        )
    }
}

/// state of the host, alive as long as its drawn
#[derive(Default)]
struct StoryComposerHostState {
    show_picker: bool,
    show_create_options: bool,
    selected_item: Option<MediaPickerItem>,
    selected_caption: String,
    selected_track: Option<MediaPickerTrackSelection>,
    opens_create_options_after_picker_dismissal: bool,
}

impl StoryComposerHostState {
    fn clear_selection(&mut self) {
        self.selected_item = None;
        self.selected_caption = String::new();
        self.selected_track = None;
        self.opens_create_options_after_picker_dismissal = false;
    }

    fn handle_picker_result(&mut self, result: MediaPickerResult) {
        let attached_track = result.attached_track;
        let item = if result.was_cancelled { None } else { result.items.into_iter().next() };
        let Some(item) = item else {
            self.opens_create_options_after_picker_dismissal = false;
            return;
        };

        let caption = item
            .image_payload
            .as_ref()
            .map(|image| image.caption.clone())
            .or_else(|| item.video_payload.as_ref().map(|video| video.caption.clone()))
            .unwrap_or_default();
        self.selected_item = Some(item);
        self.selected_caption = caption;
        self.selected_track = attached_track;
        self.opens_create_options_after_picker_dismissal = true;
    }

    fn open_create_options_on_completion(&mut self) -> bool {
        if !self.opens_create_options_after_picker_dismissal || self.selected_item.is_none() {
            return false;
        }
        self.opens_create_options_after_picker_dismissal = false;
        self.show_picker = false;
        self.show_create_options = true;
        true
    }
}

#[derive(IntoElement)]
pub struct StoryComposerHost {
    id: ElementId,
    is_presented: Binding<bool>,
    initial_track: Option<MediaPickerTrackSelection>,
    on_dismissed: StoryComposerHostDismissed,
}

impl StoryComposerHost {
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>) -> Self {
        Self {
            id: id.into(),
            is_presented,
            initial_track: None,
            on_dismissed: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn initial_track(mut self, initial_track: Option<MediaPickerTrackSelection>) -> Self {
        self.initial_track = initial_track;
        self
    }

    pub fn on_dismissed(mut self, on_dismissed: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismissed = Rc::new(on_dismissed);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn open_picker(
        state: &Entity<StoryComposerHostState>,
        is_presented: &Binding<bool>,
        on_dismissed: &StoryComposerHostDismissed,
        window: &mut Window,
        cx: &mut App,
    ) {
        let service = StoryServicesStore::shared(cx);
        if service.read(cx).is_publishing {
            is_presented.set(false, window, cx);
            on_dismissed(window, cx);
            return;
        }
        service.update(cx, |service, cx| service.reset_publish_state(cx));
        state.update(cx, |state, cx| {
            state.show_picker = true;
            cx.notify();
        });
    }

    fn handle_picker_dismissed(
        state: &Entity<StoryComposerHostState>,
        is_presented: &Binding<bool>,
        on_dismissed: &StoryComposerHostDismissed,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (opens_create_options, has_selection, cover_is_open) = {
            let state = state.read(cx);
            (
                state.opens_create_options_after_picker_dismissal,
                state.selected_item.is_some(),
                state.show_create_options,
            )
        };
        tracing::warn!(target: "riseonly::X808", opens_create_options, has_selection, cover_is_open, "story picker dismissed");
        if cover_is_open {
            return;
        }
        if !opens_create_options || !has_selection {
            on_dismissed(window, cx);
            return;
        }
        state.update(cx, |state, _| state.opens_create_options_after_picker_dismissal = false);
        is_presented.set(false, window, cx);
        let state = state.clone();
        window.defer(cx, move |_, cx| {
            state.update(cx, |state, cx| {
                if state.selected_item.is_some() {
                    state.show_create_options = true;
                    cx.notify();
                }
            });
        });
    }

    fn cover(
        &self,
        state: &Entity<StoryComposerHostState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let (item, caption, track) = {
            let read = state.read(cx);
            if !read.show_create_options {
                return None;
            }
            (read.selected_item.clone()?, read.selected_caption.clone(), read.selected_track.clone())
        };
        let presented = Binding::entity(
            state.clone(),
            |state: &StoryComposerHostState| state.show_create_options,
            |state, value, _| state.show_create_options = value,
        );
        let scrim_presented = presented.clone();
        let published_state = state.clone();
        let sheet = StoryCreateSheet::new(self.child_id("create"), item, caption, presented, move |_, cx| {
            published_state.update(cx, |state, cx| {
                state.clear_selection();
                cx.notify();
            });
        })
        .initial_track(track)
        .in_modal(true);
        Some(sheet.modal_cover(scrim_presented, window, cx))
    }
}

impl RenderOnce for StoryComposerHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| StoryComposerHostState::default());
        let presented_now = self.is_presented.get(cx);
        let (show_picker, show_create_options) = {
            let read = state.read(cx);
            (read.show_picker, read.show_create_options)
        };

        {
            let (state, is_presented, on_dismissed) = (state.clone(), self.is_presented.clone(), self.on_dismissed.clone());
            on_change(self.child_id("is-presented"), presented_now, false, window, cx, move |_, open, window, cx| {
                if *open {
                    Self::open_picker(&state, &is_presented, &on_dismissed, window, cx);
                } else if state.read(cx).show_picker {
                    state.update(cx, |state, cx| {
                        state.show_picker = false;
                        cx.notify();
                    });
                }
            });
        }
        {
            let is_presented = self.is_presented.clone();
            on_change(self.child_id("show-picker"), show_picker, false, window, cx, move |_, open, window, cx| {
                tracing::warn!(target: "riseonly::X808", open = *open, host_presented = is_presented.get(cx), "story picker flag changed");
                if !*open && is_presented.get(cx) {
                    is_presented.set(false, window, cx);
                }
            });
        }
        {
            let (state, on_dismissed) = (state.clone(), self.on_dismissed.clone());
            on_change(self.child_id("show-create"), show_create_options, false, window, cx, move |_, open, window, cx| {
                if !*open {
                    state.update(cx, |state, cx| {
                        state.clear_selection();
                        cx.notify();
                    });
                    on_dismissed(window, cx);
                }
            });
        }

        let cover = self.cover(&state, window, cx);
        let picker_presented = Binding::entity(
            state.clone(),
            |state: &StoryComposerHostState| state.show_picker,
            |state, value, _| state.show_picker = value,
        );
        let complete_state = state.clone();
        let (dismissed_state, dismissed_presented, dismissed_action) =
            (state.clone(), self.is_presented.clone(), self.on_dismissed.clone());
        let picker = story_composer_picker_hooks().map(|hooks| {
            (hooks.media_picker_sheet)(
                self.child_id("picker"),
                picker_presented,
                MediaPickerConfiguration::story(),
                ThemeStore::shared(cx),
                self.initial_track.clone(),
                Box::new(move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                    tracing::warn!(target: "riseonly::X808", cancelled = result.was_cancelled, items = result.items.len(), "story picker complete");
                    let opened = complete_state.update(cx, |state, cx| {
                        state.handle_picker_result(result);
                        let opened = state.open_create_options_on_completion();
                        cx.notify();
                        opened
                    });
                    tracing::warn!(target: "riseonly::X808", opened, "story create cover on completion");
                }),
                Box::new(move |window: &mut Window, cx: &mut App| {
                    Self::handle_picker_dismissed(
                        &dismissed_state,
                        &dismissed_presented,
                        &dismissed_action,
                        window,
                        cx,
                    );
                }),
            )
        });

        let (appear_state, appear_presented, appear_dismissed) =
            (state.clone(), self.is_presented.clone(), self.on_dismissed.clone());
        div()
            .absolute()
            .inset_0()
            .when(!show_create_options, |root| root.children(picker))
            .children(cover)
            .on_appear(self.child_id("appear"), move |window, cx| {
                if appear_presented.get(cx) {
                    Self::open_picker(&appear_state, &appear_presented, &appear_dismissed, window, cx);
                }
            })
    }
}
