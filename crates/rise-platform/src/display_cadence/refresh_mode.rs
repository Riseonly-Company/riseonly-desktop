/// timing of one xrandr mode line
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModeTiming {
    pub dot_clock: u32,
    pub h_total: u16,
    pub v_total: u16,
    pub interlace: bool,
    pub double_scan: bool,
}

/// refresh of a mode line in mhz like wayland reports, none for a mode without a clock or a total
pub fn mode_millihertz(mode: ModeTiming) -> Option<u32> {
    let mut v_total = u64::from(mode.v_total);
    if mode.double_scan {
        v_total *= 2;
    }
    if mode.interlace {
        v_total /= 2;
    }
    let lines = u64::from(mode.h_total) * v_total;
    if mode.dot_clock == 0 || lines == 0 {
        return None;
    }
    u32::try_from(u64::from(mode.dot_clock) * 1000 / lines).ok()
}

/// a rectangle in root window coordinates
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    fn overlap_area(self, other: ScreenRect) -> u64 {
        let left = i64::from(self.x.max(other.x));
        let top = i64::from(self.y.max(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        if right <= left || bottom <= top {
            return 0;
        }
        ((right - left) * (bottom - top)) as u64
    }
}

/// one crtc, where it scans out and its mode (0 is disabled)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CrtcArea {
    pub rect: ScreenRect,
    pub mode: u32,
}

/// index of the crtc that shows most of the window, like MonitorFromWindow on windows
/// a window off every crtc takes the first enabled one, none when no crtc is enabled
pub fn crtc_holding_most(window: ScreenRect, crtcs: &[CrtcArea]) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    let mut first_enabled = None;
    for (index, crtc) in crtcs.iter().enumerate() {
        if crtc.mode == 0 || crtc.rect.width == 0 || crtc.rect.height == 0 {
            continue;
        }
        first_enabled.get_or_insert(index);
        let area = window.overlap_area(crtc.rect);
        if area > 0 && best.map_or(true, |(_, best_area)| area > best_area) {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index).or(first_enabled)
}

/// current refresh of every wl_output the linux layer bound, keyed by the registry name of the global
/// updated from mode and global_remove events
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct OutputRefreshes {
    outputs: Vec<(u32, Option<u32>)>,
}

impl OutputRefreshes {
    /// a new output global was bound, its refresh is unknown until its current mode arrives
    pub fn add(&mut self, name: u32) {
        if !self.outputs.iter().any(|(known, _)| *known == name) {
            self.outputs.push((name, None));
        }
    }

    /// a wl_output mode event, only the mode flagged current counts (version 4 sends only that one)
    /// a refresh of 0 or below means no fixed rate and clears what was known
    pub fn mode(&mut self, name: u32, current: bool, refresh_millihertz: i32) {
        if !current {
            return;
        }
        let refresh = u32::try_from(refresh_millihertz).ok().filter(|rate| *rate > 0);
        match self.outputs.iter_mut().find(|(known, _)| *known == name) {
            Some((_, slot)) => *slot = refresh,
            None => self.outputs.push((name, refresh)),
        }
    }

    /// the output global went away, monitor unplugged
    pub fn remove(&mut self, name: u32) {
        self.outputs.retain(|(known, _)| *known != name);
    }

    /// registry names of the bound outputs in bind order
    pub fn names(&self) -> impl Iterator<Item = u32> + '_ {
        self.outputs.iter().map(|(name, _)| *name)
    }

    /// highest current refresh among the outputs in mhz
    pub fn fastest_millihertz(&self) -> Option<u32> {
        self.outputs.iter().filter_map(|(_, refresh)| *refresh).max()
    }
}
