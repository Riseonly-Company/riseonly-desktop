#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, ElementId, SharedString, Window};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind, BottomSheetDebugRegistrationModule,
};
use crate::modules::notify::components::announcement_modal::AnnouncementModal;
use crate::modules::notify::components::release_notes_modal::ReleaseNotesModal;
use crate::modules::notify::engine::core::rise_announcements_wire_models::{RiseAnnouncement, RiseAnnouncementSlide};
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{RiseReleaseNote, RiseReleaseSlide};

pub enum NotificationBottomSheetPreviews {}

impl NotificationBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let releases = [false, true].into_iter().map(|blocking| {
            BottomSheetDebugRegistration::new(
                format!("notifications.release.{blocking}"),
                BottomSheetDebugRegistrationModule::Notifications,
                if blocking { "release_notes_update_hint" } else { "debug_surface_release" },
                "ReleaseNotesModal",
                move |open: Binding<bool>, _: &mut Window, _: &mut App| DebugReleaseNotes { is_presented: open, blocking },
            )
            .with_kind(BottomSheetDebugRegistrationKind::Modal)
        });
        let announcements = [false, true].into_iter().map(|slides| {
            BottomSheetDebugRegistration::new(
                format!("notifications.announcement.{slides}"),
                BottomSheetDebugRegistrationModule::Notifications,
                "notification_settings_announcements",
                "AnnouncementModal",
                move |open: Binding<bool>, _: &mut Window, _: &mut App| DebugAnnouncement {
                    is_presented: open,
                    with_slides: slides,
                },
            )
            .with_variant(Some(if slides { "debug_surface_slides" } else { "editor_segment_text" }.to_owned()))
            .with_kind(BottomSheetDebugRegistrationKind::Modal)
        });
        releases.chain(announcements).collect()
    }
}

/// slide index of a preview keyed so each preview keeps its own page
fn slide_index_binding(key: ElementId, window: &mut Window, cx: &mut App) -> Binding<i64> {
    let state = window.use_keyed_state(key, cx, |_, _| 0_i64);
    Binding::entity(state, |index: &i64| *index, |index, value, _| *index = value)
}

fn release_slide(id: i64, position: i64, title: &str, body: &str) -> RiseReleaseSlide {
    RiseReleaseSlide::new(
        id,
        position,
        String::new(),
        String::new(),
        title.to_owned(),
        body.to_owned(),
        String::new(),
        String::new(),
    )
}

fn announcement_slide(id: i64, position: i64, title: &str, body: &str, action_title: &str) -> RiseAnnouncementSlide {
    RiseAnnouncementSlide::new(
        id,
        position,
        String::new(),
        String::new(),
        title.to_owned(),
        body.to_owned(),
        action_title.to_owned(),
        String::new(),
    )
}

#[derive(IntoElement)]
struct DebugReleaseNotes {
    is_presented: Binding<bool>,
    blocking: bool,
}

impl DebugReleaseNotes {
    fn note(&self) -> RiseReleaseNote {
        RiseReleaseNote::new(
            1,
            "1.0".to_owned(),
            self.blocking,
            "en".to_owned(),
            "What's new".to_owned(),
            0,
            vec![
                release_slide(
                    1,
                    0,
                    "A new way to connect",
                    "Your conversations, music and favourite moments, together in one place.",
                ),
                release_slide(2, 1, "Made for you", "Explore the details and make it your own."),
            ],
        )
    }
}

impl RenderOnce for DebugReleaseNotes {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("debug-release-notes-{}", self.blocking));
        let slide_index = slide_index_binding(ElementId::Name(key), window, cx);
        let is_open = self.is_presented.get(cx);
        let note = self.note();
        let on_dismiss = self.is_presented.clone();
        let on_action = self.is_presented;
        div().size_full().when(is_open, |host| {
            host.child(ReleaseNotesModal::new(
                note,
                slide_index,
                Rc::new(move |window: &mut Window, cx: &mut App| on_dismiss.set(false, window, cx)),
                Rc::new(move |_: &RiseReleaseSlide, window: &mut Window, cx: &mut App| on_action.set(false, window, cx)),
            ))
        })
    }
}

#[derive(IntoElement)]
struct DebugAnnouncement {
    is_presented: Binding<bool>,
    with_slides: bool,
}

impl DebugAnnouncement {
    fn announcement(&self) -> RiseAnnouncement {
        let slides = if self.with_slides {
            vec![
                announcement_slide(1, 0, "Stay connected", "Explore the new details with your friends.", "Explore"),
                announcement_slide(2, 1, "Made for you", "Your conversations and favourite moments, together.", "Done"),
            ]
        } else {
            Vec::new()
        };
        RiseAnnouncement::new(
            1,
            "en".to_owned(),
            "Hello from RISEONLY".to_owned(),
            "A place for your people, stories and music.".to_owned(),
            1_750_000_000_000,
            slides,
        )
    }
}

impl RenderOnce for DebugAnnouncement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("debug-announcement-{}", self.with_slides));
        let slide_index = slide_index_binding(ElementId::Name(key), window, cx);
        let is_open = self.is_presented.get(cx);
        let announcement = self.announcement();
        let on_dismiss = self.is_presented.clone();
        let on_action = self.is_presented;
        div().size_full().when(is_open, |host| {
            host.child(AnnouncementModal::new(
                announcement,
                slide_index,
                Rc::new(move |window: &mut Window, cx: &mut App| on_dismiss.set(false, window, cx)),
                Rc::new(move |_: &RiseAnnouncementSlide, window: &mut Window, cx: &mut App| on_action.set(false, window, cx)),
            ))
        })
    }
}
