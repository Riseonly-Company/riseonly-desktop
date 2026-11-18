use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{TestAppContext, TouchPhase};
use rise_platform::HostOs;

use crate::app::dev_drive::{
    COMMAND_DONE, COMMAND_REQUEST, COORDINATES, DevDrive, DriveButton, DriveCommand, DriveNetState, DriveNotificationOp, DriveReply,
    DriveWindowAction, HoldWindowPlan, tester_response, swipe_events,
    ENABLED, MUTE_ARGUMENT, drag_path, drag_points, SNAPSHOT_DONE, SNAPSHOT_REQUEST, parse_command, parse_keys, parse_link,
    position_in_viewport, resize_size_valid, share_arguments, snapshot_path, snapshot_stem, snapshot_windows, stealth_requested, take_command,
    take_request, target_window, typed_keystroke, window_infos, write_reply, write_report,
};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rise-dev-drive-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn only_the_exact_mute_argument_silences_sound() {
    assert!(DevDrive::parse([MUTE_ARGUMENT.to_owned()], None).sound_muted());
    assert!(DevDrive::parse(["--other".to_owned(), MUTE_ARGUMENT.to_owned()], None).sound_muted());
    assert!(!DevDrive::parse([format!("{MUTE_ARGUMENT}ish")], None).sound_muted());
    assert!(!DevDrive::parse(Vec::new(), None).sound_muted());
}

#[test]
fn only_the_exact_stealth_value_hides_the_app() {
    assert!(stealth_requested(Some(std::ffi::OsStr::new("1"))));
    assert!(!stealth_requested(Some(std::ffi::OsStr::new("0"))));
    assert!(!stealth_requested(Some(std::ffi::OsStr::new(""))));
    assert!(!stealth_requested(Some(std::ffi::OsStr::new("true"))));
    assert!(!stealth_requested(None));
}

#[test]
fn the_stealth_mode_also_mutes_the_sound() {
    let stealthy = DevDrive::parse(Vec::new(), None).with_stealth(true);
    assert!(stealthy.stealth());
    assert!(stealthy.sound_muted());

    let plain = DevDrive::parse(Vec::new(), None).with_stealth(false);
    assert!(!plain.stealth());
    assert!(!plain.sound_muted());
    assert!(DevDrive::parse([MUTE_ARGUMENT.to_owned()], None).with_stealth(false).sound_muted());
}

#[test]
fn an_empty_drive_directory_means_no_snapshot_service() {
    assert_eq!(DevDrive::parse(Vec::new(), Some(OsString::new())).drive_dir(), None);
    assert_eq!(
        DevDrive::parse(Vec::new(), Some(OsString::from("/tmp/drive"))).drive_dir(),
        Some(Path::new("/tmp/drive"))
    );
}

#[test]
fn a_build_without_the_feature_exposes_nothing() {
    if !ENABLED {
        assert_eq!(DevDrive::from_launch(), DevDrive::default());
    }
}

#[test]
fn a_request_name_cannot_leave_the_drive_directory() {
    assert_eq!(snapshot_stem("home\n"), "home");
    assert_eq!(snapshot_stem("../../etc/passwd"), "etcpasswd");
    assert_eq!(snapshot_stem("chat_list-2"), "chat_list-2");
    assert_eq!(snapshot_stem(""), "snapshot");
    assert_eq!(snapshot_stem("/ \\ ."), "snapshot");
    let dir = Path::new("/drive");
    assert_eq!(snapshot_path(dir, "home", 1), dir.join("home-1.png"));
}

#[test]
fn a_request_is_taken_once() {
    let scratch = ScratchDir::new();
    assert_eq!(take_request(scratch.path()), None);

    fs::write(scratch.path().join(SNAPSHOT_REQUEST), "feed").expect("request");
    assert_eq!(take_request(scratch.path()), Some("feed".to_owned()));
    assert!(!scratch.path().join(SNAPSHOT_REQUEST).exists());
    assert_eq!(take_request(scratch.path()), None);
}

#[test]
fn a_request_that_is_not_utf8_is_still_answered() {
    let scratch = ScratchDir::new();
    fs::write(scratch.path().join(SNAPSHOT_REQUEST), [0xff, b'a', 0xfe]).expect("request");
    assert_eq!(take_request(scratch.path()), Some("a".to_owned()));
}

#[test]
fn the_report_lands_whole_with_one_line_per_window() {
    let scratch = ScratchDir::new();
    let first = scratch.path().join("home-0.png");
    write_report(scratch.path(), &[Ok(first.clone()), Err("no renderer".to_owned())])
        .expect("report");

    let report = fs::read_to_string(scratch.path().join(SNAPSHOT_DONE)).expect("done");
    assert_eq!(report, format!("ok {}\nerror no renderer\n", first.display()));
    let leftovers: Vec<_> = fs::read_dir(scratch.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(leftovers, vec![OsString::from(SNAPSHOT_DONE)]);
}

#[gpui::test]
fn sound_is_on_until_a_muted_drive_is_installed(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(!DevDrive::is_sound_muted(cx));
        DevDrive::parse([MUTE_ARGUMENT.to_owned()], None).install(cx);
        assert!(DevDrive::is_sound_muted(cx));
    });
}

#[gpui::test]
fn a_window_that_cannot_render_reports_an_error_and_writes_no_file(cx: &mut TestAppContext) {
    let scratch = ScratchDir::new();
    cx.add_empty_window();
    let results = cx.update(|cx| snapshot_windows(scratch.path(), "home", cx));

    assert_eq!(results.len(), 1);
    assert!(results[0].is_err(), "the test platform has no headless renderer");
    assert!(!snapshot_path(scratch.path(), "home", 0).exists());
}

#[test]
fn the_friendly_mute_flag_becomes_the_launch_argument() {
    if HostOs::current() == HostOs::Windows {
        return;
    }
    let script = Path::new(env!("RISE_WORKSPACE_DIR")).join("scripts/dev-args.sh");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "set -- --mute; . \"{}\"; for a in \"$@\"; do printf '%s\\n' \"$a\"; done",
            script.display()
        ))
        .output()
        .expect("sh runs");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{MUTE_ARGUMENT}\n"));
}

fn command(body: &str) -> DriveCommand {
    parse_command(body.as_bytes()).expect("command parses").command
}

#[test]
fn every_command_parses_from_its_json() {
    assert_eq!(
        command(r#"{"cmd":"click","x":10,"y":20.5}"#),
        DriveCommand::Click { x: 10.0, y: 20.5, button: DriveButton::Left, count: 1, hold_ms: 0, snap_at_ms: None, snap_stem: None, minimise_at_ms: None, restore_at_ms: None }
    );
    assert_eq!(
        command(r#"{"cmd":"click","x":1,"y":2,"button":"right","count":2}"#),
        DriveCommand::Click { x: 1.0, y: 2.0, button: DriveButton::Right, count: 2, hold_ms: 0, snap_at_ms: None, snap_stem: None, minimise_at_ms: None, restore_at_ms: None }
    );
    assert_eq!(
        command(r#"{"cmd":"click","x":1,"y":2,"hold_ms":1500}"#),
        DriveCommand::Click { x: 1.0, y: 2.0, button: DriveButton::Left, count: 1, hold_ms: 1500, snap_at_ms: None, snap_stem: None, minimise_at_ms: None, restore_at_ms: None }
    );
    assert_eq!(command(r#"{"cmd":"move","x":1,"y":2}"#), DriveCommand::Move { x: 1.0, y: 2.0 });
    assert_eq!(
        command(r#"{"cmd":"scroll","x":1,"y":2,"dy":-300}"#),
        DriveCommand::Scroll { x: 1.0, y: 2.0, dy: -300.0, dx: 0.0 }
    );
    assert_eq!(
        command(r#"{"cmd":"scroll","x":1,"y":2,"dy":0,"dx":-240}"#),
        DriveCommand::Scroll { x: 1.0, y: 2.0, dy: 0.0, dx: -240.0 }
    );
    assert_eq!(
        command(r#"{"cmd":"type","text":"hi"}"#),
        DriveCommand::Type { text: "hi".to_owned() }
    );
    assert_eq!(
        command(r#"{"cmd":"key","keys":"cmd-shift-d"}"#),
        DriveCommand::Key { keys: "cmd-shift-d".to_owned() }
    );
    assert_eq!(
        command(r#"{"cmd":"snapshot","stem":"feed"}"#),
        DriveCommand::Snapshot { stem: "feed".to_owned() }
    );
    assert_eq!(command(r#"{"cmd":"info"}"#), DriveCommand::Info);
    assert_eq!(
        command(r#"{"cmd":"open_url","url":"riseonly://chat/42"}"#),
        DriveCommand::OpenUrl { url: "riseonly://chat/42".to_owned() }
    );
    assert_eq!(
        command(r#"{"cmd":"files","paths":["/tmp/a.png","/tmp/b.mp4"]}"#),
        DriveCommand::Files { paths: vec!["/tmp/a.png".to_owned(), "/tmp/b.mp4".to_owned()] }
    );
    assert_eq!(command(r#"{"cmd":"files"}"#), DriveCommand::Files { paths: Vec::new() });
    assert_eq!(
        command(r#"{"cmd":"share","paths":["/tmp/a.png","file:///tmp/b.mp4"]}"#),
        DriveCommand::Share { paths: vec!["/tmp/a.png".to_owned(), "file:///tmp/b.mp4".to_owned()] }
    );
    assert_eq!(
        command(r#"{"cmd":"share_text","text":"hello"}"#),
        DriveCommand::ShareText { text: "hello".to_owned() }
    );
    assert_eq!(
        command(r#"{"cmd":"drag","x1":10,"y1":20,"x2":10,"y2":120}"#),
        DriveCommand::Drag { x1: 10.0, y1: 20.0, x2: 10.0, y2: 120.0, steps: 12, hold_ms: 0, then: Vec::new(), minimise_at_ms: None, restore_at_ms: None, step_ms: 0, end_hold_ms: 0, snap_every: 0, snap_stem: None }
    );
    assert_eq!(
        command(r#"{"cmd":"drag","x1":1,"y1":2,"x2":3,"y2":4,"steps":3,"hold_ms":600}"#),
        DriveCommand::Drag { x1: 1.0, y1: 2.0, x2: 3.0, y2: 4.0, steps: 3, hold_ms: 600, then: Vec::new(), minimise_at_ms: None, restore_at_ms: None, step_ms: 0, end_hold_ms: 0, snap_every: 0, snap_stem: None }
    );
    assert_eq!(
        command(r#"{"cmd":"drag","x1":1,"y1":2,"x2":3,"y2":4,"then":[[5,6],[1,2]]}"#),
        DriveCommand::Drag { x1: 1.0, y1: 2.0, x2: 3.0, y2: 4.0, steps: 12, hold_ms: 0, then: vec![(5.0, 6.0), (1.0, 2.0)], minimise_at_ms: None, restore_at_ms: None, step_ms: 0, end_hold_ms: 0, snap_every: 0, snap_stem: None }
    );
    assert_eq!(
        command(r#"{"cmd":"location","latitude":41.5,"longitude":69.25}"#),
        DriveCommand::Location { latitude: Some(41.5), longitude: Some(69.25) }
    );
    assert_eq!(command(r#"{"cmd":"location"}"#), DriveCommand::Location { latitude: None, longitude: None });
}

#[test]
fn a_polyline_drag_passes_every_waypoint_in_order() {
    assert_eq!(
        drag_path((0.0, 0.0), &[(0.0, 20.0), (20.0, 20.0), (0.0, 0.0)], 2),
        vec![(0.0, 10.0), (0.0, 20.0), (10.0, 20.0), (20.0, 20.0), (10.0, 10.0), (0.0, 0.0)]
    );
    assert!(drag_path((1.0, 1.0), &[], 4).is_empty());
}

#[test]
fn a_drag_walks_evenly_and_ends_exactly_at_the_target() {
    assert_eq!(drag_points((0.0, 0.0), (0.0, 30.0), 3), vec![(0.0, 10.0), (0.0, 20.0), (0.0, 30.0)]);
    assert_eq!(drag_points((5.0, 5.0), (9.0, 1.0), 0), vec![(9.0, 1.0)]);
}

#[test]
fn the_envelope_carries_id_and_window_beside_the_command() {
    let request = parse_command(br#"{"id":"a1","window":2,"cmd":"info"}"#).expect("parses");
    assert_eq!(request.id.as_deref(), Some("a1"));
    assert_eq!(request.window, Some(2));
    assert_eq!(request.command, DriveCommand::Info);

    let bare = parse_command(br#"{"cmd":"info"}"#).expect("parses");
    assert_eq!((bare.id, bare.window), (None, None));
}

#[test]
fn a_bad_command_is_answered_with_its_id() {
    let (id, error) = parse_command(br#"{"id":"a2","cmd":"explode"}"#).expect_err("unknown command");
    assert_eq!(id.as_deref(), Some("a2"));
    assert!(error.starts_with("bad command"));

    assert!(parse_command(br#"{"cmd":"click","x":1}"#).is_err(), "y is required");
    assert!(parse_command(br#"{"cmd":"click","x":1,"y":2,"button":"thumb"}"#).is_err());
    let (no_id, _) = parse_command(&[0xff, 0xfe]).expect_err("not json");
    assert_eq!(no_id, None);
}

#[test]
fn a_character_types_as_a_key_with_its_text() {
    let lower = typed_keystroke('a');
    assert_eq!((lower.key.as_str(), lower.key_char.as_deref()), ("a", Some("a")));
    assert!(!lower.modifiers.shift);

    let upper = typed_keystroke('A');
    assert_eq!((upper.key.as_str(), upper.key_char.as_deref()), ("a", Some("A")));
    assert!(upper.modifiers.shift);

    let cyrillic = typed_keystroke('\u{42f}');
    assert_eq!(cyrillic.key, "\u{44f}");
    assert_eq!(cyrillic.key_char.as_deref(), Some("\u{42f}"));
    assert!(cyrillic.modifiers.shift);

    let dash = typed_keystroke('-');
    assert_eq!((dash.key.as_str(), dash.key_char.as_deref()), ("-", Some("-")));

    let named = [(' ', "space", " "), ('\n', "enter", "\n"), ('\t', "tab", "\t")];
    for (character, key, text) in named {
        let keystroke = typed_keystroke(character);
        assert_eq!((keystroke.key.as_str(), keystroke.key_char.as_deref()), (key, Some(text)));
    }
}

#[test]
fn keys_parse_with_modifiers_and_in_sequence() {
    let chord = parse_keys("cmd-shift-d").expect("chord");
    assert_eq!(chord.len(), 1);
    assert_eq!(chord[0].key, "d");
    assert!(chord[0].modifiers.platform && chord[0].modifiers.shift);

    let sequence = parse_keys("tab  enter").expect("sequence");
    let keys: Vec<&str> = sequence.iter().map(|keystroke| keystroke.key.as_str()).collect();
    assert_eq!(keys, ["tab", "enter"]);

    assert!(parse_keys("  ").is_err());
    assert!(parse_keys("foo-bar").is_err());
}

#[test]
fn a_point_counts_only_inside_the_window() {
    assert!(position_in_viewport(0.0, 0.0, 100.0, 50.0));
    assert!(position_in_viewport(99.5, 49.5, 100.0, 50.0));
    assert!(!position_in_viewport(100.0, 10.0, 100.0, 50.0));
    assert!(!position_in_viewport(10.0, 50.0, 100.0, 50.0));
    assert!(!position_in_viewport(-1.0, 10.0, 100.0, 50.0));
}

#[test]
fn only_a_word_with_a_scheme_is_a_link() {
    assert!(parse_link("riseonly://chat/42").is_ok());
    assert!(parse_link("chat/42").is_err());
    assert!(parse_link("").is_err());
}

#[test]
fn a_command_is_taken_once() {
    let scratch = ScratchDir::new();
    assert_eq!(take_command(scratch.path()), None);

    fs::write(scratch.path().join(COMMAND_REQUEST), br#"{"cmd":"info"}"#).expect("request");
    assert_eq!(take_command(scratch.path()), Some(br#"{"cmd":"info"}"#.to_vec()));
    assert!(!scratch.path().join(COMMAND_REQUEST).exists());
    assert_eq!(take_command(scratch.path()), None);
}

#[test]
fn the_answer_lands_whole_as_json() {
    let scratch = ScratchDir::new();
    let reply = DriveReply { id: Some("a3".to_owned()), ok: true, ..DriveReply::default() };
    write_reply(scratch.path(), &reply).expect("answer");

    let done = fs::read(scratch.path().join(COMMAND_DONE)).expect("done");
    let value: serde_json::Value = serde_json::from_slice(&done).expect("json");
    assert_eq!(value, serde_json::json!({"id": "a3", "ok": true}));
    let leftovers: Vec<_> = fs::read_dir(scratch.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(leftovers, vec![OsString::from(COMMAND_DONE)]);
}

#[gpui::test]
fn a_window_is_chosen_by_index_or_else_the_first_that_answers(cx: &mut TestAppContext) {
    cx.update(|cx| assert!(target_window(None, cx).is_err(), "no window is open"));

    cx.add_empty_window();
    cx.update(|cx| {
        assert_eq!(target_window(None, cx).map(|(index, _)| index), Ok(0));
        assert_eq!(target_window(Some(0), cx).map(|(index, _)| index), Ok(0));
        assert!(target_window(Some(1), cx).is_err());
    });
}

#[gpui::test]
fn info_reports_each_window_in_logical_pixels(cx: &mut TestAppContext) {
    cx.add_empty_window();
    let infos = cx.update(window_infos);

    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].index, 0);
    assert!(infos[0].width > 0.0 && infos[0].height > 0.0);
    assert!(infos[0].scale > 0.0);
    assert!(COORDINATES.contains("scale"));
}

#[test]
fn the_network_switch_parses_both_states() {
    assert_eq!(
        command(r#"{"cmd":"net","state":"off"}"#),
        DriveCommand::Net { state: DriveNetState::Off }
    );
    assert_eq!(
        command(r#"{"cmd":"net","state":"on"}"#),
        DriveCommand::Net { state: DriveNetState::On }
    );
    assert!(parse_command(br#"{"cmd":"net","state":"flaky"}"#).is_err());
    assert!(parse_command(br#"{"cmd":"net"}"#).is_err(), "state is required");
}

#[test]
fn a_refusal_carries_its_code_and_details() {
    assert_eq!(
        command(r#"{"cmd":"refusal","code":"RTC_OVERLOADED","details":{"retry_after_s":"10"}}"#),
        DriveCommand::Refusal {
            code: "RTC_OVERLOADED".to_owned(),
            details: [("retry_after_s".to_owned(), "10".to_owned())].into_iter().collect(),
        }
    );
    assert_eq!(
        command(r#"{"cmd":"refusal","code":"RTC_OVERLOADED"}"#),
        DriveCommand::Refusal { code: "RTC_OVERLOADED".to_owned(), details: Default::default() }
    );
    assert!(parse_command(br#"{"cmd":"refusal"}"#).is_err(), "code is required");
}

#[test]
fn swipe_and_held_window_switches_parse() {
    assert_eq!(
        command(r#"{"cmd":"swipe","x":1,"y":2,"dx":300}"#),
        DriveCommand::Swipe { x: 1.0, y: 2.0, dx: 300.0, dy: 0.0, steps: 12, ms: 180, momentum: false, lines: false }
    );
    assert_eq!(
        command(r#"{"cmd":"swipe","x":1,"y":2,"dy":-80,"steps":4,"ms":400,"momentum":true}"#),
        DriveCommand::Swipe { x: 1.0, y: 2.0, dx: 0.0, dy: -80.0, steps: 4, ms: 400, momentum: true, lines: false }
    );
    assert_eq!(
        command(r#"{"cmd":"drag","x1":1,"y1":2,"x2":3,"y2":4,"minimise_at_ms":300,"restore_at_ms":900}"#),
        DriveCommand::Drag {
            x1: 1.0, y1: 2.0, x2: 3.0, y2: 4.0, steps: 12, hold_ms: 0, then: Vec::new(),
            minimise_at_ms: Some(300), restore_at_ms: Some(900), step_ms: 0, end_hold_ms: 0, snap_every: 0,
            snap_stem: None,
        }
    );
}

#[test]
fn a_swipe_is_one_phased_gesture_with_an_optional_momentum_tail() {
    let events = swipe_events(100.0, 0.0, 4, 200, false, false);
    let phases: Vec<TouchPhase> = events.iter().map(|event| event.phase).collect();
    assert_eq!(
        phases,
        vec![TouchPhase::Started, TouchPhase::Moved, TouchPhase::Moved, TouchPhase::Moved, TouchPhase::Moved, TouchPhase::Ended]
    );
    let total: f32 = events.iter().map(|event| event.delta.0).sum();
    assert!((total - 100.0).abs() < 1e-3);
    assert_eq!(events[1].gap, std::time::Duration::from_millis(50));

    let with_tail = swipe_events(100.0, 0.0, 4, 200, true, false);
    let tail = &with_tail[events.len()..];
    assert!(!tail.is_empty() && tail.iter().all(|event| event.phase == TouchPhase::Moved));
    assert!(tail.windows(2).all(|pair| pair[1].delta.0 < pair[0].delta.0), "momentum decays");

    let wheel = swipe_events(0.0, -3.0, 3, 0, true, true);
    assert_eq!(wheel.len(), 3, "a notched wheel has no phases and no momentum");
    assert!(wheel.iter().all(|event| event.phase == TouchPhase::Moved));
}

#[test]
fn the_held_window_plan_switches_once_and_restores_after_the_minimise() {
    let ms = std::time::Duration::from_millis;
    let mut plan = HoldWindowPlan::from_ms(Some(300), Some(900));
    assert!(plan.take_due(ms(100)).is_empty());
    assert_eq!(plan.take_due(ms(400)), vec![DriveWindowAction::Minimise]);
    assert!(plan.take_due(ms(500)).is_empty());
    assert_eq!(plan.take_due(ms(950)), vec![DriveWindowAction::Restore]);
    assert!(plan.take_due(std::time::Duration::MAX).is_empty());

    let mut late = HoldWindowPlan::from_ms(Some(2000), Some(100));
    assert_eq!(late.take_due(ms(500)), Vec::new(), "no restore before the minimise");
    assert_eq!(late.take_due(std::time::Duration::MAX), vec![DriveWindowAction::Minimise, DriveWindowAction::Restore]);
}

#[test]
fn the_window_command_parses_minimise_and_restore() {
    assert_eq!(
        command(r#"{"cmd":"window","action":"minimise"}"#),
        DriveCommand::Window { action: DriveWindowAction::Minimise }
    );
    assert_eq!(
        command(r#"{"cmd":"window","action":"restore"}"#),
        DriveCommand::Window { action: DriveWindowAction::Restore }
    );
    assert!(parse_command(br#"{"cmd":"window","action":"zoom"}"#).is_err());
    assert!(parse_command(br#"{"cmd":"window"}"#).is_err(), "action is required");
}

#[test]
fn the_resize_command_parses_and_checks_its_size() {
    assert_eq!(
        command(r#"{"cmd":"resize_window","width":1440,"height":900}"#),
        DriveCommand::ResizeWindow { width: 1440.0, height: 900.0 }
    );
    assert!(parse_command(br#"{"cmd":"resize_window","width":1440}"#).is_err(), "height is required");
    assert!(resize_size_valid(2560.0, 1440.0));
    assert!(!resize_size_valid(0.0, 900.0));
    assert!(!resize_size_valid(1440.0, f32::NAN));
    assert!(!resize_size_valid(20000.0, 900.0));
}

#[test]
fn share_takes_absolute_paths_and_file_urls_only() {
    let paths = vec!["/tmp/a.png".to_owned(), "file:///tmp/b.mp4".to_owned()];
    let split = share_arguments(&paths).expect("files");
    assert_eq!(split.files, vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.mp4")]);
    assert!(split.links.is_empty() && split.texts.is_empty());

    assert!(share_arguments(&["riseonly://chat/42".to_owned()]).is_err());
    assert!(share_arguments(&["a.png".to_owned()]).is_err());
    assert!(share_arguments(&[]).is_err());
}

#[test]
fn notifications_command_parses_and_names_its_response() {
    assert_eq!(
        command(r#"{"cmd":"notifications"}"#),
        DriveCommand::Notifications { op: DriveNotificationOp::List, id: None, button: None, text: None }
    );
    assert_eq!(
        command(r#"{"cmd":"notifications","op":"action","notification":3,"button":"MARK_READ_ACTION"}"#),
        DriveCommand::Notifications {
            op: DriveNotificationOp::Action,
            id: Some(3),
            button: Some("MARK_READ_ACTION".to_owned()),
            text: None,
        }
    );
    let request = parse_command(br#"{"id":"req-1","cmd":"notifications","op":"click","notification":5}"#)
        .expect("request parses");
    assert_eq!(request.id.as_deref(), Some("req-1"));
    assert_eq!(
        request.command,
        DriveCommand::Notifications { op: DriveNotificationOp::Click, id: Some(5), button: None, text: None }
    );
    assert_eq!(tester_response(DriveNotificationOp::List, None, None), Ok(None));
    assert_eq!(
        tester_response(DriveNotificationOp::Click, None, None),
        Ok(Some(rise_platform::notifications::TesterResponse::Click))
    );
    assert_eq!(
        tester_response(DriveNotificationOp::Reply, None, Some("hi".to_owned())),
        Ok(Some(rise_platform::notifications::TesterResponse::Reply("hi".to_owned())))
    );
    assert!(tester_response(DriveNotificationOp::Action, Some(String::new()), None).is_err());
    assert!(tester_response(DriveNotificationOp::Reply, None, None).is_err());
}
