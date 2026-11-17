use super::display_cadence::frames_per_second_from_millihertz;
use super::refresh_mode::*;

fn timing(dot_clock: u32, h_total: u16, v_total: u16) -> ModeTiming {
    ModeTiming {
        dot_clock,
        h_total,
        v_total,
        interlace: false,
        double_scan: false,
    }
}

fn rect(x: i32, y: i32, width: u32, height: u32) -> ScreenRect {
    ScreenRect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn common_mode_lines_give_their_rates() {
    let cea = mode_millihertz(timing(148_500_000, 2200, 1125)).unwrap();
    assert_eq!(cea, 60_000);
    assert_eq!(frames_per_second_from_millihertz(cea), 60);
    let fast = mode_millihertz(timing(592_250_000, 2640, 1558)).unwrap();
    assert_eq!(fast, 143_990);
    assert_eq!(frames_per_second_from_millihertz(fast), 144);
    let ntsc = mode_millihertz(timing(148_351_648, 2200, 1125)).unwrap();
    assert_eq!(ntsc, 59_940);
    assert_eq!(frames_per_second_from_millihertz(ntsc), 60);
}

#[test]
fn interlace_and_double_scan_change_the_vertical_total() {
    let base = timing(74_250_000, 2200, 1125);
    let interlaced = ModeTiming {
        interlace: true,
        ..base
    };
    let doubled = ModeTiming {
        double_scan: true,
        ..base
    };
    assert_eq!(mode_millihertz(base), Some(30_000));
    assert_eq!(
        mode_millihertz(interlaced),
        Some(60_053),
        "1125 lines halve to 562, as xrandr prints 60.05"
    );
    assert_eq!(mode_millihertz(doubled), Some(15_000));
}

#[test]
fn a_mode_without_clock_or_totals_has_no_rate() {
    assert_eq!(mode_millihertz(timing(0, 2200, 1125)), None);
    assert_eq!(mode_millihertz(timing(148_500_000, 0, 1125)), None);
    assert_eq!(mode_millihertz(timing(148_500_000, 2200, 0)), None);
}

#[test]
fn the_crtc_with_most_of_the_window_wins() {
    let crtcs = [
        CrtcArea {
            rect: rect(0, 0, 1920, 1080),
            mode: 71,
        },
        CrtcArea {
            rect: rect(1920, 0, 2560, 1440),
            mode: 72,
        },
    ];
    assert_eq!(crtc_holding_most(rect(100, 100, 800, 600), &crtcs), Some(0));
    assert_eq!(crtc_holding_most(rect(1800, 100, 800, 600), &crtcs), Some(1));
    assert_eq!(crtc_holding_most(rect(1500, 100, 800, 600), &crtcs), Some(0));
}

#[test]
fn a_window_off_every_crtc_takes_the_first_enabled_one() {
    let crtcs = [
        CrtcArea {
            rect: rect(0, 0, 0, 0),
            mode: 0,
        },
        CrtcArea {
            rect: rect(0, 0, 1920, 1080),
            mode: 71,
        },
    ];
    assert_eq!(crtc_holding_most(rect(-5000, -5000, 300, 200), &crtcs), Some(1));
    assert_eq!(crtc_holding_most(rect(0, 0, 300, 200), &crtcs[..1]), None);
    assert_eq!(crtc_holding_most(rect(0, 0, 300, 200), &[]), None);
}

#[test]
fn output_table_follows_current_modes() {
    let mut outputs = OutputRefreshes::default();
    assert_eq!(outputs.fastest_millihertz(), None);
    outputs.add(30);
    outputs.add(31);
    outputs.add(30);
    assert_eq!(outputs.names().collect::<Vec<_>>(), vec![30, 31]);
    assert_eq!(outputs.fastest_millihertz(), None);

    outputs.mode(30, true, 59_940);
    outputs.mode(31, false, 240_000);
    assert_eq!(outputs.fastest_millihertz(), Some(59_940), "only the current mode counts");

    outputs.mode(31, true, 143_856);
    assert_eq!(outputs.fastest_millihertz(), Some(143_856));

    outputs.mode(31, true, 0);
    assert_eq!(
        outputs.fastest_millihertz(),
        Some(59_940),
        "a rate of 0 is an output without a fixed rate"
    );

    outputs.mode(31, true, 120_000);
    outputs.remove(31);
    assert_eq!(outputs.fastest_millihertz(), Some(59_940));
    assert_eq!(outputs.names().collect::<Vec<_>>(), vec![30]);
}
