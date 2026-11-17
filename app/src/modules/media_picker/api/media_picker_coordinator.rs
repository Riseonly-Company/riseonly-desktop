use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_theme::ThemeStore;

use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;

/// completion callback, the app context stands in for the main actor
pub type MediaPickerCompletion = Box<dyn FnOnce(MediaPickerResult, &mut App)>;

pub struct MediaPickerCoordinatorActivePresentation {
    pub id: Uuid,
    pub configuration: MediaPickerConfiguration,
    pub theme_store: Entity<ThemeStore>,
    completion: MediaPickerCompletion,
}

/// one active picker presentation at a time with its configuration and theme and completion
pub struct MediaPickerCoordinator {
    active: Option<MediaPickerCoordinatorActivePresentation>,
}

impl MediaPickerCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { active: None }
    }

    pub fn active(&self) -> Option<&MediaPickerCoordinatorActivePresentation> {
        self.active.as_ref()
    }

    pub fn present(
        &mut self,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        completion: MediaPickerCompletion,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some() {
            completion(MediaPickerResult::CANCELLED, cx);
            return;
        }
        self.active = Some(MediaPickerCoordinatorActivePresentation {
            id: Uuid::new_v4(),
            configuration,
            theme_store,
            completion,
        });
        cx.notify();
    }

    pub fn complete(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        let callback = self.active.take().map(|active| active.completion);
        cx.notify();
        if let Some(callback) = callback {
            callback(result, cx);
        }
    }

    pub fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        self.complete(MediaPickerResult::CANCELLED, cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.active = None;
        cx.notify();
    }
}
