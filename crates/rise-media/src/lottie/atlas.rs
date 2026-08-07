//! One texture for hundreds of stickers.
//!
//! THE PROBLEM THIS SOLVES
//!
//! A sticker is 32 to 256 pixels square. Giving each one its own texture means
//! an emoji picker allocates several hundred GPU objects, rebinds once per
//! sticker per frame, and — because a sticker appears and disappears with the
//! scroll — allocates and frees them continuously. That is the per-frame
//! allocation pattern the whole media layer exists to avoid, at a different
//! scale from video.
//!
//! So stickers live in shared pages and a player owns a rectangle inside one.
//! Frames are written into that rectangle; nothing is allocated between a
//! sticker appearing and disappearing.
//!
//! WHY A GRID AND NOT A PACKER
//!
//! Every tile is square and comes from a small set of sizes, so a shelf or
//! skyline packer would solve a problem this workload does not have while
//! adding fragmentation, which is the failure mode that eventually shows up as
//! "the picker stops drawing after ten minutes". A bucketed grid is O(1) to
//! allocate, O(1) to free, and cannot fragment.

use std::collections::HashMap;

use super::super::texture::external_texture::{BudgetError, BudgetTracker};
use super::raster_policy::{MAXIMUM_DIMENSION, MINIMUM_DIMENSION};

/// Tile sizes are rounded up to a multiple of this.
///
/// Without it, every distinct point size on every distinct monitor scale is its
/// own page: 148, 150 and 152 would be three atlases holding one sticker each.
/// The cost is at most 31 wasted pixels per side.
pub const TILE_GRANULARITY: u32 = 32;

/// Side of every atlas page, in pixels.
///
/// 1024² BGRA is 4 MiB, which is one comfortable allocation and divides evenly
/// for the common tile sizes. Larger pages waste more on the last partial row
/// of tiles; smaller ones stop being worth the sharing.
pub const PAGE_SIDE: u32 = 1024;

pub const fn page_bytes() -> u64 {
    PAGE_SIDE as u64 * PAGE_SIDE as u64 * 4
}

/// The tile size a rasterised sticker of this dimension occupies.
pub fn tile_side(dimension: u32) -> u32 {
    let clamped = dimension.clamp(MINIMUM_DIMENSION, MAXIMUM_DIMENSION);
    clamped.div_ceil(TILE_GRANULARITY) * TILE_GRANULARITY
}

pub const fn tiles_per_row(tile: u32) -> u32 {
    PAGE_SIDE / tile
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AtlasError {
    /// A new page would cross the process-wide GPU memory ceiling. The caller
    /// releases an offscreen sticker rather than the atlas silently growing.
    Budget(BudgetError),
}

/// Where one sticker's frames are written.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Tile {
    pub tile_side: u32,
    pub page: usize,
    pub slot: u32,
}

impl Tile {
    /// Pixel origin of this tile inside its page.
    pub const fn origin(&self) -> (u32, u32) {
        let per_row = tiles_per_row(self.tile_side);
        let column = self.slot % per_row;
        let row = self.slot / per_row;
        (column * self.tile_side, row * self.tile_side)
    }

    /// Normalised texture coordinates: what a shader is actually given.
    pub fn uv_rect(&self) -> (f32, f32, f32, f32) {
        let (x, y) = self.origin();
        let side = PAGE_SIDE as f32;
        (
            x as f32 / side,
            y as f32 / side,
            self.tile_side as f32 / side,
            self.tile_side as f32 / side,
        )
    }
}

#[derive(Debug)]
struct Page {
    /// One bit would do, but the free list makes allocation O(1) rather than a
    /// scan, and a picker allocates on every scroll tick.
    free: Vec<u32>,
    live: u32,
}

impl Page {
    fn new(tile: u32) -> Self {
        let per_row = tiles_per_row(tile);
        let capacity = per_row * per_row;
        Self {
            free: (0..capacity).rev().collect(),
            live: 0,
        }
    }
}

/// The process's sticker texture memory.
///
/// Shares the video path's ledger on purpose: a feed playing three videos and a
/// chat showing forty stickers are competing for the same GPU memory, and two
/// independent ceilings add up to a number nobody chose.
#[derive(Debug)]
pub struct TextureAtlas {
    pages: HashMap<u32, Vec<Page>>,
    tracker: BudgetTracker,
    reserved_pages: usize,
}

impl TextureAtlas {
    pub fn new(tracker: BudgetTracker) -> Self {
        Self {
            pages: HashMap::new(),
            tracker,
            reserved_pages: 0,
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_pages as u64 * page_bytes()
    }

    pub fn budget(&self) -> &BudgetTracker {
        &self.tracker
    }

    pub fn allocate(&mut self, dimension: u32) -> Result<Tile, AtlasError> {
        let tile = tile_side(dimension);
        let pages = self.pages.entry(tile).or_default();

        for (index, page) in pages.iter_mut().enumerate() {
            if let Some(slot) = page.free.pop() {
                page.live += 1;
                return Ok(Tile {
                    tile_side: tile,
                    page: index,
                    slot,
                });
            }
        }

        self.tracker
            .reserve(page_bytes())
            .map_err(AtlasError::Budget)?;
        self.reserved_pages += 1;

        let pages = self.pages.entry(tile).or_default();
        let mut page = Page::new(tile);
        let slot = page.free.pop().expect("a fresh page has free tiles");
        page.live += 1;
        pages.push(page);

        Ok(Tile {
            tile_side: tile,
            page: pages.len() - 1,
            slot,
        })
    }

    /// Hand a tile back.
    ///
    /// An emptied page is kept rather than freed. A picker scrolled up and down
    /// empties and refills the same page continuously, and returning its memory
    /// on every pass would trade a bounded allocation for a permanent churn.
    /// `shrink` is where the memory actually goes back.
    pub fn release(&mut self, tile: Tile) {
        let Some(pages) = self.pages.get_mut(&tile.tile_side) else {
            return;
        };
        let Some(page) = pages.get_mut(tile.page) else {
            return;
        };
        if page.free.contains(&tile.slot) {
            return;
        }

        page.free.push(tile.slot);
        page.live = page.live.saturating_sub(1);
    }

    /// Release every page that holds nothing, keeping one per tile size.
    ///
    /// Called when the sticker surface leaves the screen entirely, on account
    /// switch, and under memory pressure. Keeping one page per size means the
    /// next chat opened does not pay for a fresh allocation.
    ///
    /// Only trailing empty pages are released: a tile's index is its page's
    /// position, so removing from the middle would silently repoint every live
    /// tile above it at someone else's memory.
    pub fn shrink(&mut self) {
        for pages in self.pages.values_mut() {
            while pages.len() > 1 && pages.last().is_some_and(|page| page.live == 0) {
                pages.pop();
                self.reserved_pages -= 1;
                self.tracker.release(page_bytes());
            }
        }
    }

    /// Give everything back. The atlas is empty afterwards and every previously
    /// issued tile is invalid.
    pub fn release_all(&mut self) {
        self.pages.clear();
        self.tracker
            .release(self.reserved_pages as u64 * page_bytes());
        self.reserved_pages = 0;
    }
}

impl Drop for TextureAtlas {
    fn drop(&mut self) {
        self.tracker
            .release(self.reserved_pages as u64 * page_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::texture::external_texture::TextureBudget;
    use super::*;

    #[test]
    fn tile_sizes_collapse_to_a_small_set() {
        assert_eq!(tile_side(32), 32);
        assert_eq!(tile_side(40), 64);
        assert_eq!(tile_side(148), 160);
        assert_eq!(tile_side(150), 160);
        assert_eq!(tile_side(152), 160);
        assert_eq!(tile_side(256), 256);
    }

    #[test]
    fn no_dimension_produces_a_tile_larger_than_a_page() {
        for dimension in 1..=512 {
            let tile = tile_side(dimension);
            assert!(
                tile <= PAGE_SIDE,
                "dimension {dimension} needs a {tile}px tile"
            );
            assert!(tiles_per_row(tile) >= 1);
        }
    }

    #[test]
    fn tiles_never_overlap_within_a_page() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let mut seen: Vec<(u32, u32)> = Vec::new();

        let capacity = tiles_per_row(64) * tiles_per_row(64);
        for _ in 0..capacity {
            let tile = atlas.allocate(64).unwrap();
            let origin = tile.origin();
            assert!(!seen.contains(&origin), "two stickers share {origin:?}");
            assert!(origin.0 + 64 <= PAGE_SIDE && origin.1 + 64 <= PAGE_SIDE);
            seen.push(origin);
        }

        assert_eq!(atlas.page_count(), 1);
    }

    #[test]
    fn a_full_page_spills_into_a_second_one() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let capacity = tiles_per_row(256) * tiles_per_row(256);

        for _ in 0..capacity {
            atlas.allocate(256).unwrap();
        }
        assert_eq!(atlas.page_count(), 1);

        atlas.allocate(256).unwrap();
        assert_eq!(atlas.page_count(), 2);
    }

    #[test]
    fn two_tile_sizes_do_not_share_a_page() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());

        atlas.allocate(32).unwrap();
        atlas.allocate(256).unwrap();

        assert_eq!(atlas.page_count(), 2);
    }

    #[test]
    fn a_released_tile_is_handed_out_again() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let first = atlas.allocate(128).unwrap();

        atlas.release(first);
        let second = atlas.allocate(128).unwrap();

        assert_eq!(first, second);
        assert_eq!(atlas.page_count(), 1);
    }

    #[test]
    fn scrolling_a_picker_forever_never_grows_the_atlas() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let mut live: Vec<Tile> = Vec::new();

        for step in 0..5_000 {
            live.push(atlas.allocate(64).unwrap());
            if live.len() > 40 {
                atlas.release(live.remove(0));
            }
            assert!(
                atlas.page_count() <= 1,
                "at step {step} the atlas had grown to {} pages",
                atlas.page_count()
            );
        }
    }

    #[test]
    fn the_atlas_is_priced_into_the_same_ceiling_as_video() {
        let tracker = BudgetTracker::new(TextureBudget::new(page_bytes() * 2));
        let mut atlas = TextureAtlas::new(tracker.clone());

        atlas.allocate(256).unwrap();
        assert_eq!(tracker.in_use(), page_bytes());

        atlas.allocate(32).unwrap();
        assert_eq!(tracker.in_use(), page_bytes() * 2);

        assert!(matches!(
            atlas.allocate(128),
            Err(AtlasError::Budget(BudgetError::WouldExceedCeiling { .. }))
        ));
    }

    #[test]
    fn shrinking_returns_memory_but_keeps_one_page_warm() {
        let tracker = BudgetTracker::default();
        let mut atlas = TextureAtlas::new(tracker.clone());

        let capacity = tiles_per_row(256) * tiles_per_row(256);
        let tiles: Vec<Tile> = (0..capacity + 1)
            .map(|_| atlas.allocate(256).unwrap())
            .collect();
        assert_eq!(atlas.page_count(), 2);

        for tile in tiles {
            atlas.release(tile);
        }
        atlas.shrink();

        assert_eq!(atlas.page_count(), 1);
        assert_eq!(tracker.in_use(), page_bytes());
    }

    #[test]
    fn shrinking_never_moves_a_page_a_live_tile_points_at() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let capacity = (tiles_per_row(256) * tiles_per_row(256)) as usize;

        let first_page: Vec<Tile> = (0..capacity)
            .map(|_| atlas.allocate(256).unwrap())
            .collect();
        let second_page_tile = atlas.allocate(256).unwrap();
        assert_eq!(second_page_tile.page, 1);

        for tile in &first_page {
            atlas.release(*tile);
        }
        atlas.shrink();

        assert_eq!(atlas.page_count(), 2, "page 0 is empty but page 1 is live");
        let reissued = atlas.allocate(256).unwrap();
        assert_eq!(reissued.page, 0);
        assert_ne!(reissued, second_page_tile);
    }

    #[test]
    fn dropping_the_atlas_returns_its_whole_reservation() {
        let tracker = BudgetTracker::default();
        {
            let mut atlas = TextureAtlas::new(tracker.clone());
            for _ in 0..40 {
                atlas.allocate(256).unwrap();
            }
            assert!(tracker.in_use() > 0);
        }

        assert_eq!(tracker.in_use(), 0);
    }

    #[test]
    fn releasing_a_tile_twice_does_not_hand_the_same_slot_to_two_stickers() {
        let mut atlas = TextureAtlas::new(BudgetTracker::default());
        let tile = atlas.allocate(64).unwrap();

        atlas.release(tile);
        atlas.release(tile);

        let first = atlas.allocate(64).unwrap();
        let second = atlas.allocate(64).unwrap();
        assert_ne!(first, second);
    }
}
