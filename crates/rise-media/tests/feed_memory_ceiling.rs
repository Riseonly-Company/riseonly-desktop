//! Scroll a synthetic feed and assert the memory ceiling, twice over: exactly
//! against the ledger, and coarsely against RSS where the OS reports it.

use rise_media::texture::external_texture::{
    BudgetTracker, FRAMES_IN_FLIGHT, ImportPlan, StreamTexture, TextureBudget,
};
use rise_media::texture::frame::{FrameFormat, FrameGeometry};
use rise_media::video::feed_scheduler::{FeedScheduler, StreamId, StreamState, Viewport};
use rise_platform::HostOs;

use std::collections::HashMap;

const FEED_LENGTH: u64 = 500;
const FRAMES_PER_STREAM: u32 = 90;

fn hd() -> FrameGeometry {
    FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap()
}

fn viewport_at(position: u64) -> Viewport {
    Viewport::new(
        vec![position],
        (position + 1..(position + 4).min(FEED_LENGTH)).collect(),
        (0..position).rev().take(3).collect(),
    )
}

/// Drives the scheduler's plan against real `StreamTexture` allocations.
struct SyntheticFeed {
    scheduler: FeedScheduler,
    textures: HashMap<StreamId, StreamTexture>,
    tracker: BudgetTracker,
    peak_bytes: u64,
}

impl SyntheticFeed {
    fn new(budget: TextureBudget) -> Self {
        let tracker = BudgetTracker::new(budget);

        Self {
            scheduler: FeedScheduler::new(tracker.clone()),
            textures: HashMap::new(),
            tracker,
            peak_bytes: 0,
        }
    }

    fn scroll_to(&mut self, position: u64) {
        let plan = self.scheduler.reconcile(&viewport_at(position));

        // Closes first, as the plan orders them; opens first would hold both.
        for id in plan.closes() {
            self.textures.remove(&id);
        }

        for id in plan.opens() {
            let texture = StreamTexture::open(
                hd(),
                ImportPlan::hardware(HostOs::current(), FrameFormat::Nv12),
                self.tracker.clone(),
            )
            .expect("the scheduler's pool cap must keep every open within the budget");

            self.textures.insert(id, texture);
        }

        // Every live stream decodes, whether visible or warm.
        for texture in self.textures.values_mut() {
            for _ in 0..FRAMES_PER_STREAM {
                texture
                    .write(&hd())
                    .expect("steady-state playback must not reallocate");
            }
        }

        self.peak_bytes = self.peak_bytes.max(self.tracker.in_use());
    }
}

#[test]
fn scrolling_a_long_feed_never_crosses_the_texture_ceiling() {
    let mut feed = SyntheticFeed::new(TextureBudget::default());

    for position in 0..FEED_LENGTH {
        feed.scroll_to(position);

        assert!(
            feed.tracker.in_use() <= feed.tracker.ceiling(),
            "at item {position} the feed held {} bytes against a ceiling of {}",
            feed.tracker.in_use(),
            feed.tracker.ceiling()
        );
    }

    assert!(
        feed.peak_bytes > 0,
        "the feed never allocated, so this proved nothing"
    );
}

#[test]
fn a_five_hundred_item_feed_costs_no_more_than_a_five_item_one() {
    let mut short_feed = SyntheticFeed::new(TextureBudget::default());
    for position in 0..5 {
        short_feed.scroll_to(position);
    }

    let mut long_feed = SyntheticFeed::new(TextureBudget::default());
    for position in 0..FEED_LENGTH {
        long_feed.scroll_to(position);
    }

    assert_eq!(
        long_feed.peak_bytes, short_feed.peak_bytes,
        "memory grew with feed length, which is the leak this whole module exists to prevent"
    );
}

#[test]
fn three_simultaneous_streams_stay_inside_the_ceiling_for_a_full_minute() {
    let tracker = BudgetTracker::default();
    let plan = ImportPlan::hardware(HostOs::current(), FrameFormat::Nv12);

    let mut streams: Vec<StreamTexture> = (0..3)
        .map(|_| StreamTexture::open(hd(), plan, tracker.clone()).unwrap())
        .collect();

    let after_open = tracker.in_use();

    // Sixty seconds at 60fps on all three.
    for _ in 0..3600 {
        for stream in &mut streams {
            stream.write(&hd()).unwrap();
        }
    }

    assert_eq!(
        tracker.in_use(),
        after_open,
        "a minute of playback allocated; frames are not being written into the same texture"
    );
    assert!(tracker.in_use() <= tracker.ceiling());
    assert_eq!(
        after_open,
        hd().byte_len() * FRAMES_IN_FLIGHT * 3,
        "three 1080p streams should cost exactly three streams' worth"
    );
}

#[test]
fn tearing_the_feed_down_returns_every_byte() {
    let mut feed = SyntheticFeed::new(TextureBudget::default());
    for position in 0..50 {
        feed.scroll_to(position);
    }

    assert!(feed.tracker.in_use() > 0);

    feed.scheduler.close_all();
    feed.textures.clear();

    assert_eq!(
        feed.tracker.in_use(),
        0,
        "leaving the feed must release GPU memory, not merely stop decoding"
    );
}

#[test]
fn the_scheduler_and_the_allocations_never_disagree() {
    let mut feed = SyntheticFeed::new(TextureBudget::default());

    for position in 0..FEED_LENGTH {
        feed.scroll_to(position);

        let live: Vec<StreamId> = feed.textures.keys().copied().collect();
        assert_eq!(
            live.len(),
            feed.scheduler.live_count(),
            "at item {position} the pool and the allocations disagreed"
        );

        for id in live {
            assert_ne!(
                feed.scheduler.state(id),
                StreamState::Closed,
                "stream {id} holds a texture the scheduler believes is closed"
            );
        }
    }
}

/// Resident set size in bytes, or `None` where the OS will not report it.
fn resident_bytes() -> Option<u64> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return None;
    }

    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;

    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

#[test]
fn the_process_itself_does_not_grow_across_a_long_scroll() {
    let Some(before) = resident_bytes() else {
        eprintln!("skipped: resident set size is not readable on this host");
        return;
    };

    let mut feed = SyntheticFeed::new(TextureBudget::default());
    for position in 0..FEED_LENGTH {
        feed.scroll_to(position);
    }
    feed.scheduler.close_all();
    feed.textures.clear();

    let after = resident_bytes().expect("rss was readable a moment ago");
    let growth = after.saturating_sub(before);

    // Generous: this catches a leak proportional to feed length, not allocator noise.
    const ALLOWED_GROWTH: u64 = 64 * 1024 * 1024;

    assert!(
        growth < ALLOWED_GROWTH,
        "scrolling {FEED_LENGTH} items grew RSS by {growth} bytes ({} MiB)",
        growth / 1024 / 1024
    );
}
