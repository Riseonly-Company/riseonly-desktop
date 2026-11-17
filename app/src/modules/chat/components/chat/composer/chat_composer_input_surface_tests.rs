use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    Context, IntoElement, ParentElement, Render, TestAppContext, VisualTestContext, Window, WindowHandle, bounds, div, point,
    size,
};

use crate::modules::chat::components::chat::composer::chat_composer_input_surface::{
    ChatComposerBottomSafeAreaPolicy, ChatComposerInputSurface,
};
use crate::modules::chat::components::chat::composer::chat_inline_composer_panel::ChatInlineComposerPresentationContract;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::{
    ChatComposerBottomPaddingPolicy, ChatKeyboardFrameReader, ChatKeyboardGeometryPolicy, ChatKeyboardLayoutState,
    ChatPickerContentSafeAreaPolicy, ChatPickerPanelHeightPolicy,
};

const FEED_SPACING: f32 = 10.0;

fn bits(values: &[f32]) -> HashSet<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

#[test]
fn system_keyboard_uses_native_safe_area_instead_of_manual_padding() {
    assert_eq!(ChatComposerBottomPaddingPolicy::resolve(34.0, 300.0, false, 336.0, false, 8.0), 34.0);
}

#[test]
fn closed_keyboard_falls_back_to_container_safe_area() {
    assert_eq!(ChatComposerBottomPaddingPolicy::resolve(34.0, 0.0, false, 336.0, false, 8.0), 34.0);
}

#[test]
fn system_keyboard_uses_emoji_icon() {
    assert_eq!(ChatComposerInputSurface::SystemKeyboard.toggle_icon_system_name(), "face.smiling");
}

#[test]
fn picker_uses_keyboard_icon() {
    assert_eq!(ChatComposerInputSurface::Picker.toggle_icon_system_name(), "keyboard");
}

#[test]
fn returning_to_system_keyboard_immediately_restores_emoji_icon() {
    let surface = ChatComposerInputSurface::Picker.toggled();

    assert_eq!(surface.toggle_icon_system_name(), "face.smiling");
}

#[test]
fn repeated_toggle_round_trips_without_sticky_icon_state() {
    let mut surface = ChatComposerInputSurface::SystemKeyboard;

    for _ in 0..20 {
        surface = surface.toggled();
        assert_eq!(surface, ChatComposerInputSurface::Picker);
        assert_eq!(surface.toggle_icon_system_name(), "keyboard");
        surface = surface.toggled();
        assert_eq!(surface, ChatComposerInputSurface::SystemKeyboard);
        assert_eq!(surface.toggle_icon_system_name(), "face.smiling");
    }
}

#[test]
fn docked_keyboard_overlap_is_independent_of_swift_ui_layout_bounds() {
    let window = bounds(point(0.0_f32, 0.0), size(390.0, 844.0));
    let keyboard = bounds(point(0.0_f32, 510.0), size(390.0, 334.0));

    assert_eq!(ChatKeyboardFrameReader::docked_keyboard_overlap(keyboard, window), 334.0);
}

#[test]
fn floating_or_hidden_keyboard_does_not_move_the_composer() {
    let window = bounds(point(0.0_f32, 0.0), size(390.0, 844.0));

    assert_eq!(
        ChatKeyboardFrameReader::docked_keyboard_overlap(bounds(point(80.0, 300.0), size(230.0, 200.0)), window),
        0.0
    );
    assert_eq!(
        ChatKeyboardFrameReader::docked_keyboard_overlap(bounds(point(0.0, 844.0), size(390.0, 334.0)), window),
        0.0
    );
    assert_eq!(ChatKeyboardFrameReader::docked_keyboard_overlap(window, window), 0.0);
}

#[test]
fn un_laid_out_guide_cannot_become_a_full_screen_keyboard() {
    let viewport_height = 932.0;
    let safe_top = 62.0;
    let safe_bottom = 34.0;

    assert_eq!(ChatKeyboardGeometryPolicy::normalized_tracker_overlap(0.0, viewport_height, safe_top, safe_bottom), None);
    assert_eq!(
        ChatKeyboardGeometryPolicy::normalized_tracker_overlap(safe_top, viewport_height, safe_top, safe_bottom),
        None
    );
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(870.0), 0.0, 0.0, 0.0), None);
    assert_eq!(ChatKeyboardGeometryPolicy::normalized_target_height(800.0, viewport_height, safe_top), None);
}

#[test]
fn confirmed_keyboard_envelope_allows_interactive_frames_only() {
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(180.0), 334.0, 334.0, 120.0), Some(180.0));
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(180.0), 0.0, 334.0, 120.0), Some(180.0));
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(250.0), 180.0, 334.0, 180.0), Some(250.0));
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(0.0), 0.0, 334.0, 120.0), Some(0.0));
    assert_eq!(ChatKeyboardGeometryPolicy::accepted_guide_height(Some(870.0), 0.0, 334.0, 0.0), None);
}

#[test]
fn presentation_sampling_stops_as_soon_as_either_target_settles() {
    assert!(ChatKeyboardGeometryPolicy::should_stop_sampling(3, 334.0, 334.0, 334.0, true));
    assert!(ChatKeyboardGeometryPolicy::should_stop_sampling(3, 0.0, 0.0, 0.0, true));
    assert!(!ChatKeyboardGeometryPolicy::should_stop_sampling(2, 334.0, 334.0, 334.0, true));
}

struct KeyboardReaderHost {
    shown: bool,
    sampled_states: Rc<RefCell<Vec<ChatKeyboardLayoutState>>>,
}

impl Render for KeyboardReaderHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let sampled_states = self.sampled_states.clone();
        let mut root = div();
        if self.shown {
            root = root.child(
                ChatKeyboardFrameReader::new("keyboard_reader", move |state, _, _| sampled_states.borrow_mut().push(state))
                    .reports_presentation_frames(true),
            );
        }
        root
    }
}

type SampledStates = Rc<RefCell<Vec<ChatKeyboardLayoutState>>>;

fn open_reader(cx: &mut TestAppContext, shown: bool) -> (WindowHandle<KeyboardReaderHost>, SampledStates) {
    let sampled_states: SampledStates = Rc::default();
    let host_states = sampled_states.clone();
    let window = cx.add_window(move |_, _| KeyboardReaderHost {
        shown,
        sampled_states: host_states,
    });
    VisualTestContext::from_window(window.into(), cx).run_until_parked();
    (window, sampled_states)
}

fn set_shown(cx: &mut TestAppContext, window: WindowHandle<KeyboardReaderHost>, shown: bool) {
    let updated = window.update(cx, |host, _, cx| {
        host.shown = shown;
        cx.notify();
    });
    assert!(updated.is_ok(), "the test window is open");
    cx.run_until_parked();
}

fn assert_hidden_and_settled(state: Option<ChatKeyboardLayoutState>) {
    assert_eq!(state.map(|state| state.height), Some(0.0));
    assert_eq!(state.map(|state| state.target_height), Some(0.0));
    assert_eq!(state.map(|state| state.is_settled), Some(true));
}

#[gpui::test]
fn keyboard_probe_uses_a_full_window_tracking_host(cx: &mut TestAppContext) {
    let (_window, sampled_states) = open_reader(cx, true);

    assert_hidden_and_settled(sampled_states.borrow().last().copied());
}

#[gpui::test]
fn keyboard_notification_survives_probe_attachment_race(cx: &mut TestAppContext) {
    let (window, sampled_states) = open_reader(cx, false);
    assert!(sampled_states.borrow().is_empty());

    set_shown(cx, window, true);

    assert_hidden_and_settled(sampled_states.borrow().last().copied());
}

#[gpui::test]
fn temporary_probe_detach_preserves_the_open_keyboard_session(cx: &mut TestAppContext) {
    let (window, sampled_states) = open_reader(cx, true);
    set_shown(cx, window, false);
    set_shown(cx, window, true);

    let states = sampled_states.borrow();
    assert!(states.len() >= 2, "a reader drawn again reports again");
    assert!(states.iter().all(|state| state.target_height == 0.0 && state.is_settled));
}

#[test]
fn system_keyboard_frames_do_not_rebuild_composer_or_feed_insets() {
    let safe_bottom = 34.0;
    let row_height = 42.0;
    let top_padding = 5.0;
    let keyboard_frames = [0.0, 0.5, 1.0, 1.1, 5.0, 10.0, 20.0, 29.0, 34.0, 40.0, 120.0, 220.0, 334.0];

    let mut resolved_composer_heights = Vec::new();
    let mut resolved_feed_insets = Vec::new();
    for keyboard_height in keyboard_frames {
        let bottom_padding =
            ChatComposerBottomPaddingPolicy::resolve(safe_bottom, keyboard_height, false, 336.0, false, top_padding);
        let composer_height = row_height + top_padding + bottom_padding;
        let feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(composer_height, FEED_SPACING);
        assert_eq!(bottom_padding, safe_bottom);
        resolved_composer_heights.push(composer_height);
        resolved_feed_insets.push(feed_inset);
    }

    assert_eq!(bits(&resolved_composer_heights).len(), 1);
    assert_eq!(bits(&resolved_feed_insets).len(), 1);
}

#[test]
fn picker_handoff_matches_the_native_keyboard_envelope() {
    let safe_bottom = 34.0;
    let top_padding = 5.0;
    let picker_height = 336.0;
    let keyboard_targets = [0.0, 302.0, 320.0, 336.0, 356.0, 370.0];
    let handoff_padding: Vec<f32> = keyboard_targets
        .iter()
        .map(|keyboard_height| {
            ChatComposerBottomPaddingPolicy::resolve(safe_bottom, *keyboard_height, true, picker_height, true, top_padding)
        })
        .collect();

    let native_equivalent_padding = picker_height + top_padding;
    assert!(handoff_padding.iter().all(|padding| *padding == native_equivalent_padding));
}

#[test]
fn picker_panel_and_composer_share_the_native_keyboard_top_edge() {
    let screen_height = 844.0;
    let safe_bottom = 34.0;
    let panel_height = 336.0;
    let top_padding = 5.0;
    let panel_top = screen_height - panel_height;
    let composer_bottom = screen_height
        - ChatComposerBottomPaddingPolicy::resolve(safe_bottom, panel_height, true, panel_height, true, top_padding);
    let native_keyboard_top = screen_height - panel_height;

    assert_eq!(panel_top, native_keyboard_top);
    assert_eq!(composer_bottom, native_keyboard_top - top_padding);
}

#[test]
fn picker_keeps_the_window_safe_area_while_the_system_keyboard_closes() {
    let window_safe_bottom = 34.0;
    let shell_safe_bottom_frames = [0.0, 0.0, 12.0, 34.0];
    let resolved_safe_bottom_frames: Vec<f32> = shell_safe_bottom_frames
        .iter()
        .map(|shell| ChatPickerContentSafeAreaPolicy::resolved_safe_bottom(*shell, window_safe_bottom))
        .collect();
    let padding_frames: Vec<f32> = shell_safe_bottom_frames
        .iter()
        .map(|shell| ChatComposerBottomPaddingPolicy::resolve(*shell, 336.0, true, 336.0, false, 5.0))
        .collect();

    assert!(resolved_safe_bottom_frames.iter().all(|bottom| *bottom == window_safe_bottom));
    assert_eq!(bits(&padding_frames).len(), 1);
    assert_eq!(padding_frames.first().copied(), Some(341.0));
}

#[test]
fn picker_handoff_keeps_the_captured_panel_height() {
    assert_eq!(ChatPickerPanelHeightPolicy::resolve(370.0, Some(336.0)), 336.0);
    assert_eq!(ChatPickerPanelHeightPolicy::resolve(370.0, None), 370.0);
}

#[test]
fn picker_handoff_freezes_all_bottom_safe_area_regions() {
    assert!(ChatComposerBottomSafeAreaPolicy::ignored_regions(true, false));
    assert!(ChatComposerBottomSafeAreaPolicy::ignored_regions(false, true));
    assert!(!ChatComposerBottomSafeAreaPolicy::ignored_regions(false, false));
}

#[test]
fn closed_keyboard_cannot_create_a_viewport_sized_feed_inset() {
    let viewport_height = 932.0;
    let bottom_padding = ChatComposerBottomPaddingPolicy::resolve(34.0, 0.0, false, 336.0, false, 5.0);
    let composer_total = 42.0 + 5.0 + bottom_padding;
    let feed_inset = ChatInlineComposerPresentationContract::feed_bottom_inset(composer_total, FEED_SPACING) + 10.0;

    assert!(feed_inset < viewport_height);
    assert_eq!(feed_inset, 88.0);
}
