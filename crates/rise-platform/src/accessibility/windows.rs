use ::windows::Foundation::TypedEventHandler;
use ::windows::UI::ViewManagement::{UISettings, UISettingsAnimationsEnabledChangedEventArgs};
use ::windows::core::IInspectable;

use super::accessibility::*;

pub(super) struct NativeAccessibility;

impl AccessibilitySettings for NativeAccessibility {
    fn reduce_transparency(&self) -> Result<bool, AccessibilityUnavailable> {
        read_reduce_transparency(&ui_settings()?)
    }

    fn watch_reduce_transparency(
        &self,
        sink: ReduceTransparencySink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        let settings = ui_settings()?;
        let handler =
            TypedEventHandler::<UISettings, IInspectable>::new(move |sender, _args| {
                if let Some(sender) = sender.as_ref() {
                    deliver(&sink, read_reduce_transparency(sender), "reduce transparency");
                }
                Ok(())
            });
        let token = settings
            .AdvancedEffectsEnabledChanged(&handler)
            .map_err(|error| refused("AdvancedEffectsEnabledChanged", error))?;
        Ok(AccessibilityWatch::new(Registration {
            settings,
            token,
            event: Event::AdvancedEffects,
        }))
    }

    fn reduce_motion(&self) -> Result<bool, AccessibilityUnavailable> {
        read_reduce_motion(&ui_settings()?)
    }

    fn watch_reduce_motion(
        &self,
        sink: ReduceMotionSink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        let settings = ui_settings()?;
        let handler = TypedEventHandler::<UISettings, UISettingsAnimationsEnabledChangedEventArgs>::new(
            move |sender, _args| {
                if let Some(sender) = sender.as_ref() {
                    deliver(&sink, read_reduce_motion(sender), "reduce motion");
                }
                Ok(())
            },
        );
        let token = settings
            .AnimationsEnabledChanged(&handler)
            .map_err(|error| refused("AnimationsEnabledChanged", error))?;
        Ok(AccessibilityWatch::new(Registration {
            settings,
            token,
            event: Event::Animations,
        }))
    }
}

fn ui_settings() -> Result<UISettings, AccessibilityUnavailable> {
    UISettings::new().map_err(|error| refused("UISettings activation", error))
}

fn read_reduce_transparency(settings: &UISettings) -> Result<bool, AccessibilityUnavailable> {
    settings
        .AdvancedEffectsEnabled()
        .map(reduce_transparency_from_advanced_effects)
        .map_err(|error| refused("AdvancedEffectsEnabled", error))
}

/// animation effects off means reduce motion on
fn read_reduce_motion(settings: &UISettings) -> Result<bool, AccessibilityUnavailable> {
    settings
        .AnimationsEnabled()
        .map(|enabled| !enabled)
        .map_err(|error| refused("AnimationsEnabled", error))
}

/// hands a value read in a change event to the sink, on the thread pool thread of the event
fn deliver(
    sink: &(dyn Fn(bool) + Send + Sync),
    read: Result<bool, AccessibilityUnavailable>,
    setting: &'static str,
) {
    match read {
        Ok(reduce) => {
            let delivered =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(reduce)));
            if delivered.is_err() {
                tracing::error!(target: "riseonly", setting, reduce, "accessibility change handler panicked");
            }
        }
        Err(reason) => {
            tracing::warn!(target: "riseonly", setting, %reason, "accessibility change unreadable");
        }
    }
}

fn refused(call: &str, error: ::windows::core::Error) -> AccessibilityUnavailable {
    AccessibilityUnavailable::SourceRefused(format!("{call}: {error}"))
}

/// which settings event a registration removes
#[derive(Clone, Copy)]
enum Event {
    AdvancedEffects,
    Animations,
}

/// keeps the settings instance the handler is registered on
/// drop removes the handler, which also frees its own reference and its sink
struct Registration {
    settings: UISettings,
    token: i64,
    event: Event,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let (removed, setting) = match self.event {
            Event::AdvancedEffects => (
                self.settings.RemoveAdvancedEffectsEnabledChanged(self.token),
                "reduce transparency",
            ),
            Event::Animations => (
                self.settings.RemoveAnimationsEnabledChanged(self.token),
                "reduce motion",
            ),
        };
        if let Err(error) = removed {
            tracing::warn!(target: "riseonly", setting, %error, "accessibility observer not removed");
        }
    }
}
