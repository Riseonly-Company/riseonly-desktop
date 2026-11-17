use crate::map_view::*;
use rise_platform::HostOs;

const MOSCOW: LatLng = LatLng::new(55.7558, 37.6173);
const NEW_YORK: LatLng = LatLng::new(40.7128, -74.0060);

fn viewport() -> Viewport {
    Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 2.0,
    }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn same_place(a: LatLng, b: LatLng) -> bool {
    close(a.latitude, b.latitude, 1e-6) && close(wrap_longitude(a.longitude - b.longitude), 0.0, 1e-6)
}

fn cameras() -> Vec<Camera> {
    let mut all = Vec::new();
    for bearing in [0.0, 37.0, 90.0, 211.0] {
        for pitch in [0.0, 30.0, 60.0] {
            all.push(Camera {
                center: MOSCOW,
                zoom: 11.5,
                bearing,
                pitch,
            });
        }
    }
    all
}

#[test]
fn mercator_round_trips() {
    for coordinate in [MOSCOW, NEW_YORK, LatLng::new(-33.86, 151.21), LatLng::new(0.0, 0.0)] {
        let (x, y) = mercator(coordinate);
        assert!(same_place(inverse_mercator(x, y), coordinate), "{coordinate:?}");
    }
}

#[test]
fn the_center_draws_at_the_middle_of_the_viewport() {
    for camera in cameras() {
        let point = Projection::new(camera, viewport()).project(MOSCOW).expect("centre on screen");
        assert!(close(point.x, 400.0, 1e-6) && close(point.y, 300.0, 1e-6), "{camera:?} {point:?}");
    }
}

#[test]
fn project_and_unproject_are_inverse_for_every_bearing_and_pitch() {
    let points = [
        ScreenPoint::new(10.0, 590.0),
        ScreenPoint::new(790.0, 250.0),
        ScreenPoint::new(400.0, 120.0),
        ScreenPoint::new(123.0, 456.0),
    ];
    for camera in cameras() {
        let projection = Projection::new(camera, viewport());
        for point in points {
            let coordinate = projection.unproject(point).expect("below the horizon");
            let back = projection.project(coordinate).expect("on screen");
            assert!(close(back.x, point.x, 1e-6) && close(back.y, point.y, 1e-6), "{camera:?} {point:?} {back:?}");
        }
    }
}

#[test]
fn a_flat_north_up_map_has_east_to_the_right_and_north_up() {
    let projection = Projection::new(Camera::new(MOSCOW, 10.0), viewport());
    let east = projection.project(LatLng::new(MOSCOW.latitude, MOSCOW.longitude + 0.1)).expect("east");
    let north = projection.project(LatLng::new(MOSCOW.latitude + 0.1, MOSCOW.longitude)).expect("north");
    assert!(east.x > 400.0 && close(east.y, 300.0, 1e-6));
    assert!(north.y < 300.0 && close(north.x, 400.0, 1e-6));
}

#[test]
fn a_bearing_of_ninety_puts_east_at_the_top() {
    let camera = Camera {
        bearing: 90.0,
        ..Camera::new(MOSCOW, 10.0)
    };
    let east = Projection::new(camera, viewport())
        .project(LatLng::new(MOSCOW.latitude, MOSCOW.longitude + 0.1))
        .expect("east");
    assert!(east.y < 300.0 && close(east.x, 400.0, 1e-6), "{east:?}");
}

#[test]
fn one_zoom_level_doubles_the_world() {
    let near = Camera::new(MOSCOW, 12.0);
    assert!(close(near.world_size(), 2.0 * Camera::new(MOSCOW, 11.0).world_size(), 1e-9));
    assert!(close(Camera::new(MOSCOW, 0.0).world_size(), TILE_SIZE, 1e-9));
}

#[test]
fn the_sky_of_a_pitched_map_has_no_coordinate() {
    let camera = Camera {
        pitch: MAX_PITCH,
        ..Camera::new(MOSCOW, 10.0)
    };
    let projection = Projection::new(camera, viewport());
    assert_eq!(projection.unproject(ScreenPoint::new(400.0, -1000.0)), None);
    assert!(projection.unproject(ScreenPoint::new(400.0, 0.0)).is_some());
}

#[test]
fn the_nearest_world_copy_is_projected_across_the_antimeridian() {
    let camera = Camera::new(LatLng::new(0.0, 179.9), 6.0);
    let projection = Projection::new(camera, viewport());
    let across = projection.project(LatLng::new(0.0, -179.9)).expect("across");
    assert!(across.x > 400.0 && across.x < 800.0, "{across:?}");
}

#[test]
fn clamping_keeps_the_camera_inside_mapview_limits() {
    let camera = Camera {
        center: LatLng::new(89.0, 200.0),
        zoom: 25.0,
        bearing: -30.0,
        pitch: 80.0,
    }
    .clamped();
    assert!(close(camera.center.latitude, MAX_LATITUDE, 1e-9));
    assert!(close(camera.center.longitude, -160.0, 1e-9));
    assert!(close(camera.zoom, MAX_ZOOM, 1e-9));
    assert!(close(camera.bearing, 330.0, 1e-9));
    assert!(close(camera.pitch, MAX_PITCH, 1e-9));
}

#[test]
fn shortest_turn_goes_the_short_way() {
    assert!(close(shortest_turn(350.0, 10.0), 20.0, 1e-9));
    assert!(close(shortest_turn(10.0, 350.0), -20.0, 1e-9));
}

#[test]
fn a_pan_drag_keeps_the_grabbed_place_under_the_pointer() {
    for camera in cameras() {
        let mut controller = MapInputController::default();
        let start = ScreenPoint::new(300.0, 400.0);
        let end = ScreenPoint::new(520.0, 330.0);
        let grabbed = Projection::new(camera, viewport()).unproject(start).expect("grab");
        let started = controller.handle(
            MapInput::DragStart {
                point: start,
                button: DragButton::Primary,
                modifiers: InputModifiers::default(),
            },
            &camera,
            &viewport(),
        );
        assert_eq!(started, None);
        let Some(CameraCommand::Jump { camera: moved, reason }) =
            controller.handle(MapInput::DragMove { point: end }, &camera, &viewport())
        else {
            panic!("a drag pans");
        };
        assert_eq!(reason, CameraChangeReason::GESTURE_PAN);
        let now = Projection::new(moved, viewport()).project(grabbed).expect("on screen");
        assert!(close(now.x, end.x, 1e-6) && close(now.y, end.y, 1e-6), "{camera:?} {now:?}");
    }
}

#[test]
fn a_secondary_or_control_drag_rotates() {
    let camera = Camera::new(MOSCOW, 10.0);
    for (button, control) in [(DragButton::Secondary, false), (DragButton::Primary, true)] {
        let mut controller = MapInputController::default();
        controller.handle(
            MapInput::DragStart {
                point: ScreenPoint::new(100.0, 100.0),
                button,
                modifiers: InputModifiers {
                    control,
                    ..InputModifiers::default()
                },
            },
            &camera,
            &viewport(),
        );
        let Some(CameraCommand::Jump { camera: turned, reason }) = controller.handle(
            MapInput::DragMove {
                point: ScreenPoint::new(150.0, 100.0),
            },
            &camera,
            &viewport(),
        ) else {
            panic!("a rotating drag turns the map");
        };
        assert!(reason.intersects(CameraChangeReason::GESTURE_ROTATE));
        assert!(close(turned.bearing, 50.0 * DRAG_BEARING_PER_POINT, 1e-9));
        assert!(same_place(turned.center, camera.center));
    }
}

#[test]
fn a_drag_ends_on_release() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    controller.handle(
        MapInput::DragStart {
            point: ScreenPoint::new(1.0, 1.0),
            button: DragButton::Primary,
            modifiers: InputModifiers::default(),
        },
        &camera,
        &viewport(),
    );
    assert!(controller.is_dragging());
    controller.handle(MapInput::DragEnd, &camera, &viewport());
    assert!(!controller.is_dragging());
    assert_eq!(
        controller.handle(MapInput::DragMove { point: ScreenPoint::new(9.0, 9.0) }, &camera, &viewport()),
        None
    );
}

#[test]
fn zooming_keeps_the_place_under_the_anchor() {
    for camera in cameras() {
        let projection = Projection::new(camera, viewport());
        let anchor = ScreenPoint::new(620.0, 410.0);
        let place = projection.unproject(anchor).expect("place");
        let zoomed = zoom_around(&projection, 1.5, anchor).expect("zoomed");
        assert!(close(zoomed.zoom, camera.zoom + 1.5, 1e-9));
        let now = Projection::new(zoomed, viewport()).project(place).expect("on screen");
        assert!(close(now.x, anchor.x, 1e-6) && close(now.y, anchor.y, 1e-6), "{camera:?} {now:?}");
    }
}

#[test]
fn a_notched_wheel_zooms_and_counts_as_a_zoom_gesture() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    let Some(CameraCommand::Jump { camera: zoomed, reason }) = controller.handle(
        MapInput::Wheel {
            point: ScreenPoint::new(400.0, 300.0),
            delta: WheelDelta::Lines { x: 0.0, y: 10.0 },
            modifiers: InputModifiers::default(),
        },
        &camera,
        &viewport(),
    ) else {
        panic!("a wheel zooms");
    };
    assert!(close(zoomed.zoom, 10.0 + WHEEL_ZOOM_CAP, 1e-9), "one event is capped");
    assert!(reason.intersects(CameraChangeReason::ZOOM_GESTURES));
}

#[test]
fn a_trackpad_scroll_pans() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    let command = controller.handle(
        MapInput::Wheel {
            point: ScreenPoint::new(400.0, 300.0),
            delta: WheelDelta::Pixels { x: 40.0, y: -25.0 },
            modifiers: InputModifiers::default(),
        },
        &camera,
        &viewport(),
    );
    let Some(CameraCommand::Jump { camera: moved, reason }) = command else {
        panic!("a trackpad scroll pans");
    };
    assert_eq!(reason, CameraChangeReason::GESTURE_PAN);
    assert!(close(moved.zoom, camera.zoom, 1e-12));
    let old_center = Projection::new(moved, viewport()).project(MOSCOW).expect("on screen");
    assert!(close(old_center.x, 440.0, 1e-6) && close(old_center.y, 275.0, 1e-6));
}

#[test]
fn a_pinch_zooms_by_its_scale() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    let Some(CameraCommand::Jump { camera: zoomed, reason }) = controller.handle(
        MapInput::Pinch {
            point: ScreenPoint::new(400.0, 300.0),
            delta: 1.0,
        },
        &camera,
        &viewport(),
    ) else {
        panic!("a pinch zooms");
    };
    assert!(close(zoomed.zoom, 11.0, 1e-9));
    assert_eq!(reason, CameraChangeReason::GESTURE_PINCH);
}

#[test]
fn a_double_click_animates_one_level_in_and_alt_one_out() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    for (alt, zoom, reason) in [
        (false, 11.0, CameraChangeReason::GESTURE_ZOOM_IN),
        (true, 9.0, CameraChangeReason::GESTURE_ZOOM_OUT),
    ] {
        let Some(CameraCommand::Animate { animation, reason: got }) = controller.handle(
            MapInput::DoubleClick {
                point: ScreenPoint::new(200.0, 200.0),
                modifiers: InputModifiers {
                    alt,
                    ..InputModifiers::default()
                },
            },
            &camera,
            &viewport(),
        ) else {
            panic!("a double click animates");
        };
        assert_eq!(got, reason);
        assert!(close(animation.to.zoom, zoom, 1e-9));
        assert!(close(animation.duration_secs, DEFAULT_ANIMATION_SECS, 1e-12));
    }
}

#[test]
fn shortcut_keys_are_left_to_the_app_but_plus_and_minus_zoom() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    let secondary = InputModifiers {
        secondary: true,
        ..InputModifiers::default()
    };
    for key in [MapKey::Up, MapKey::Left] {
        assert_eq!(controller.handle(MapInput::Key { key, modifiers: secondary }, &camera, &viewport()), None);
    }
    for (key, sign) in [(MapKey::ZoomIn, 1.0), (MapKey::ZoomOut, -1.0)] {
        let Some(CameraCommand::Animate { animation, .. }) =
            controller.handle(MapInput::Key { key, modifiers: secondary }, &camera, &viewport())
        else {
            panic!("cmd plus and minus zoom");
        };
        assert!(close(animation.to.zoom, 10.0 + sign, 1e-9));
    }
}

#[test]
fn a_modified_precise_scroll_zooms_instead_of_panning() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    for modifiers in [
        InputModifiers { secondary: true, ..InputModifiers::default() },
        InputModifiers { control: true, ..InputModifiers::default() },
    ] {
        let wheel = MapInput::Wheel {
            point: viewport().center(),
            delta: WheelDelta::Pixels { x: 0.0, y: WHEEL_ZOOM_POINTS_PER_LEVEL / 2.0 },
            modifiers,
        };
        let Some(CameraCommand::Jump { camera: zoomed, .. }) = controller.handle(wheel, &camera, &viewport()) else {
            panic!("a modified scroll zooms");
        };
        assert!(close(zoomed.zoom, 10.5, 1e-9));
    }
}

#[test]
fn arrow_keys_show_more_of_the_map_in_their_direction() {
    let camera = Camera::new(MOSCOW, 10.0);
    let mut controller = MapInputController::default();
    let Some(CameraCommand::Animate { animation, .. }) = controller.handle(
        MapInput::Key {
            key: MapKey::Up,
            modifiers: InputModifiers::default(),
        },
        &camera,
        &viewport(),
    ) else {
        panic!("an arrow pans");
    };
    assert!(animation.to.center.latitude > camera.center.latitude);
    let Some(CameraCommand::Animate { animation, reason }) = controller.handle(
        MapInput::Key {
            key: MapKey::Right,
            modifiers: InputModifiers {
                alt: true,
                ..InputModifiers::default()
            },
        },
        &camera,
        &viewport(),
    ) else {
        panic!("Option+Right turns");
    };
    assert_eq!(reason, CameraChangeReason::GESTURE_ROTATE);
    assert!(close(animation.to.bearing, KEY_ROTATE_DEGREES, 1e-9));
}

#[test]
fn an_ease_starts_and_ends_on_its_cameras() {
    let from = Camera::new(MOSCOW, 10.0);
    let to = Camera {
        bearing: 350.0,
        ..Camera::new(NEW_YORK, 12.0)
    };
    let animation = CameraAnimation::ease(from, to, 0.3);
    let start = animation.sample(0.0);
    assert!(same_place(start.center, from.center) && close(start.zoom, 10.0, 1e-9));
    assert_eq!(animation.sample(0.3), animation.to);
    assert_eq!(animation.sample(5.0), animation.to);
    let middle = animation.sample(0.15);
    assert!(middle.bearing > 300.0 || middle.bearing < 1e-9, "turns the short way: {middle:?}");
}

#[test]
fn a_fly_rises_between_far_places_and_lands_on_target() {
    let from = Camera::new(MOSCOW, 12.0);
    let to = Camera::new(NEW_YORK, 12.0);
    let animation = CameraAnimation::fly(from, to, &viewport());
    assert!(animation.duration_secs > DEFAULT_ANIMATION_SECS);
    let start = animation.sample(0.0);
    assert!(same_place(start.center, from.center) && close(start.zoom, 12.0, 1e-6));
    let middle = animation.sample(animation.duration_secs / 2.0);
    assert!(middle.zoom < 8.0, "the camera rises mid-flight: {middle:?}");
    let near_end = animation.sample(animation.duration_secs * 0.999_999);
    assert!(close(near_end.zoom, 12.0, 1e-3), "{near_end:?}");
    assert!(close(near_end.center.latitude, NEW_YORK.latitude, 1e-3), "{near_end:?}");
    assert_eq!(animation.sample(animation.duration_secs), animation.to);
}

#[test]
fn a_fly_in_place_only_zooms() {
    let from = Camera::new(MOSCOW, 5.0);
    let to = Camera::new(MOSCOW, 9.0);
    let animation = CameraAnimation::fly(from, to, &viewport());
    let middle = animation.sample(animation.duration_secs / 2.0);
    assert!(same_place(middle.center, MOSCOW));
    assert!(middle.zoom > 5.0 && middle.zoom < 9.0, "{middle:?}");
}

#[test]
fn the_ease_curves_are_monotonic_from_zero_to_one() {
    for curve in [UnitBezier::new(0.0, 0.0, 0.25, 1.0), UnitBezier::new(0.25, 0.1, 0.25, 1.0)] {
        assert!(close(curve.solve(0.0), 0.0, 1e-6));
        assert!(close(curve.solve(1.0), 1.0, 1e-6));
        let mut last = 0.0;
        for step in 1..=100 {
            let value = curve.solve(f64::from(step) / 100.0);
            assert!(value + 1e-9 >= last, "step {step}");
            last = value;
        }
    }
}

#[test]
fn the_topmost_annotation_wins_a_click() {
    let projection = Projection::new(Camera::new(MOSCOW, 14.0), viewport());
    let pin = |id| Annotation {
        id,
        coordinate: MOSCOW,
        width: 40.0,
        height: 50.0,
        anchor: (0.5, 1.0),
    };
    let pins = [pin(1), pin(2)];
    assert_eq!(projection.hit_test(&pins, ScreenPoint::new(400.0, 280.0)), Some(2));
    assert_eq!(projection.hit_test(&pins, ScreenPoint::new(400.0, 310.0)), None, "below the tip");
    let frame = projection.annotation_frame(&pins[0]).expect("frame");
    assert!(close(frame.origin.x, 380.0, 1e-6) && close(frame.origin.y, 250.0, 1e-6));
}

#[test]
fn camera_change_reasons_match_mapview_bits() {
    assert_eq!(CameraChangeReason::GESTURE_PAN.0, 1 << 2);
    assert!(CameraChangeReason::ZOOM_GESTURES.intersects(CameraChangeReason::GESTURE_ZOOM_OUT));
    assert!(!CameraChangeReason::ZOOM_GESTURES.intersects(CameraChangeReason::GESTURE_PAN));
    assert!(!CameraChangeReason::PROGRAMMATIC.is_gesture());
    assert!(CameraChangeReason::GESTURE_ROTATE.is_gesture());
}

#[test]
fn a_snapshot_request_is_checked_before_rendering() {
    let request = |width, height, style: &str| SnapshotRequest {
        style_json: style.to_owned(),
        width,
        height,
        scale_factor: 2.0,
        camera: Camera::new(MOSCOW, 2.0),
    };
    assert_eq!(request(1024, 512, "{}").pixel_size(), Ok((2048, 1024)));
    assert_eq!(request(0, 512, "{}").pixel_size(), Err(SnapshotError::EmptySize { width: 0, height: 512 }));
    assert_eq!(request(8, 8, "  ").pixel_size(), Err(SnapshotError::NoStyle));
    assert_eq!(request(5000, 8, "{}").pixel_size(), Err(SnapshotError::TooLarge(10000)));
}

#[test]
fn the_viewport_renders_in_device_pixels() {
    assert_eq!(viewport().pixel_size(), (1600, 1200));
}

#[test]
fn every_host_has_a_gpu_backend_and_an_honest_answer() {
    assert_eq!(backend_for(HostOs::MacOs), GpuBackend::Metal);
    assert_eq!(backend_for(HostOs::Windows), GpuBackend::Vulkan);
    assert_eq!(backend_for(HostOs::Linux), GpuBackend::Vulkan);
    match availability() {
        Ok(backend) => {
            assert_ne!(HostOs::current(), HostOs::Windows);
            assert_eq!(backend, backend_for(HostOs::current()));
        }
        Err(MapUnavailable::NoNativeCore { host, backend, reason }) => {
            assert_eq!(host, HostOs::Windows);
            assert_eq!(backend, backend_for(host));
            assert!(!reason.is_empty());
        }
        Err(other) => panic!("availability never starts a thread: {other}"),
    }
}

#[test]
fn a_frame_swaps_rgba_into_bgra_and_rejects_short_buffers() {
    let camera = Camera::new(MOSCOW, 10.0);
    let frame = MapFrame::from_rgba(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8], camera).expect("two pixels");
    assert_eq!(frame.bgra, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    assert_eq!(frame.camera, camera);
    assert!(frame.geometry().is_ok());
    assert!(MapFrame::from_rgba(2, 2, &[0; 8], camera).is_none());
    assert!(MapFrame::from_rgba(0, 2, &[], camera).is_none());
}

#[test]
fn draining_keeps_events_and_only_the_newest_frame() {
    let frame = |zoom: f64| {
        MapEvent::Frame(MapFrame::from_rgba(1, 1, &[0, 0, 0, 255], Camera::new(MOSCOW, zoom)).expect("one pixel"))
    };
    let kept = newest_frame_only(vec![frame(1.0), MapEvent::StyleLoaded, frame(2.0), MapEvent::Idle]);
    assert_eq!(kept, vec![MapEvent::StyleLoaded, frame(2.0), MapEvent::Idle]);
}

#[test]
fn a_drag_reports_changing_then_one_did_change() {
    let mut driver = CameraDriver::new(Camera::new(MOSCOW, 10.0));
    let moved = Camera::new(NEW_YORK, 10.0);
    let events = driver.apply(
        CameraCommand::Jump {
            camera: moved,
            reason: CameraChangeReason::GESTURE_PAN,
        },
        0.0,
    );
    assert!(matches!(events.as_slice(), [RegionEvent::IsChanging { reason, .. }] if *reason == CameraChangeReason::GESTURE_PAN));
    let settled = driver.settle();
    assert!(matches!(settled.as_slice(), [RegionEvent::DidChange { animated: false, .. }]));
    assert!(driver.settle().is_empty());
    assert_eq!(driver.camera(), moved.clamped());
}

#[test]
fn an_animation_ends_with_did_change_and_a_new_command_cancels_it() {
    let from = Camera::new(MOSCOW, 10.0);
    let to = Camera::new(MOSCOW, 12.0);
    let mut driver = CameraDriver::new(from);
    driver.apply(
        CameraCommand::Animate {
            animation: CameraAnimation::ease(from, to, 1.0),
            reason: CameraChangeReason::PROGRAMMATIC,
        },
        5.0,
    );
    assert!(driver.is_animating());
    let halfway = driver.tick(5.5);
    assert!(matches!(halfway.as_slice(), [RegionEvent::IsChanging { .. }]));
    let done = driver.tick(6.0);
    assert!(matches!(done.last(), Some(RegionEvent::DidChange { animated: true, .. })));
    assert_eq!(driver.camera(), to.clamped());
    assert!(!driver.is_animating());

    driver.apply(
        CameraCommand::Animate {
            animation: CameraAnimation::ease(to, from, 1.0),
            reason: CameraChangeReason::PROGRAMMATIC,
        },
        7.0,
    );
    let cut = driver.jump_to(from, CameraChangeReason::PROGRAMMATIC);
    assert!(matches!(
        cut.first(),
        Some(RegionEvent::DidChange { reason, .. }) if reason.intersects(CameraChangeReason::TRANSITION_CANCELLED)
    ));
    assert!(matches!(cut.last(), Some(RegionEvent::DidChange { animated: false, .. })));
}

#[test]
fn tile_bounds_cover_the_world_at_zoom_zero() {
    let (north_west, south_east) = TileId { zoom: 0, x: 0, y: 0 }.bounds();
    assert!((north_west.longitude + 180.0).abs() < 1e-9);
    assert!((south_east.longitude - 180.0).abs() < 1e-9);
    assert!((north_west.latitude - MAX_LATITUDE).abs() < 1e-6);
    assert!((south_east.latitude + MAX_LATITUDE).abs() < 1e-6);
}

#[test]
fn computed_tiles_cover_the_view_inside_the_zoom_range() {
    let projection = Projection::new(Camera::new(MOSCOW, 10.4), viewport());
    let tiles = ComputedTiles { min_zoom: 0, max_zoom: 8 }.tiles_in_view(&projection);
    assert!(!tiles.is_empty());
    assert!(tiles.iter().all(|tile| tile.zoom == 8));
    let (x, y) = mercator(MOSCOW);
    let home = TileId {
        zoom: 8,
        x: (x * 256.0).floor() as u32,
        y: (y * 256.0).floor() as u32,
    };
    assert!(tiles.contains(&home));
}

#[test]
fn a_feature_collection_keeps_only_features() {
    let json = feature_collection(vec![
        serde_json::json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [37.6, 55.7]}, "properties": {}}),
        serde_json::json!({"type": "Point", "coordinates": [0.0, 0.0]}),
    ]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(value["type"], "FeatureCollection");
    assert_eq!(value["features"].as_array().map(Vec::len), Some(1));
}

#[test]
fn style_requests_have_names_for_their_failures() {
    assert_eq!(MapRequest::SetStyle("{}".to_owned()).name(), "set_style");
    assert_eq!(
        MapRequest::AddLayer {
            layer_json: "{}".to_owned(),
            before: None,
        }
        .name(),
        "add_layer"
    );
    assert_eq!(MapRequest::RemoveImage("pin".to_owned()).name(), "remove_image");
}
