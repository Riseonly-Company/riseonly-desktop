use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use gpui::{Bounds, Pixels, Size, point, px, size};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;

use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageMediaItem, MessageMediaKind, MessageMediaViewPhase, RemoteMediaURL,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MessageGridCell {
    pub media_item_id: String,
    pub render_id: String,
    pub index: usize,
    pub upload_slot_index: i64,
    pub kind: MessageMediaKind,
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub duration: Option<f64>,
    pub display_source: String,
    pub upload_seed: Option<MessageGridUploadSeed>,
    pub download_seed: Option<MessageGridDownloadSeed>,
    pub cell_frame: Bounds<Pixels>,
    pub image_frame: Bounds<Pixels>,
    pub has_spoiler: bool,
    pub view_phase: Option<MessageMediaViewPhase>,
    pub view_is_timed: bool,
    pub view_seconds: Option<i64>,
}

impl MessageGridCell {
    pub fn id(&self) -> &str {
        &self.render_id
    }
}

impl Eq for MessageGridCell {}

impl Hash for MessageGridCell {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.media_item_id.hash(state);
        self.render_id.hash(state);
        self.index.hash(state);
        self.upload_slot_index.hash(state);
        self.kind.hash(state);
        self.url.hash(state);
        self.thumbnail_url.hash(state);
        self.duration.map(f64::to_bits).hash(state);
        self.display_source.hash(state);
        self.upload_seed.hash(state);
        self.download_seed.hash(state);
        self.cell_frame.hash(state);
        self.image_frame.hash(state);
        self.has_spoiler.hash(state);
        self.view_phase.hash(state);
        self.view_is_timed.hash(state);
        self.view_seconds.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageGridUploadSeed {
    pub is_compressing: bool,
    pub fraction: f64,
    pub completed_bytes: i64,
    pub total_bytes: i64,
}

impl Eq for MessageGridUploadSeed {}

impl Hash for MessageGridUploadSeed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.is_compressing.hash(state);
        self.fraction.to_bits().hash(state);
        self.completed_bytes.hash(state);
        self.total_bytes.hash(state);
    }
}

/// static facts a tile needs to describe an incoming transfer
/// no live bytes on purpose, a byte count here would rediff the whole mosaic every tick
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageGridDownloadSeed {
    pub cache_key: String,
    pub remote_url: String,
    pub declared_byte_count: i64,
    pub is_streamable: bool,
    pub shows_idle_affordance: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageGridLayout {
    pub cells: Vec<MessageGridCell>,
    pub container_size: Size<Pixels>,
    pub is_single: bool,
    pub rows: Vec<MessageGridRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageGridRow {
    pub top: Pixels,
    pub height: Pixels,
    pub cell_indices: Vec<usize>,
}

pub enum MessageGridLayoutEngine {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageGridLayoutEngineProportion {
    Portrait,
    Square,
    Landscape,
}

struct MessageGridLayoutEngineMosaicCandidate {
    cells_per_line: Vec<usize>,
    line_heights: Vec<f32>,
}

/// frame math in plain points, gpui bounds are built once per cell
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GridRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl GridRect {
    fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    fn max_x(&self) -> f32 {
        self.x + self.width
    }

    fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(point(px(self.x), px(self.y)), size(px(self.width), px(self.height)))
    }
}

/// memo for the mosaic solve, compute runs first on every render so it re-solved on each reconfigure
/// a single item takes the cheap path but an album runs the full mosaic
#[derive(Clone, PartialEq, Eq, Hash)]
struct MessageGridLayoutEngineCacheKey {
    items: Vec<MessageMediaItem>,
    max_width: u32,
    max_row_height: u32,
    spacing: u32,
    owner_local_id: Option<String>,
}

#[derive(Default)]
struct MessageGridLayoutEngineCache {
    cache: HashMap<MessageGridLayoutEngineCacheKey, MessageGridLayout>,
    cache_order: VecDeque<MessageGridLayoutEngineCacheKey>,
}

const CACHE_LIMIT: usize = 96;
const MINIMUM_MOSAIC_CELL_WIDTH: f32 = 68.0;
const MINIMUM_MOSAIC_CELL_HEIGHT: f32 = 81.0;

thread_local! {
    static CACHE: RefCell<MessageGridLayoutEngineCache> = RefCell::new(MessageGridLayoutEngineCache::default());
}

impl MessageGridLayoutEngine {
    pub const COMPACT_MAXIMUM_SIZE: Size<Pixels> = size(px(300.0), px(380.0));
    pub const COMPACT_MINIMUM_SIZE: Size<Pixels> = size(px(170.0), px(74.0));
    pub const REGULAR_MAXIMUM_SIZE: Size<Pixels> = size(px(440.0), px(440.0));

    fn cached(key: &MessageGridLayoutEngineCacheKey) -> Option<MessageGridLayout> {
        CACHE.with(|cache| cache.borrow().cache.get(key).cloned())
    }

    fn store(layout: &MessageGridLayout, key: MessageGridLayoutEngineCacheKey) {
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.cache.contains_key(&key) {
                cache.cache_order.push_back(key.clone());
                if cache.cache_order.len() > CACHE_LIMIT
                    && let Some(evicted) = cache.cache_order.pop_front()
                {
                    cache.cache.remove(&evicted);
                }
            }
            cache.cache.insert(key, layout.clone());
        });
    }

    /// computes the grid layout, spacing defaults to 1 and owner local id to none
    pub fn compute(
        items: &[MessageMediaItem],
        max_width: f32,
        max_row_height: f32,
        spacing: f32,
        owner_local_id: Option<&str>,
    ) -> MessageGridLayout {
        if items.is_empty() {
            return MessageGridLayout {
                cells: Vec::new(),
                container_size: Size::default(),
                is_single: false,
                rows: Vec::new(),
            };
        }
        let cache_key = MessageGridLayoutEngineCacheKey {
            items: items.to_vec(),
            max_width: max_width.to_bits(),
            max_row_height: max_row_height.to_bits(),
            spacing: spacing.to_bits(),
            owner_local_id: owner_local_id.map(str::to_owned),
        };
        if let Some(hit) = Self::cached(&cache_key) {
            return hit;
        }

        let maximum_size = Self::maximum_size(max_row_height);
        let maximum_width = f32::from(maximum_size.width);
        let maximum_height = f32::from(maximum_size.height);
        let available_width = max_width.min(maximum_width).max(1.0);
        let available_height = max_row_height.min(maximum_height).max(1.0);

        if items.len() == 1 {
            let single = Self::single_item_layout(
                &items[0],
                available_width,
                available_height,
                maximum_size,
                owner_local_id,
                items[0].local_slot_index.max(0),
            );
            Self::store(&single, cache_key);
            return single;
        }

        let scale = 1f32
            .min(available_width / maximum_width)
            .min(available_height / maximum_height);
        let mosaic_limit = (maximum_width * scale, maximum_height * scale);
        let ratios: Vec<f32> = items.iter().map(Self::aspect).collect();
        let frames = Self::mosaic_frames(&ratios, mosaic_limit, spacing.max(0.0));
        let cells: Vec<MessageGridCell> = items
            .iter()
            .zip(frames)
            .enumerate()
            .map(|(index, (item, frame))| {
                let upload_slot_index = item.local_slot_index.max(0);
                MessageGridCell {
                    media_item_id: item.id.clone(),
                    render_id: Self::render_id(owner_local_id, item),
                    index,
                    upload_slot_index,
                    kind: item.kind,
                    url: item.url.clone(),
                    thumbnail_url: item.thumbnail_url.clone(),
                    duration: item.duration,
                    display_source: item.display_source(),
                    upload_seed: Self::upload_seed(item),
                    download_seed: Self::download_seed(item),
                    cell_frame: frame.bounds(),
                    image_frame: Self::aspect_fit_frame(Self::aspect(item), frame).bounds(),
                    has_spoiler: item.has_spoiler,
                    view_phase: item.view_phase,
                    view_is_timed: item.view_limit.as_ref().is_some_and(|limit| limit.is_timed()),
                    view_seconds: item.view_limit.as_ref().and_then(|limit| limit.seconds),
                }
            })
            .collect();
        let maximum_x = cells
            .iter()
            .map(|cell| f32::from(cell.cell_frame.right()))
            .fold(None, |acc: Option<f32>, value| Some(acc.map_or(value, |acc| acc.max(value))))
            .unwrap_or(0.0);
        let maximum_y = cells
            .iter()
            .map(|cell| f32::from(cell.cell_frame.bottom()))
            .fold(None, |acc: Option<f32>, value| Some(acc.map_or(value, |acc| acc.max(value))))
            .unwrap_or(0.0);
        let container_size = size(px(maximum_x.round()), px(maximum_y.round()));
        let rows = Self::horizontal_bands(&cells);
        let mosaic = MessageGridLayout { cells, container_size, is_single: false, rows };
        Self::store(&mosaic, cache_key);
        mosaic
    }

    fn single_item_layout(
        item: &MessageMediaItem,
        available_width: f32,
        available_height: f32,
        maximum_size: Size<Pixels>,
        owner_local_id: Option<&str>,
        upload_slot_index: i64,
    ) -> MessageGridLayout {
        let ratio = Self::aspect(item);
        let native_size = if item.width.is_finite() && item.height.is_finite() && item.width > 0.0 && item.height > 0.0 {
            (item.width, item.height)
        } else {
            (available_width, available_width / ratio)
        };

        let maximum_fit_scale =
            (f32::from(maximum_size.width) / native_size.0).min(f32::from(maximum_size.height) / native_size.1);
        let maximum_fitted_width = (native_size.0 * maximum_fit_scale).floor();
        let minimum_width = f32::from(Self::COMPACT_MINIMUM_SIZE.width).min(available_width);
        let width = minimum_width
            .max(native_size.0.min(maximum_fitted_width).min(available_width))
            .floor();
        let minimum_height = f32::from(Self::COMPACT_MINIMUM_SIZE.height).min(available_height);
        let height = minimum_height
            .max((width / ratio).floor().min(available_height))
            .floor();
        let cell_frame = GridRect::new(0.0, 0.0, width, height);
        let cell = MessageGridCell {
            media_item_id: item.id.clone(),
            render_id: Self::render_id(owner_local_id, item),
            index: 0,
            upload_slot_index,
            kind: item.kind,
            url: item.url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            duration: item.duration,
            display_source: item.display_source(),
            upload_seed: Self::upload_seed(item),
            download_seed: Self::download_seed(item),
            cell_frame: cell_frame.bounds(),
            image_frame: Self::aspect_fit_frame(ratio, cell_frame).bounds(),
            has_spoiler: item.has_spoiler,
            view_phase: item.view_phase,
            view_is_timed: item.view_limit.as_ref().is_some_and(|limit| limit.is_timed()),
            view_seconds: item.view_limit.as_ref().and_then(|limit| limit.seconds),
        };
        MessageGridLayout {
            cells: vec![cell],
            container_size: size(px(width), px(height)),
            is_single: true,
            rows: vec![MessageGridRow { top: px(0.0), height: px(height), cell_indices: vec![0] }],
        }
    }

    fn render_id(owner_local_id: Option<&str>, item: &MessageMediaItem) -> String {
        match owner_local_id.map(trimming_whitespaces_and_newlines) {
            Some(owner_local_id) if !owner_local_id.is_empty() => item.stable_render_id(owner_local_id),
            _ => item.id.clone(),
        }
    }

    fn upload_seed(item: &MessageMediaItem) -> Option<MessageGridUploadSeed> {
        if !item.shows_upload_overlay() {
            return None;
        }
        Some(MessageGridUploadSeed {
            is_compressing: item.is_compressing,
            fraction: item.normalized_transfer_progress(),
            completed_bytes: item.clamped_uploaded_bytes(),
            total_bytes: item.effective_total_bytes(),
        })
    }

    pub fn download_seed(item: &MessageMediaItem) -> Option<MessageGridDownloadSeed> {
        if item.is_uploading || item.is_compressing || item.resolved_local_preview_url().is_some() {
            return None;
        }

        let is_video = item.kind == MessageMediaKind::Video;
        let raw_source = if is_video { item.url.clone() } else { item.display_source() };
        let normalized = RemoteMediaURL::optional_normalized(Some(raw_source.as_str()))?;
        let remote = Url::from_string(&normalized)?;
        let scheme = remote.scheme().map(str::to_lowercase);
        if scheme.as_deref() != Some("https") && scheme.as_deref() != Some("http") {
            return None;
        }

        Some(MessageGridDownloadSeed {
            cache_key: RiseMediaCacheIdentity::key(&remote),
            remote_url: normalized,
            declared_byte_count: if is_video { item.file_size.unwrap_or(0).max(0) } else { 0 },
            is_streamable: is_video,
            shows_idle_affordance: is_video,
        })
    }

    fn maximum_size(max_row_height: f32) -> Size<Pixels> {
        if max_row_height > f32::from(Self::COMPACT_MAXIMUM_SIZE.height) {
            return Self::REGULAR_MAXIMUM_SIZE;
        }
        Self::COMPACT_MAXIMUM_SIZE
    }

    fn mosaic_frames(ratios: &[f32], limit: (f32, f32), spacing: f32) -> Vec<GridRect> {
        let proportions: Vec<MessageGridLayoutEngineProportion> =
            ratios.iter().map(|&ratio| Self::proportion(ratio)).collect();
        let biased_average = (1.0 + ratios.iter().sum::<f32>()) / ratios.len() as f32;
        let requires_general_layout =
            limit.0 < f32::from(Self::COMPACT_MINIMUM_SIZE.width) || ratios.iter().any(|&ratio| ratio > 2.0);

        if !requires_general_layout {
            match ratios.len() {
                2 => return Self::two_item_frames(ratios, &proportions, biased_average, limit, spacing),
                3 => {
                    return Self::three_item_frames(
                        ratios,
                        proportions[0] == MessageGridLayoutEngineProportion::Portrait,
                        limit,
                        spacing,
                    );
                }
                4 => {
                    return Self::four_item_frames(
                        ratios,
                        proportions[0] == MessageGridLayoutEngineProportion::Landscape,
                        limit,
                        spacing,
                    );
                }
                _ => {}
            }
        }

        Self::optimized_frames(ratios, biased_average, limit.0, spacing)
    }

    fn two_item_frames(
        ratios: &[f32],
        proportions: &[MessageGridLayoutEngineProportion],
        average: f32,
        limit: (f32, f32),
        spacing: f32,
    ) -> Vec<GridRect> {
        let (limit_width, limit_height) = limit;
        let both_landscape = proportions.iter().all(|&p| p == MessageGridLayoutEngineProportion::Landscape);
        let both_square = proportions.iter().all(|&p| p == MessageGridLayoutEngineProportion::Square);
        let maximum_ratio = limit_width / limit_height.max(1.0);

        if both_landscape && average > 1.4 * maximum_ratio && ratios[1] - ratios[0] < 0.2 {
            let height = (limit_width / ratios[0])
                .min(limit_width / ratios[1])
                .min((limit_height - spacing) / 2.0)
                .floor();
            return vec![
                GridRect::new(0.0, 0.0, limit_width, height),
                GridRect::new(0.0, height + spacing, limit_width, height),
            ];
        }

        if both_landscape || both_square {
            let width = (limit_width - spacing) / 2.0;
            let height = (width / ratios[0]).min(width / ratios[1]).min(limit_height).floor();
            return vec![
                GridRect::new(0.0, 0.0, width, height),
                GridRect::new(width + spacing, 0.0, width, height),
            ];
        }

        let usable_width = limit_width - spacing;
        let balanced_second_width = (usable_width / ratios[0] / (1.0 / ratios[0] + 1.0 / ratios[1])).round();
        let second_width = (usable_width * 0.5).min(balanced_second_width).floor();
        let first_width = limit_width - second_width - spacing;
        let height = limit_height
            .min((first_width / ratios[0]).min(second_width / ratios[1]).round())
            .floor();
        vec![
            GridRect::new(0.0, 0.0, first_width, height),
            GridRect::new(first_width + spacing, 0.0, second_width, height),
        ]
    }

    fn three_item_frames(ratios: &[f32], first_is_portrait: bool, limit: (f32, f32), spacing: f32) -> Vec<GridRect> {
        let (limit_width, limit_height) = limit;
        if first_is_portrait {
            let lower_right_height = ((limit_height - spacing) * 0.5)
                .min((ratios[1] * (limit_width - spacing) / (ratios[2] + ratios[1])).round());
            let upper_right_height = limit_height - lower_right_height - spacing;
            let right_width = MINIMUM_MOSAIC_CELL_WIDTH.max(
                ((limit_width - spacing) * 0.5)
                    .min((lower_right_height * ratios[2]).min(upper_right_height * ratios[1]).round()),
            );
            let left_width = (limit_height * ratios[0]).min(limit_width - spacing - right_width).round();
            return vec![
                GridRect::new(0.0, 0.0, left_width, limit_height),
                GridRect::new(left_width + spacing, 0.0, right_width, upper_right_height),
                GridRect::new(left_width + spacing, upper_right_height + spacing, right_width, lower_right_height),
            ];
        }

        let top_height = (limit_width / ratios[0]).min((limit_height - spacing) * 0.66).floor();
        let lower_width = (limit_width - spacing) / 2.0;
        let lower_height = (limit_height - top_height - spacing)
            .min((lower_width / ratios[1]).min(lower_width / ratios[2]).round());
        vec![
            GridRect::new(0.0, 0.0, limit_width, top_height),
            GridRect::new(0.0, top_height + spacing, lower_width, lower_height),
            GridRect::new(lower_width + spacing, top_height + spacing, lower_width, lower_height),
        ]
    }

    fn four_item_frames(ratios: &[f32], first_is_landscape: bool, limit: (f32, f32), spacing: f32) -> Vec<GridRect> {
        let (limit_width, limit_height) = limit;
        if first_is_landscape {
            let top_height = (limit_width / ratios[0]).min((limit_height - spacing) * 0.66).round();
            let mut lower_height = ((limit_width - 2.0 * spacing) / (ratios[1] + ratios[2] + ratios[3])).round();
            let left_width = MINIMUM_MOSAIC_CELL_WIDTH
                .max(((limit_width - 2.0 * spacing) * 0.4).min(lower_height * ratios[1]));
            let right_width = MINIMUM_MOSAIC_CELL_WIDTH
                .max((limit_width - 2.0 * spacing) * 0.33)
                .max(lower_height * ratios[3]);
            let middle_width = limit_width - left_width - right_width - 2.0 * spacing;
            lower_height = MINIMUM_MOSAIC_CELL_HEIGHT.max((limit_height - top_height - spacing).min(lower_height));
            let lower_y = top_height + spacing;
            return vec![
                GridRect::new(0.0, 0.0, limit_width, top_height),
                GridRect::new(0.0, lower_y, left_width, lower_height),
                GridRect::new(left_width + spacing, lower_y, middle_width, lower_height),
                GridRect::new(left_width + middle_width + 2.0 * spacing, lower_y, right_width, lower_height),
            ];
        }

        let left_width = (limit_height * ratios[0]).min((limit_width - spacing) * 0.6).round();
        let mut right_width =
            ((limit_height - 2.0 * spacing) / (1.0 / ratios[1] + 1.0 / ratios[2] + 1.0 / ratios[3])).round();
        let first_right_height = (right_width / ratios[1]).floor();
        let second_right_height = (right_width / ratios[2]).floor();
        let third_right_height = limit_height - first_right_height - second_right_height - 2.0 * spacing;
        right_width = MINIMUM_MOSAIC_CELL_WIDTH.max((limit_width - left_width - spacing).min(right_width));
        let right_x = left_width + spacing;
        vec![
            GridRect::new(0.0, 0.0, left_width, limit_height),
            GridRect::new(right_x, 0.0, right_width, first_right_height),
            GridRect::new(right_x, first_right_height + spacing, right_width, second_right_height),
            GridRect::new(
                right_x,
                first_right_height + second_right_height + 2.0 * spacing,
                right_width,
                third_right_height,
            ),
        ]
    }

    fn optimized_frames(ratios: &[f32], average: f32, width: f32, spacing: f32) -> Vec<GridRect> {
        let crop_ratios: Vec<f32> = ratios
            .iter()
            .map(|&ratio| {
                let balanced = if average > 1.1 { ratio.max(1.0) } else { ratio.min(1.0) };
                balanced.max(0.66667).min(1.7)
            })
            .collect();
        let attempts = Self::layout_candidates(&crop_ratios, width, spacing, average);
        let target_height = (width / 3.0 * 4.0).floor();
        let best = attempts
            .into_iter()
            .min_by(|lhs, rhs| {
                Self::score(lhs, target_height, spacing)
                    .partial_cmp(&Self::score(rhs, target_height, spacing))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|| Self::fallback_candidate(&crop_ratios, width, spacing));

        let mut frames = vec![GridRect::default(); ratios.len()];
        let mut item_index = 0;
        let mut y = 0.0_f32;
        for (line_index, &item_count) in best.cells_per_line.iter().enumerate() {
            let line_height = best.line_heights[line_index].ceil();
            let mut x = 0.0_f32;
            for _ in 0..item_count {
                if item_index >= frames.len() {
                    continue;
                }
                let cell_width = (crop_ratios[item_index] * line_height).ceil();
                frames[item_index] = GridRect::new(x, y, cell_width, line_height);
                x += cell_width + spacing;
                item_index += 1;
            }
            y += line_height + spacing;
        }

        let mut maximum_line_width = 0.0_f32;
        let mut cursor = 0;
        for &count in &best.cells_per_line {
            if count > 0 && cursor + count - 1 < frames.len() {
                maximum_line_width = maximum_line_width.max(frames[cursor + count - 1].max_x());
            }
            cursor += count;
        }

        cursor = 0;
        for &count in &best.cells_per_line {
            if count > 0 && cursor + count - 1 < frames.len() {
                let final_index = cursor + count - 1;
                frames[final_index].width = frames[final_index]
                    .width
                    .max(maximum_line_width - frames[final_index].x);
            }
            cursor += count;
        }
        frames
    }

    fn layout_candidates(
        ratios: &[f32],
        width: f32,
        spacing: f32,
        average: f32,
    ) -> Vec<MessageGridLayoutEngineMosaicCandidate> {
        let height = |range: std::ops::Range<usize>| -> f32 {
            let line_count = range.len();
            let ratio_sum: f32 = ratios[range].iter().sum();
            (width - (line_count as f32 - 1.0) * spacing) / ratio_sum
        };

        let mut result = Vec::new();
        let count = ratios.len();
        if count < 2 {
            return result;
        }

        for first in 1..count {
            let second = count - first;
            if first > 3 || second > 3 {
                continue;
            }
            result.push(MessageGridLayoutEngineMosaicCandidate {
                cells_per_line: vec![first, second],
                line_heights: vec![height(0..first), height(first..count)],
            });
        }

        if count >= 3 {
            for first in 1..(count - 1) {
                for second in 1..(count - first) {
                    let third = count - first - second;
                    if first > 3 || second > (if average < 0.85 { 4 } else { 3 }) || third > 3 {
                        continue;
                    }
                    result.push(MessageGridLayoutEngineMosaicCandidate {
                        cells_per_line: vec![first, second, third],
                        line_heights: vec![
                            height(0..first),
                            height(first..(first + second)),
                            height((first + second)..count),
                        ],
                    });
                }
            }
        }

        if count >= 4 {
            for first in 1..(count - 2) {
                for second in 1..(count - first - 1) {
                    for third in 1..(count - first - second) {
                        let fourth = count - first - second - third;
                        if first > 3 || second > 3 || third > 3 || fourth > 3 {
                            continue;
                        }
                        let second_start = first;
                        let third_start = second_start + second;
                        let fourth_start = third_start + third;
                        result.push(MessageGridLayoutEngineMosaicCandidate {
                            cells_per_line: vec![first, second, third, fourth],
                            line_heights: vec![
                                height(0..second_start),
                                height(second_start..third_start),
                                height(third_start..fourth_start),
                                height(fourth_start..count),
                            ],
                        });
                    }
                }
            }
        }
        result
    }

    fn fallback_candidate(ratios: &[f32], width: f32, spacing: f32) -> MessageGridLayoutEngineMosaicCandidate {
        let mut counts = Vec::new();
        let mut heights = Vec::new();
        let mut cursor = 0;
        while cursor < ratios.len() {
            let count = 3.min(ratios.len() - cursor);
            let line_ratios: f32 = ratios[cursor..(cursor + count)].iter().sum();
            let height = (width - (count as f32 - 1.0) * spacing) / line_ratios;
            counts.push(count);
            heights.push(height);
            cursor += count;
        }
        MessageGridLayoutEngineMosaicCandidate { cells_per_line: counts, line_heights: heights }
    }

    fn score(candidate: &MessageGridLayoutEngineMosaicCandidate, target_height: f32, spacing: f32) -> f32 {
        let mut progressive_height = spacing * (candidate.line_heights.len() as f32 - 1.0);
        let mut first_progressive_height = f32::MAX;
        for height in &candidate.line_heights {
            progressive_height += height.floor();
            first_progressive_height = first_progressive_height.min(progressive_height);
        }

        let mut value = (progressive_height - target_height).abs();
        if candidate.cells_per_line.windows(2).any(|pair| pair[0] > pair[1]) {
            value *= 1.5;
        }
        if first_progressive_height < MINIMUM_MOSAIC_CELL_WIDTH {
            value *= 1.5;
        }
        value
    }

    fn horizontal_bands(cells: &[MessageGridCell]) -> Vec<MessageGridRow> {
        let mut boundaries: Vec<f32> = cells
            .iter()
            .flat_map(|cell| [f32::from(cell.cell_frame.top()), f32::from(cell.cell_frame.bottom())])
            .collect();
        boundaries.sort_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal));
        boundaries.dedup();
        if boundaries.len() < 2 {
            return Vec::new();
        }

        boundaries
            .windows(2)
            .filter_map(|pair| {
                let (top, bottom) = (pair[0], pair[1]);
                if bottom <= top {
                    return None;
                }
                let indices: Vec<usize> = cells
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| {
                        f32::from(cell.cell_frame.top()) <= top + 0.001
                            && f32::from(cell.cell_frame.bottom()) >= bottom - 0.001
                    })
                    .map(|(index, _)| index)
                    .collect();
                if indices.is_empty() {
                    return None;
                }
                Some(MessageGridRow { top: px(top), height: px(bottom - top), cell_indices: indices })
            })
            .collect()
    }

    fn aspect(item: &MessageMediaItem) -> f32 {
        let ar = item.aspect_ratio();
        if !ar.is_finite() || ar <= 0.001 {
            return 1.0;
        }
        ar.max(0.05).min(20.0)
    }

    fn proportion(ratio: f32) -> MessageGridLayoutEngineProportion {
        if ratio > 1.2 {
            return MessageGridLayoutEngineProportion::Landscape;
        }
        if ratio < 0.8 {
            return MessageGridLayoutEngineProportion::Portrait;
        }
        MessageGridLayoutEngineProportion::Square
    }

    fn aspect_fit_frame(ratio: f32, frame: GridRect) -> GridRect {
        if frame.width <= 0.0 || frame.height <= 0.0 {
            return frame;
        }
        let (width, height) = if ratio > frame.width / frame.height {
            let width = frame.width.floor();
            (width, (width / ratio).floor())
        } else {
            let height = frame.height.floor();
            ((height * ratio).floor(), height)
        };
        GridRect::new(
            frame.x + (frame.width - width) / 2.0,
            frame.y + (frame.height - height) / 2.0,
            width,
            height,
        )
    }
}
