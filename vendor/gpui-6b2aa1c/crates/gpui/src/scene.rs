// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels,
    Point, Radians, ScaledPixels, Size, bounds_tree::BoundsTree, point,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
};

/// The two top bits of a glyph swap key whose glyphs may slide inside their clip (Riseonly 0079):
/// glyphs painted under such a key stay in the scene while their clip hides them, so
/// [`Scene::shift_glyphs`] can bring them in. The other glyph key families keep these bits apart
/// (0056 badge keys set bit 31 only, 0078 clock keys bit 30 only).
pub const GLYPH_SHIFT_KEY_BITS: u32 = 0b11 << 30;

/// Whether `pad` is a glyph swap key whose glyphs may slide (Riseonly 0079).
pub fn is_glyph_shift_key(pad: u32) -> bool {
    pad & GLYPH_SHIFT_KEY_BITS == GLYPH_SHIFT_KEY_BITS
}

/// Bit 29 of a glyph swap key whose SVG icons carry the key too (Riseonly 0097), with both top bits
/// clear: an icon painted under such a key can fade in the frame on screen through
/// [`Scene::fade_glyph_sprites`]. The other glyph key families leave their SVGs unkeyed, so a
/// recoloured or sliding label keeps its icons as painted.
pub const SVG_FADE_KEY_BIT: u32 = 1 << 29;

/// Whether `pad` is a glyph swap key whose SVG icons carry it (Riseonly 0097).
pub fn is_svg_fade_key(pad: u32) -> bool {
    pad & (GLYPH_SHIFT_KEY_BITS | SVG_FADE_KEY_BIT) == SVG_FADE_KEY_BIT
}

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

/// A boolean stored as a `u32` so that GPU-facing structs contain no
/// compiler-inserted padding bytes, which would be undefined behavior to
/// reinterpret as `&[u8]` when writing instance buffers. Guaranteed to be
/// `0` or `1` by construction; shaders read it as a `u32`/`uint`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PaddedBool32(u32);

impl From<bool> for PaddedBool32 {
    fn from(value: bool) -> Self {
        PaddedBool32(value as u32)
    }
}

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
    /// Paint operations painted under a shift key and how far their copies on screen moved
    /// since (Riseonly 0084).
    pub(crate) shift_groups: Vec<ShiftGroup>,
    /// The clips of the cached views painting now, each cut by the ones around it (Riseonly
    /// 0099): the frame draws a primitive cut by the last, while the paint operations keep the
    /// clip it was painted with inside its view.
    pub(crate) clip_stack: Vec<Bounds<ScaledPixels>>,
}

/// The paint operations [`crate::Window::with_shift_group`] recorded under `key`, and how
/// [`Scene::shift_group`] and [`Scene::move_group`] moved, scaled and faded their primitives on
/// screen from the copies kept for replay (Riseonly 0084, 0112).
#[derive(Clone, Debug)]
pub(crate) struct ShiftGroup {
    key: u64,
    operations: Range<usize>,
    motion: GroupMotion,
    /// The clip of the cached view painting the group (Riseonly 0099): it stays while the group
    /// moves.
    outer: Option<Bounds<ScaledPixels>>,
}

/// Where the primitives of a shift group show against the paint kept for replay (Riseonly 0112):
/// a point painted at `p` shows at `p * scale + translation`, every colour times `fade`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroupMotion {
    pub(crate) scale: f32,
    pub(crate) translation: Point<ScaledPixels>,
    pub(crate) fade: f32,
    /// The clips stay where they were painted, as a paint offset inside a fixed mask draws; else
    /// a moved primitive takes its clip along, as a moved replay does.
    pub(crate) clip_stays: bool,
}

impl GroupMotion {
    pub(crate) const IDENTITY: Self = Self {
        scale: 1.0,
        translation: Point { x: ScaledPixels(0.0), y: ScaledPixels(0.0) },
        fade: 1.0,
        clip_stays: false,
    };

    /// `painted` as it shows under this motion, cut by `outer`. None when the motion scales a
    /// primitive the scene can only move (a quad, a shadow, an underline, an untransformed sprite or
    /// a surface): its view then repaints.
    fn show(&self, painted: &Primitive, outer: Option<Bounds<ScaledPixels>>) -> Option<Primitive> {
        let mut shown = painted.clone();
        if self.scale == 1.0 {
            shown.translate(self.translation);
            if self.clip_stays {
                shown.keep_clips_of(painted);
            }
        } else if !shown.scale_by(self.scale, self.translation) {
            return None;
        }
        if self.fade != 1.0 {
            shown.fade(self.fade);
        }
        if let Some(clip) = outer {
            shown.clip_to(&clip);
        }
        Some(shown)
    }
}

/// Replaces the primitive of `list` (sorted by draw order) drawn at `order` with `bounds` by
/// `next` of it (Riseonly 0084). False when none is.
fn shift_live<T>(
    list: &mut [T],
    order: DrawOrder,
    bounds: Bounds<ScaledPixels>,
    of: impl Fn(&T) -> (DrawOrder, Bounds<ScaledPixels>),
    next: impl FnOnce(&T) -> T,
) -> bool {
    let start = list.partition_point(|item| of(item).0 < order);
    for item in &mut list[start..] {
        let (item_order, item_bounds) = of(item);
        if item_order != order {
            break;
        }
        if item_bounds == bounds {
            *item = next(item);
            return true;
        }
    }
    false
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
        self.shift_groups.clear();
        self.clip_stack.clear();
    }

    /// Names the paint operations from `start` to now `key` (Riseonly 0084). Zero names nothing.
    pub(crate) fn close_shift_group(&mut self, key: u64, start: usize) {
        let end = self.paint_operations.len();
        if key != 0 && end > start {
            self.shift_groups.push(ShiftGroup {
                key,
                operations: start..end,
                motion: GroupMotion::IDENTITY,
                outer: self.clip_stack.last().copied(),
            });
        }
    }

    /// Moves every primitive painted under the shift key `key` by `offset` in the frame on screen,
    /// clip included, as a moved replay does (Riseonly 0084). The copies kept for replay stay where
    /// they were painted, so a later replay at a new spot still moves them from there. False when
    /// no group carries the key or a primitive of it was not found on screen; the caller then
    /// repaints through its view.
    pub fn shift_group(&mut self, key: u64, offset: Point<ScaledPixels>) -> bool {
        let Some(index) = self.shift_groups.iter().rposition(|group| group.key == key) else {
            return false;
        };
        if offset.x.0 == 0.0 && offset.y.0 == 0.0 {
            return true;
        }
        let motion = self.shift_groups[index].motion;
        self.move_group_at(index, GroupMotion { translation: motion.translation + offset, ..motion })
    }

    /// Shows every primitive painted under the shift key `key` scaled by `scale`, moved by
    /// `translation` and faded by `fade` against its paint, clip of the view around it kept
    /// (Riseonly 0112): a point painted at `p` lands at `p * scale + translation`. The copies kept
    /// for replay stay as painted, so each call starts from the paint, and a fade back from zero
    /// still finds its colours. Scaling takes only paths and transformed sprites, what a paint
    /// under a transformation records. False, changing nothing, when no group carries the key or
    /// the motion cannot show a primitive of it; false when a primitive was not found on screen.
    /// The caller then repaints through its view.
    pub(crate) fn move_group(&mut self, key: u64, motion: GroupMotion) -> bool {
        match self.shift_groups.iter().rposition(|group| group.key == key) {
            Some(index) => self.move_group_at(index, motion),
            None => false,
        }
    }

    fn move_group_at(&mut self, index: usize, motion: GroupMotion) -> bool {
        let group = self.shift_groups[index].clone();
        if group.motion == motion {
            return true;
        }
        let mut moves = Vec::new();
        for operation in &self.paint_operations[group.operations.clone()] {
            let PaintOperation::Primitive(painted) = operation else {
                continue;
            };
            let (Some(shown), Some(next)) = (group.motion.show(painted, None), motion.show(painted, group.outer))
            else {
                return false;
            };
            moves.push((shown, next));
        }
        let mut found = true;
        for (shown, next) in moves {
            found &= match (shown, next) {
                (Primitive::Shadow(shown), Primitive::Shadow(next)) => {
                    shift_live(&mut self.shadows, shown.order, shown.bounds, |item| (item.order, item.bounds), |_| next)
                }
                (Primitive::Quad(shown), Primitive::Quad(next)) => {
                    shift_live(&mut self.quads, shown.order, shown.bounds, |item| (item.order, item.bounds), |_| next)
                }
                (Primitive::Path(shown), Primitive::Path(next)) => shift_live(
                    &mut self.paths,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |live| {
                        let mut next = next;
                        next.id = live.id;
                        next
                    },
                ),
                (Primitive::Underline(shown), Primitive::Underline(next)) => shift_live(
                    &mut self.underlines,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |_| next,
                ),
                (Primitive::MonochromeSprite(shown), Primitive::MonochromeSprite(next)) => shift_live(
                    &mut self.monochrome_sprites,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |_| next,
                ),
                (Primitive::SubpixelSprite(shown), Primitive::SubpixelSprite(next)) => shift_live(
                    &mut self.subpixel_sprites,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |_| next,
                ),
                (Primitive::PolychromeSprite(shown), Primitive::PolychromeSprite(next)) => shift_live(
                    &mut self.polychrome_sprites,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |_| next,
                ),
                (Primitive::Surface(shown), Primitive::Surface(next)) => shift_live(
                    &mut self.surfaces,
                    shown.order,
                    shown.bounds,
                    |item| (item.order, item.bounds),
                    |_| next,
                ),
                _ => false,
            };
        }
        self.shift_groups[index].motion = motion;
        found
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    /// Opens the clip boundary of a cached view (Riseonly 0099): `clip` is the mask around the view
    /// as painted inside the boundary around it, and every primitive painted until
    /// [`Self::pop_clip`] is drawn cut by it and by the clips around it. A replay of the paint at
    /// another origin moves `clip` with it, while the clips around stay those of the frame.
    pub(crate) fn push_clip(&mut self, clip: Bounds<ScaledPixels>) {
        let shown = match self.clip_stack.last() {
            Some(outer) => outer.intersect(&clip),
            None => clip,
        };
        self.clip_stack.push(shown);
        self.paint_operations.push(PaintOperation::PushClip(clip));
    }

    /// Closes the clip boundary [`Self::push_clip`] opened (Riseonly 0099).
    pub(crate) fn pop_clip(&mut self) {
        self.clip_stack.pop();
        self.paint_operations.push(PaintOperation::PopClip);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        // Riseonly 0099: under the clip boundary of a cached view the frame draws the primitive cut
        // by it, while the paint operations keep the primitive as painted, even when cut away
        // whole, for a replay at another origin or under another clip to cut again.
        if let Some(clip) = self.clip_stack.last().copied() {
            let mut shown = primitive.clone();
            shown.clip_to(&clip);
            if let Some(shown) = self.insert_shown(shown) {
                primitive.take_slot(&shown);
            }
            self.paint_operations.push(PaintOperation::Primitive(primitive));
            return;
        }
        if let Some(primitive) = self.insert_shown(primitive) {
            self.paint_operations.push(PaintOperation::Primitive(primitive));
        }
    }

    /// Puts `primitive` in the lists drawn this frame and gives it back with its draw order, or
    /// nothing when its clip cuts it away whole.
    fn insert_shown(&mut self, mut primitive: Primitive) -> Option<Primitive> {
        let mut clipped_bounds = primitive
            .painted_bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            // A sliding glyph hidden by its clip now may slide into it later (0079): it is kept and
            // ordered by its clip, the place it can show up in.
            let slides = match &primitive {
                Primitive::MonochromeSprite(sprite) => is_glyph_shift_key(sprite.pad),
                Primitive::SubpixelSprite(sprite) => is_glyph_shift_key(sprite.pad),
                Primitive::PolychromeSprite(sprite) => is_glyph_shift_key(sprite.pad),
                _ => false,
            };
            if !slides || primitive.content_mask().bounds.is_empty() {
                return None;
            }
            clipped_bounds = primitive.content_mask().bounds;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        Some(primitive)
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        // Riseonly 0112: a whole replay of a cached view keeps its shift groups and shows them
        // where they were moved, so an ancestor redrawing around a row riding a motion neither
        // snaps it back to its paint nor makes the next move refuse.
        let groups: Vec<&ShiftGroup> = prev_scene
            .shift_groups
            .iter()
            .filter(|group| range.start <= group.operations.start && group.operations.end <= range.end)
            .collect();
        if groups.is_empty() {
            self.replay_operations(&prev_scene.paint_operations[range]);
            return;
        }
        let mut starts = vec![0; groups.len()];
        for (at, operation) in prev_scene.paint_operations[range.clone()].iter().enumerate() {
            let at = range.start + at;
            for (start, group) in starts.iter_mut().zip(&groups) {
                if group.operations.start == at {
                    *start = self.paint_operations.len();
                }
            }
            let moved = groups
                .iter()
                .rev()
                .find(|group| group.operations.contains(&at) && group.motion != GroupMotion::IDENTITY);
            match (operation, moved) {
                (PaintOperation::Primitive(painted), Some(group)) => match group.motion.show(painted, group.outer) {
                    Some(shown) => self.insert_primitive_shown(painted.clone(), shown),
                    None => self.insert_primitive(painted.clone()),
                },
                (operation, _) => self.replay_operations(std::slice::from_ref(operation)),
            }
            for (start, group) in starts.iter().zip(&groups) {
                if group.operations.end == at + 1 && self.paint_operations.len() > *start {
                    self.shift_groups.push(ShiftGroup {
                        operations: *start..self.paint_operations.len(),
                        ..(*group).clone()
                    });
                }
            }
        }
    }

    /// Puts `shown`, a moved copy of `painted`, in the lists drawn this frame and keeps `painted`
    /// for replay (Riseonly 0112), as [`Self::insert_primitive`] keeps a cut one.
    fn insert_primitive_shown(&mut self, mut painted: Primitive, mut shown: Primitive) {
        if let Some(clip) = self.clip_stack.last().copied() {
            shown.clip_to(&clip);
        }
        if let Some(shown) = self.insert_shown(shown) {
            painted.take_slot(&shown);
        }
        self.paint_operations.push(PaintOperation::Primitive(painted));
    }

    /// Replays operations kept out of the frames, the paint of a parked cached view (Riseonly 0034).
    pub(crate) fn replay_operations(&mut self, operations: &[PaintOperation]) {
        for operation in operations {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::PushClip(clip) => self.push_clip(*clip),
                PaintOperation::PopClip => self.pop_clip(),
            }
        }
    }

    /// Replays a range of the previous scene moved by `offset`: the paint of a cached view that
    /// only changed its origin, a cell scrolled within its clip (Riseonly 0041).
    pub(crate) fn replay_moved(
        &mut self,
        range: Range<usize>,
        prev_scene: &Scene,
        offset: Point<ScaledPixels>,
    ) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => {
                    let mut primitive = primitive.clone();
                    primitive.translate(offset);
                    self.insert_primitive(primitive);
                }
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds + offset),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::PushClip(clip) => self.push_clip(*clip + offset),
                PaintOperation::PopClip => self.pop_clip(),
            }
        }
    }

    /// Replays a range of the previous scene moved by `offset` under another element opacity
    /// (Riseonly 0085): a cached row riding a fade keeps its frame instead of rendering.
    pub(crate) fn replay_moved_faded(
        &mut self,
        range: Range<usize>,
        prev_scene: &Scene,
        offset: Point<ScaledPixels>,
        factor: f32,
    ) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => {
                    let mut primitive = primitive.clone();
                    primitive.translate(offset);
                    primitive.fade(factor);
                    self.insert_primitive(primitive);
                }
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds + offset),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::PushClip(clip) => self.push_clip(*clip + offset),
                PaintOperation::PopClip => self.pop_clip(),
            }
        }
    }

    /// Replays a range of the previous scene in place under another element opacity (Riseonly
    /// 0065): each colour takes `factor`, the new opacity over the one it was painted with.
    pub(crate) fn replay_faded(&mut self, range: Range<usize>, prev_scene: &Scene, factor: f32) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => {
                    let mut primitive = primitive.clone();
                    primitive.fade(factor);
                    self.insert_primitive(primitive);
                }
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::PushClip(clip) => self.push_clip(*clip),
                PaintOperation::PopClip => self.pop_clip(),
            }
        }
    }

    /// Replays a range of the previous scene painted under one transformation as the paint under
    /// another (Riseonly 0053): a cached view flying in a zoom transition keeps its last frame and
    /// takes the matrix and the clip of this one instead of rendering. Layers keep their bounds,
    /// which only order what they hold.
    pub(crate) fn replay_transformed(
        &mut self,
        range: Range<usize>,
        prev_scene: &Scene,
        replay: &TransformedReplay,
    ) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => {
                    let mut primitive = primitive.clone();
                    primitive.retransform(replay);
                    self.insert_primitive(primitive);
                }
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
                // Riseonly 0104: a clip painted under the old matrix (a scroll clip inside a
                // scaling modal) follows the content through the delta, inside the new mask.
                PaintOperation::PushClip(clip) => self.push_clip(replay.follow_clip(clip)),
                PaintOperation::PopClip => self.pop_clip(),
            }
        }
    }

    /// Gives every surface painted with `swap_key` a new picture in place (Riseonly 0046): a video
    /// frame reaches the screen with a present of this scene, without a render, layout or paint of
    /// the views around it. The copies kept for replay take it too. False when no surface of the
    /// scene carries the key (its element left the scene or was clipped away).
    pub fn swap_surface(&mut self, swap_key: u64, mut update: impl FnMut(&mut PaintSurface)) -> bool {
        if swap_key == 0 {
            return false;
        }
        let mut found = false;
        for surface in self.surfaces.iter_mut().filter(|surface| surface.swap_key == swap_key) {
            update(surface);
            found = true;
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::Surface(surface)) = operation
                && surface.swap_key == swap_key
            {
                update(surface);
            }
        }
        found
    }

    /// Gives every path painted with `swap_key` the geometry of `local` in place (Riseonly 0047):
    /// `local` is laid out from the anchor the path was painted with, so a spinner turns with a
    /// present of this scene, without a render, layout or paint of the views around it. Order,
    /// content mask and colour stay. The copies kept for replay take it too. False when no path of
    /// the scene carries the key.
    pub fn swap_path(&mut self, swap_key: u64, local: &Path<ScaledPixels>) -> bool {
        if swap_key == 0 {
            return false;
        }
        let mut found = false;
        for path in self.paths.iter_mut().filter(|path| path.swap_key == swap_key) {
            path.take_local_geometry(local);
            found = true;
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::Path(path)) = operation
                && path.swap_key == swap_key
            {
                path.take_local_geometry(local);
            }
        }
        found
    }

    /// [`Self::swap_path`] for many keys in one pass over the scene, each path also taking the
    /// colour of its swap, under the element opacity it was painted with (Riseonly 0049): a field
    /// of twinkling sparkles moves and fades with a present. True only when every nonzero key was
    /// found; the paths that were found keep the swap either way.
    pub fn swap_paths(&mut self, swaps: &[(u64, Path<ScaledPixels>, Background)]) -> bool {
        let slots: std::collections::HashMap<u64, usize> = swaps
            .iter()
            .enumerate()
            .filter(|(_, swap)| swap.0 != 0)
            .map(|(slot, swap)| (swap.0, slot))
            .collect();
        if slots.is_empty() {
            return false;
        }
        let mut found = vec![false; swaps.len()];
        for path in &mut self.paths {
            if path.swap_key != 0
                && let Some(&slot) = slots.get(&path.swap_key)
            {
                let (_, local, color) = &swaps[slot];
                path.take_local_geometry(local);
                path.color = color.opacity(path.swap_opacity);
                found[slot] = true;
            }
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::Path(path)) = operation
                && path.swap_key != 0
                && let Some(&slot) = slots.get(&path.swap_key)
            {
                let (_, local, color) = &swaps[slot];
                path.take_local_geometry(local);
                path.color = color.opacity(path.swap_opacity);
            }
        }
        slots.values().all(|&slot| found[slot])
    }

    /// Recolours in place every glyph sprite carrying `key` in its pad (Riseonly 0056): `colour`
    /// gets the place of the glyph centre across the span of all glyphs of the key, 0 at the
    /// left edge and 1 at the right; the sprite keeps its alpha. The copies kept for replay take
    /// it too. False when no glyph carries the key.
    pub fn swap_glyph_colours(&mut self, key: u32, colour: &dyn Fn(f32) -> Hsla) -> bool {
        if key == 0 {
            return false;
        }
        let mut span: Option<(f32, f32)> = None;
        let mut widen = |bounds: &Bounds<ScaledPixels>| {
            let (left, right) = (bounds.origin.x.0, bounds.origin.x.0 + bounds.size.width.0);
            span = Some(match span {
                Some((min, max)) => (min.min(left), max.max(right)),
                None => (left, right),
            });
        };
        for sprite in self.monochrome_sprites.iter().filter(|sprite| sprite.pad == key) {
            widen(&sprite.bounds);
        }
        for sprite in self.subpixel_sprites.iter().filter(|sprite| sprite.pad == key) {
            widen(&sprite.bounds);
        }
        let Some((min, max)) = span else {
            return false;
        };
        let width = (max - min).max(1.0);
        let recolour = |bounds: &Bounds<ScaledPixels>, color: &mut Hsla| {
            let place = ((bounds.origin.x.0 + bounds.size.width.0 / 2.0 - min) / width).clamp(0.0, 1.0);
            let alpha = color.a;
            *color = colour(place);
            color.a = alpha;
        };
        for sprite in self.monochrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            recolour(&sprite.bounds, &mut sprite.color);
        }
        for sprite in self.subpixel_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            recolour(&sprite.bounds, &mut sprite.color);
        }
        for operation in &mut self.paint_operations {
            match operation {
                PaintOperation::Primitive(Primitive::MonochromeSprite(sprite)) if sprite.pad == key => {
                    recolour(&sprite.bounds, &mut sprite.color);
                }
                PaintOperation::Primitive(Primitive::SubpixelSprite(sprite)) if sprite.pad == key => {
                    recolour(&sprite.bounds, &mut sprite.color);
                }
                _ => {}
            }
        }
        true
    }

    /// Shows the glyphs of the `shown` keys and hides those of the `hidden` keys in the frame on
    /// screen and in the copies kept for replay (Riseonly 0078): every glyph keeps its colour, a
    /// hidden one takes alpha zero and a shown one the highest alpha any glyph of these keys was
    /// painted with, so a label painted as stacked candidates (one digit shown per slot) changes
    /// with a present. False, with nothing changed, when a key is zero or no glyph carries it.
    pub fn swap_glyph_visibility(&mut self, shown: &[u32], hidden: &[u32]) -> bool {
        let carries = |key: u32| {
            key != 0
                && (self.monochrome_sprites.iter().any(|sprite| sprite.pad == key)
                    || self.subpixel_sprites.iter().any(|sprite| sprite.pad == key))
        };
        if !shown.iter().chain(hidden.iter()).all(|key| carries(*key)) {
            return false;
        }
        let in_set = |pad: u32| pad != 0 && (shown.contains(&pad) || hidden.contains(&pad));
        let mut alpha: f32 = 0.0;
        for sprite in self.monochrome_sprites.iter().filter(|sprite| in_set(sprite.pad)) {
            alpha = alpha.max(sprite.color.a);
        }
        for sprite in self.subpixel_sprites.iter().filter(|sprite| in_set(sprite.pad)) {
            alpha = alpha.max(sprite.color.a);
        }
        let apply = |pad: u32, color: &mut Hsla| {
            if shown.contains(&pad) {
                color.a = alpha;
            } else if hidden.contains(&pad) {
                color.a = 0.0;
            }
        };
        for sprite in self.monochrome_sprites.iter_mut().filter(|sprite| sprite.pad != 0) {
            apply(sprite.pad, &mut sprite.color);
        }
        for sprite in self.subpixel_sprites.iter_mut().filter(|sprite| sprite.pad != 0) {
            apply(sprite.pad, &mut sprite.color);
        }
        for operation in &mut self.paint_operations {
            match operation {
                PaintOperation::Primitive(Primitive::MonochromeSprite(sprite)) if sprite.pad != 0 => {
                    apply(sprite.pad, &mut sprite.color);
                }
                PaintOperation::Primitive(Primitive::SubpixelSprite(sprite)) if sprite.pad != 0 => {
                    apply(sprite.pad, &mut sprite.color);
                }
                _ => {}
            }
        }
        true
    }

    /// Moves every glyph painted under the sliding key `key` (see [`GLYPH_SHIFT_KEY_BITS`]) by
    /// `dx` inside its clip, in the frame on screen and in the copies kept for replay (Riseonly
    /// 0079): a marquee title slides with a present. None, with nothing changed, when no glyph
    /// carries the key; otherwise whether any glyph of the key shows inside its clip before or
    /// after the move (the frame needs a present).
    pub fn shift_glyphs(&mut self, key: u32, dx: ScaledPixels) -> Option<bool> {
        if !is_glyph_shift_key(key) {
            return None;
        }
        let carries = self.monochrome_sprites.iter().any(|sprite| sprite.pad == key)
            || self.subpixel_sprites.iter().any(|sprite| sprite.pad == key)
            || self.polychrome_sprites.iter().any(|sprite| sprite.pad == key);
        if !carries {
            return None;
        }
        let offset = point(dx, ScaledPixels(0.0));
        let mut shows = false;
        let mut slide = |primitive: Primitive| -> Primitive {
            let showing = |primitive: &Primitive| {
                !primitive.painted_bounds().intersect(&primitive.content_mask().bounds).is_empty()
            };
            shows |= showing(&primitive);
            let mut primitive = primitive;
            primitive.slide_in_clip(offset);
            shows |= showing(&primitive);
            primitive
        };
        for sprite in self.monochrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            if let Primitive::MonochromeSprite(moved) = slide(Primitive::MonochromeSprite(*sprite)) {
                *sprite = moved;
            }
        }
        for sprite in self.subpixel_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            if let Primitive::SubpixelSprite(moved) = slide(Primitive::SubpixelSprite(*sprite)) {
                *sprite = moved;
            }
        }
        for sprite in self.polychrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            if let Primitive::PolychromeSprite(moved) = slide(Primitive::PolychromeSprite(*sprite)) {
                *sprite = moved;
            }
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(primitive) = operation {
                let carries_key = match primitive {
                    Primitive::MonochromeSprite(sprite) => sprite.pad == key,
                    Primitive::SubpixelSprite(sprite) => sprite.pad == key,
                    Primitive::PolychromeSprite(sprite) => sprite.pad == key,
                    _ => false,
                };
                if carries_key {
                    primitive.slide_in_clip(offset);
                }
            }
        }
        Some(shows)
    }

    /// Moves every image sprite carrying `key` in its pad from the atlas tile `previous` to the same
    /// place in `next` (Riseonly 0057): an animated sticker or GIF shows its next frame with a
    /// present, without a render, layout or paint of the views around it. A sprite showing a cut
    /// of its picture keeps that cut. The copies kept for replay take it too. False, with nothing
    /// changed, when no sprite carries the key, one of them shows another tile than `previous`, or
    /// the two tiles differ in size.
    pub fn swap_image_tile(&mut self, key: u32, previous: AtlasTile, next: AtlasTile) -> bool {
        if key == 0 || previous.bounds.size != next.bounds.size {
            return false;
        }
        let shows_previous = |sprite: &PolychromeSprite| {
            sprite.tile.texture_id == previous.texture_id && sprite.tile.tile_id == previous.tile_id
        };
        let moved = |tile: AtlasTile| AtlasTile {
            bounds: Bounds {
                origin: point(
                    crate::DevicePixels(next.bounds.origin.x.0 + tile.bounds.origin.x.0 - previous.bounds.origin.x.0),
                    crate::DevicePixels(next.bounds.origin.y.0 + tile.bounds.origin.y.0 - previous.bounds.origin.y.0),
                ),
                size: tile.bounds.size,
            },
            ..next
        };
        let mut found = false;
        for sprite in self.polychrome_sprites.iter().filter(|sprite| sprite.pad == key) {
            if !shows_previous(sprite) {
                return false;
            }
            found = true;
        }
        if !found {
            return false;
        }
        for sprite in self.polychrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            sprite.tile = moved(sprite.tile);
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::PolychromeSprite(sprite)) = operation
                && sprite.pad == key
                && shows_previous(&*sprite)
            {
                sprite.tile = moved(sprite.tile);
            }
        }
        true
    }

    /// Scales the alpha of every monochrome sprite (glyph or SVG icon) painted under `key` by
    /// `factor`, in the scene and in the paint operations kept for replay (Riseonly 0097). False
    /// when no sprite carries the key.
    pub fn fade_glyph_sprites(&mut self, key: u32, factor: f32) -> bool {
        if key == 0 || !factor.is_finite() || factor < 0.0 {
            return false;
        }
        let mut found = false;
        for sprite in self.monochrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            sprite.color.a = (sprite.color.a * factor).min(1.0);
            found = true;
        }
        if !found {
            return false;
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::MonochromeSprite(sprite)) = operation
                && sprite.pad == key
            {
                sprite.color.a = (sprite.color.a * factor).min(1.0);
            }
        }
        true
    }

    /// Scales the opacity of every image sprite painted under `key` by `factor`, in the scene and
    /// in the paint operations kept for replay (Riseonly 0072). False when no sprite carries the key.
    pub fn fade_image_sprites(&mut self, key: u32, factor: f32) -> bool {
        if key == 0 || !factor.is_finite() || factor < 0.0 {
            return false;
        }
        let mut found = false;
        for sprite in self.polychrome_sprites.iter_mut().filter(|sprite| sprite.pad == key) {
            sprite.opacity = (sprite.opacity * factor).min(1.0);
            found = true;
        }
        if !found {
            return false;
        }
        for operation in &mut self.paint_operations {
            if let PaintOperation::Primitive(Primitive::PolychromeSprite(sprite)) = operation
                && sprite.pad == key
            {
                sprite.opacity = (sprite.opacity * factor).min(1.0);
            }
        }
        true
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces.sort_by_key(|surface| surface.order);
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
}

#[derive(Clone)]
pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
    /// Riseonly 0099: the clip boundary of a cached view, see [`Scene::push_clip`].
    PushClip(Bounds<ScaledPixels>),
    PopClip,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    /// Where the primitive lands on screen: a sprite's `bounds` are the ones before its
    /// transformation, while the shaders clip it after, so culling and draw order must not use them.
    pub(crate) fn painted_bounds(&self) -> Bounds<ScaledPixels> {
        match self {
            Primitive::MonochromeSprite(sprite) => {
                transformed_bounds(&sprite.bounds, &sprite.transformation)
            }
            Primitive::SubpixelSprite(sprite) => {
                transformed_bounds(&sprite.bounds, &sprite.transformation)
            }
            Primitive::PolychromeSprite(sprite) => {
                transformed_bounds(&sprite.bounds, &sprite.transformation)
            }
            primitive => *primitive.bounds(),
        }
    }

    /// Carries a primitive painted under the old transformation to the new one (Riseonly 0053), as
    /// `Window::with_transformation` would have painted it: paths and transformed sprites take the
    /// delta, a surface its box, the rest stays where layout put it; every clip becomes the mask
    /// the transformation clips with now, the only mask a paint under it records.
    fn retransform(&mut self, replay: &TransformedReplay) {
        let delta = &replay.delta;
        let map = |position: Point<ScaledPixels>| {
            point(
                ScaledPixels(delta.rotation_scale[0][0] * position.x.0 + delta.translation[0]),
                ScaledPixels(delta.rotation_scale[1][1] * position.y.0 + delta.translation[1]),
            )
        };
        let map_bounds = |bounds: &Bounds<ScaledPixels>| {
            Bounds::from_corners(map(bounds.origin), map(bounds.bottom_right()))
        };
        let snapped = |bounds: Bounds<ScaledPixels>| {
            let left = bounds.origin.x.0.round();
            let top = bounds.origin.y.0.round();
            let right = bounds.bottom_right().x.0.round().max(left);
            let bottom = bounds.bottom_right().y.0.round().max(top);
            Bounds::from_corners(
                point(ScaledPixels(left), ScaledPixels(top)),
                point(ScaledPixels(right), ScaledPixels(bottom)),
            )
        };
        let retransformed = |transformation: &mut TransformationMatrix| {
            // invariant: a sprite left untransformed under the old matrix stays so under the new.
            if *transformation != TransformationMatrix::unit() {
                *transformation = delta.compose(*transformation);
            }
        };
        if replay.fade != 1.0 {
            self.fade(replay.fade);
        }
        match self {
            Primitive::Shadow(shadow) => shadow.content_mask = replay.snapped_mask.clone(),
            Primitive::Quad(quad) => quad.content_mask = replay.snapped_mask.clone(),
            Primitive::Underline(underline) => underline.content_mask = replay.snapped_mask.clone(),
            Primitive::Path(path) => {
                path.bounds = map_bounds(&path.bounds);
                path.content_mask = replay.path_mask.clone();
                for vertex in &mut path.vertices {
                    vertex.xy_position = map(vertex.xy_position);
                    vertex.content_mask = replay.path_mask.clone();
                }
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.content_mask = replay.snapped_mask.clone();
                retransformed(&mut sprite.transformation);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.content_mask = replay.snapped_mask.clone();
                retransformed(&mut sprite.transformation);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.content_mask = replay.snapped_mask.clone();
                retransformed(&mut sprite.transformation);
            }
            Primitive::Surface(surface) => {
                surface.bounds = snapped(map_bounds(&surface.bounds));
                if surface.corner_clip_bounds != Bounds::default() {
                    surface.corner_clip_bounds = snapped(map_bounds(&surface.corner_clip_bounds));
                }
                surface.content_mask = replay.snapped_mask.clone();
            }
        }
    }

    /// Multiplies the element opacity the primitive was painted under by `factor` (Riseonly 0059),
    /// above 1 too: a colour painted at alpha `a * o` takes `a * o * factor`. A surface carries no
    /// element opacity.
    fn fade(&mut self, factor: f32) {
        fn faded(color: &mut Hsla, factor: f32) {
            color.a = (color.a * factor).clamp(0.0, 1.0);
        }
        fn faded_background(background: &mut Background, factor: f32) {
            faded(&mut background.solid, factor);
            for stop in &mut background.colors {
                faded(&mut stop.color, factor);
            }
        }
        match self {
            Primitive::Shadow(shadow) => faded(&mut shadow.color, factor),
            Primitive::Quad(quad) => {
                faded_background(&mut quad.background, factor);
                faded(&mut quad.border_color, factor);
            }
            Primitive::Underline(underline) => faded(&mut underline.color, factor),
            Primitive::Path(path) => {
                faded_background(&mut path.color, factor);
                path.swap_opacity = (path.swap_opacity * factor).clamp(0.0, 1.0);
            }
            Primitive::MonochromeSprite(sprite) => faded(&mut sprite.color, factor),
            Primitive::SubpixelSprite(sprite) => faded(&mut sprite.color, factor),
            Primitive::PolychromeSprite(sprite) => {
                sprite.opacity = (sprite.opacity * factor).clamp(0.0, 1.0);
            }
            Primitive::Surface(_) => {}
        }
    }

    /// Moves the primitive, its clip and its transformation by `offset` (Riseonly 0041). The clip
    /// moves along: a moved replay only runs while the view sits wholly inside the outer clip, so
    /// every clip the primitive keeps is the view's own.
    fn translate(&mut self, offset: Point<ScaledPixels>) {
        fn moved(bounds: &mut Bounds<ScaledPixels>, offset: Point<ScaledPixels>) {
            bounds.origin = bounds.origin + offset;
        }
        // A sprite transformation maps absolute points (R * p + t): moving the sprite by d keeps
        // its picture when t becomes t + d - R * d.
        fn moved_transformation(
            transformation: &mut TransformationMatrix,
            offset: Point<ScaledPixels>,
        ) {
            let r = transformation.rotation_scale;
            let (x, y) = (offset.x.0, offset.y.0);
            transformation.translation[0] += x - (r[0][0] * x + r[0][1] * y);
            transformation.translation[1] += y - (r[1][0] * x + r[1][1] * y);
        }
        match self {
            Primitive::Shadow(shadow) => {
                moved(&mut shadow.bounds, offset);
                moved(&mut shadow.element_bounds, offset);
                moved(&mut shadow.content_mask.bounds, offset);
            }
            Primitive::Quad(quad) => {
                moved(&mut quad.bounds, offset);
                moved(&mut quad.content_mask.bounds, offset);
            }
            Primitive::Path(path) => {
                moved(&mut path.bounds, offset);
                moved(&mut path.content_mask.bounds, offset);
                path.swap_anchor = path.swap_anchor + offset;
                for vertex in &mut path.vertices {
                    vertex.xy_position = vertex.xy_position + offset;
                    moved(&mut vertex.content_mask.bounds, offset);
                }
            }
            Primitive::Underline(underline) => {
                moved(&mut underline.bounds, offset);
                moved(&mut underline.content_mask.bounds, offset);
            }
            Primitive::MonochromeSprite(sprite) => {
                moved(&mut sprite.bounds, offset);
                moved(&mut sprite.content_mask.bounds, offset);
                moved_transformation(&mut sprite.transformation, offset);
            }
            Primitive::SubpixelSprite(sprite) => {
                moved(&mut sprite.bounds, offset);
                moved(&mut sprite.content_mask.bounds, offset);
                moved_transformation(&mut sprite.transformation, offset);
            }
            Primitive::PolychromeSprite(sprite) => {
                moved(&mut sprite.bounds, offset);
                moved(&mut sprite.content_mask.bounds, offset);
                moved_transformation(&mut sprite.transformation, offset);
            }
            Primitive::Surface(surface) => {
                moved(&mut surface.bounds, offset);
                moved(&mut surface.content_mask.bounds, offset);
                moved(&mut surface.corner_clip_bounds, offset);
            }
        }
    }

    /// Moves a sprite by `offset` while its clip stays where it is (Riseonly 0079): a sliding
    /// glyph moves under a fixed clip. Other primitives do not slide.
    fn slide_in_clip(&mut self, offset: Point<ScaledPixels>) {
        if !matches!(
            self,
            Primitive::MonochromeSprite(_) | Primitive::SubpixelSprite(_) | Primitive::PolychromeSprite(_)
        ) {
            return;
        }
        let mask = self.content_mask().clone();
        self.translate(offset);
        match self {
            Primitive::MonochromeSprite(sprite) => sprite.content_mask = mask,
            Primitive::SubpixelSprite(sprite) => sprite.content_mask = mask,
            Primitive::PolychromeSprite(sprite) => sprite.content_mask = mask,
            _ => {}
        }
    }

    /// Cuts the clip of the primitive by `clip` (Riseonly 0099).
    pub(crate) fn clip_to(&mut self, clip: &Bounds<ScaledPixels>) {
        let mask = match self {
            Primitive::Shadow(shadow) => &mut shadow.content_mask,
            Primitive::Quad(quad) => &mut quad.content_mask,
            Primitive::Path(path) => &mut path.content_mask,
            Primitive::Underline(underline) => &mut underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &mut sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &mut sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &mut sprite.content_mask,
            Primitive::Surface(surface) => &mut surface.content_mask,
        };
        mask.bounds = mask.bounds.intersect(clip);
    }

    /// Gives the primitive back the clips `painted`, the copy it was moved from, kept (Riseonly
    /// 0112).
    fn keep_clips_of(&mut self, painted: &Primitive) {
        match (self, painted) {
            (Primitive::Shadow(this), Primitive::Shadow(painted)) => this.content_mask = painted.content_mask.clone(),
            (Primitive::Quad(this), Primitive::Quad(painted)) => this.content_mask = painted.content_mask.clone(),
            (Primitive::Path(this), Primitive::Path(painted)) => {
                this.content_mask = painted.content_mask.clone();
                for (vertex, kept) in this.vertices.iter_mut().zip(&painted.vertices) {
                    vertex.content_mask = kept.content_mask.clone();
                }
            }
            (Primitive::Underline(this), Primitive::Underline(painted)) => {
                this.content_mask = painted.content_mask.clone()
            }
            (Primitive::MonochromeSprite(this), Primitive::MonochromeSprite(painted)) => {
                this.content_mask = painted.content_mask.clone()
            }
            (Primitive::SubpixelSprite(this), Primitive::SubpixelSprite(painted)) => {
                this.content_mask = painted.content_mask.clone()
            }
            (Primitive::PolychromeSprite(this), Primitive::PolychromeSprite(painted)) => {
                this.content_mask = painted.content_mask.clone()
            }
            (Primitive::Surface(this), Primitive::Surface(painted)) => this.content_mask = painted.content_mask.clone(),
            _ => {}
        }
    }

    /// Scales the primitive by `scale` and moves it by `translation`, a point at `p` to
    /// `p * scale + translation` (Riseonly 0112), as a paint under that transformation would:
    /// paths take their vertices, transformed sprites their matrix; the clip stays. False,
    /// changing nothing, for what a transformation does not scale on screen.
    fn scale_by(&mut self, scale: f32, translation: Point<ScaledPixels>) -> bool {
        let map = |position: Point<ScaledPixels>| {
            point(
                ScaledPixels(scale * position.x.0 + translation.x.0),
                ScaledPixels(scale * position.y.0 + translation.y.0),
            )
        };
        let delta = TransformationMatrix {
            rotation_scale: [[scale, 0.0], [0.0, scale]],
            translation: [translation.x.0, translation.y.0],
        };
        let scaled = |transformation: &mut TransformationMatrix| {
            if *transformation == TransformationMatrix::unit() {
                return false;
            }
            *transformation = delta.compose(*transformation);
            true
        };
        match self {
            Primitive::Path(path) => {
                path.bounds = Bounds::from_corners(map(path.bounds.origin), map(path.bounds.bottom_right()));
                for vertex in &mut path.vertices {
                    vertex.xy_position = map(vertex.xy_position);
                }
                true
            }
            Primitive::MonochromeSprite(sprite) => scaled(&mut sprite.transformation),
            Primitive::SubpixelSprite(sprite) => scaled(&mut sprite.transformation),
            Primitive::PolychromeSprite(sprite) => scaled(&mut sprite.transformation),
            _ => false,
        }
    }


    /// Takes the draw order (and path id) the scene gave `shown`, a cut copy of this primitive
    /// (Riseonly 0099).
    fn take_slot(&mut self, shown: &Primitive) {
        match (self, shown) {
            (Primitive::Shadow(this), Primitive::Shadow(shown)) => this.order = shown.order,
            (Primitive::Quad(this), Primitive::Quad(shown)) => this.order = shown.order,
            (Primitive::Path(this), Primitive::Path(shown)) => {
                this.order = shown.order;
                this.id = shown.id;
            }
            (Primitive::Underline(this), Primitive::Underline(shown)) => this.order = shown.order,
            (Primitive::MonochromeSprite(this), Primitive::MonochromeSprite(shown)) => this.order = shown.order,
            (Primitive::SubpixelSprite(this), Primitive::SubpixelSprite(shown)) => this.order = shown.order,
            (Primitive::PolychromeSprite(this), Primitive::PolychromeSprite(shown)) => this.order = shown.order,
            (Primitive::Surface(this), Primitive::Surface(shown)) => this.order = shown.order,
            _ => {}
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
}

impl PrimitiveBatch {
    #[expect(missing_docs)]
    pub fn label(&self) -> String {
        match self {
            Self::Shadows(range) => format!("shadows ({})", range.len()),
            Self::Quads(range) => format!("quads ({})", range.len()),
            Self::Paths(range) => format!("paths ({})", range.len()),
            Self::Underlines(range) => format!("underlines ({})", range.len()),
            Self::MonochromeSprites { texture_id, range } => {
                format!(
                    "monochrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::SubpixelSprites { texture_id, range } => {
                format!(
                    "subpixel sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::PolychromeSprites { texture_id, range } => {
                format!(
                    "polychrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::Surfaces(range) => format!("surfaces ({})", range.len()),
        }
    }
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    /// 0 = drop shadow (rendered outside the element), 1 = inset shadow (rendered inside).
    pub inset: u32,
    pub pad: u32, // align to 8 bytes
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
    /// No border: the quad blurs what is drawn under it before its own background (Riseonly 0110).
    /// `border_widths.top` holds the gaussian sigma and `border_widths.bottom` the height of the
    /// bottom strip over which the blur thins out to none; `border_color` stays transparent. Paint
    /// it with `Window::paint_backdrop_blur`. Only Metal blurs (`BACKDROP_BLUR_SUPPORTED`); other
    /// renderers draw just the background.
    BackdropBlur = 2,
}

/// Whether the renderer blurs under a quad of `BorderStyle::BackdropBlur` (Riseonly 0110).
pub const BACKDROP_BLUR_SUPPORTED: bool = cfg!(target_os = "macos");

/// How a cached view painted under one transformation replays under another (Riseonly 0053).
pub(crate) struct TransformedReplay {
    /// The new transformation after the inverse of the old one: an axis-aligned scale and a move.
    pub(crate) delta: TransformationMatrix,
    /// The opacity now over the opacity painted with (Riseonly 0059): 1 when it stayed.
    pub(crate) fade: f32,
    /// The mask the new transformation clips with, as a path keeps it.
    pub(crate) path_mask: ContentMask<ScaledPixels>,
    /// The same mask snapped out to device pixels, as every other primitive keeps it.
    pub(crate) snapped_mask: ContentMask<ScaledPixels>,
}

impl TransformedReplay {
    /// `clip`, painted under the old transformation, where the delta takes what it cut, snapped
    /// out to device pixels and kept inside the clip of the new transformation (Riseonly 0104).
    pub(crate) fn follow_clip(&self, clip: &Bounds<ScaledPixels>) -> Bounds<ScaledPixels> {
        let [[scale_x, _], [_, scale_y]] = self.delta.rotation_scale;
        let [move_x, move_y] = self.delta.translation;
        let left = (clip.left().0 * scale_x + move_x).floor();
        let top = (clip.top().0 * scale_y + move_y).floor();
        let right = (clip.right().0 * scale_x + move_x).ceil().max(left);
        let bottom = (clip.bottom().0 * scale_y + move_y).ceil().max(top);
        Bounds::from_corners(
            point(ScaledPixels(left), ScaledPixels(top)),
            point(ScaledPixels(right), ScaledPixels(bottom)),
        )
        .intersect(&self.snapped_mask.bounds)
    }
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

/// The axis-aligned box around `bounds` after `transformation`.
fn transformed_bounds(
    bounds: &Bounds<ScaledPixels>,
    transformation: &TransformationMatrix,
) -> Bounds<ScaledPixels> {
    if *transformation == TransformationMatrix::unit() {
        return *bounds;
    }
    let left = bounds.origin.x.0;
    let top = bounds.origin.y.0;
    let right = left + bounds.size.width.0;
    let bottom = top + bounds.size.height.0;
    let matrix = transformation.rotation_scale;
    let mut min = [f32::MAX, f32::MAX];
    let mut max = [f32::MIN, f32::MIN];
    for (x, y) in [(left, top), (right, top), (right, bottom), (left, bottom)] {
        for axis in 0..2 {
            let value =
                transformation.translation[axis] + matrix[axis][0] * x + matrix[axis][1] * y;
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    Bounds {
        origin: point(ScaledPixels(min[0]), ScaledPixels(min[1])),
        size: Size {
            width: ScaledPixels(max[0] - min[0]),
            height: ScaledPixels(max[1] - min[1]),
        },
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    /// The rounded box of the innermost clipping container with corner radii (a circle for
    /// `rounded_full().overflow_hidden()`): the surface is cut to it, zero radii cut nothing.
    pub corner_clip_bounds: Bounds<ScaledPixels>,
    /// Radii of `corner_clip_bounds`.
    pub corner_radii: Corners<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub image_buffer: core_video::pixel_buffer::CVPixelBuffer,
    /// The decoded frame, off macOS: renderers that own their textures upload and draw it.
    #[cfg(not(target_os = "macos"))]
    pub frame: OwnedSurfaceFrame,
    /// Names the surface for [`Scene::swap_surface`] (Riseonly 0046); zero names nothing.
    pub swap_key: u64,
}

/// The YCbCr to RGB conversion the surface shader applies when a frame names none: full-range
/// BT.601, the matrix gpui's Metal surface path uses. Column-major, applied to (Y, Cb, Cr, 1).
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_YCBCR_TO_RGB: [[f32; 4]; 4] = [
    [1.0, 1.0, 1.0, 0.0],
    [0.0, -0.3441, 1.7720, 0.0],
    [1.4020, -0.7141, 0.0, 0.0],
    [-0.7010, 0.5291, -0.8860, 1.0],
];

/// How the pixels of an [`OwnedSurfaceFrame`] are laid out.
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnedSurfacePixelFormat {
    /// A luma plane and an interleaved CbCr plane at half resolution, converted to RGB with the
    /// frame's matrix and drawn opaque.
    Nv12,
    /// One plane of 4-byte B, G, R, A pixels with premultiplied alpha, blended over what lies
    /// beneath; no colour conversion.
    Bgra,
}

/// An NV12 picture already on the GPU, in a texture another D3D11 device shares by NT handle
/// (IDXGIResource1::CreateSharedHandle, D3D11_RESOURCE_MISC_SHARED_NTHANDLE with a keyed mutex).
/// The DirectX 11 renderer opens it with ID3D11Device1::OpenSharedResource1 instead of uploading
/// planes, and takes key 0 of its IDXGIKeyedMutex around the draw.
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SharedSurfaceHandle {
    /// The NT handle, valid while the frame that carries it lives.
    pub handle: isize,
    /// Stable for the life of the shared texture: the renderer caches what it opened by it.
    pub key: u64,
}

/// One decoded video frame in CPU memory, drawn by renderers that own their surface textures
/// (the wgpu and DirectX 11 renderers). The renderer keeps one texture set per `id` and uploads
/// only when `generation` changes, so a paused stream costs no upload.
#[cfg(not(target_os = "macos"))]
#[derive(Clone)]
pub struct OwnedSurfaceFrame {
    id: u64,
    generation: u64,
    size: Size<crate::DevicePixels>,
    pixel_format: OwnedSurfacePixelFormat,
    luma: std::sync::Arc<[u8]>,
    luma_stride: u32,
    chroma: std::sync::Arc<[u8]>,
    chroma_stride: u32,
    ycbcr_to_rgb: [[f32; 4]; 4],
    /// Set for a shared GPU picture: the handle and what keeps its texture and handle alive.
    #[cfg(target_os = "windows")]
    shared: Option<(
        SharedSurfaceHandle,
        std::sync::Arc<dyn std::any::Any + Send + Sync>,
    )>,
}

#[cfg(not(target_os = "macos"))]
impl OwnedSurfaceFrame {
    /// An NV12 frame: `luma` is `height` rows of `width` bytes, `chroma` is `ceil(height / 2)`
    /// rows of `ceil(width / 2)` interleaved Cb, Cr byte pairs, each row `*_stride` bytes apart.
    /// Returns `None` when a plane is shorter than its rows need or the size is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn nv12(
        id: u64,
        generation: u64,
        width: u32,
        height: u32,
        luma: std::sync::Arc<[u8]>,
        luma_stride: u32,
        chroma: std::sync::Arc<[u8]>,
        chroma_stride: u32,
    ) -> Option<Self> {
        let size = crate::size(
            crate::DevicePixels(i32::try_from(width).ok()?),
            crate::DevicePixels(i32::try_from(height).ok()?),
        );
        let frame = Self {
            id,
            generation,
            size,
            pixel_format: OwnedSurfacePixelFormat::Nv12,
            luma,
            luma_stride,
            chroma,
            chroma_stride,
            ycbcr_to_rgb: DEFAULT_YCBCR_TO_RGB,
            #[cfg(target_os = "windows")]
            shared: None,
        };
        frame.planes_fit().then_some(frame)
    }

    /// A BGRA frame with premultiplied alpha: `pixels` is `height` rows of `width * 4` bytes in
    /// B, G, R, A order, each row `stride` bytes apart. Returns `None` when `pixels` is shorter
    /// than its rows need or the size is empty.
    pub fn bgra(
        id: u64,
        generation: u64,
        width: u32,
        height: u32,
        pixels: std::sync::Arc<[u8]>,
        stride: u32,
    ) -> Option<Self> {
        let size = crate::size(
            crate::DevicePixels(i32::try_from(width).ok()?),
            crate::DevicePixels(i32::try_from(height).ok()?),
        );
        let frame = Self {
            id,
            generation,
            size,
            pixel_format: OwnedSurfacePixelFormat::Bgra,
            luma: pixels,
            luma_stride: stride,
            chroma: std::sync::Arc::from([]),
            chroma_stride: 0,
            ycbcr_to_rgb: DEFAULT_YCBCR_TO_RGB,
            #[cfg(target_os = "windows")]
            shared: None,
        };
        frame.planes_fit().then_some(frame)
    }

    /// An NV12 picture of `width` x `height` (both even) in a shared D3D11 texture; `keep` owns
    /// the texture and its handle and is dropped with the last clone of the frame, so the producer
    /// does not reuse the texture while a scene still draws it. Returns `None` for an odd or empty
    /// size or a null handle.
    #[cfg(target_os = "windows")]
    pub fn shared_nv12(
        id: u64,
        generation: u64,
        width: u32,
        height: u32,
        handle: SharedSurfaceHandle,
        keep: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 || handle.handle == 0 {
            return None;
        }
        let size = crate::size(
            crate::DevicePixels(i32::try_from(width).ok()?),
            crate::DevicePixels(i32::try_from(height).ok()?),
        );
        Some(Self {
            id,
            generation,
            size,
            pixel_format: OwnedSurfacePixelFormat::Nv12,
            luma: std::sync::Arc::from([]),
            luma_stride: 0,
            chroma: std::sync::Arc::from([]),
            chroma_stride: 0,
            ycbcr_to_rgb: DEFAULT_YCBCR_TO_RGB,
            shared: Some((handle, keep)),
        })
    }

    /// The shared texture of a GPU picture, `None` for a frame whose planes are in CPU memory.
    #[cfg(target_os = "windows")]
    pub fn shared_handle(&self) -> Option<SharedSurfaceHandle> {
        self.shared.as_ref().map(|(handle, _)| *handle)
    }

    /// Replaces the conversion matrix (column-major, applied to (Y, Cb, Cr, 1)).
    pub fn with_ycbcr_to_rgb(mut self, matrix: [[f32; 4]; 4]) -> Self {
        self.ycbcr_to_rgb = matrix;
        self
    }

    fn planes_fit(&self) -> bool {
        let (width, height) = (self.width() as u64, self.height() as u64);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let fits = |len: usize, stride: u32, row: u64, rows: u64| {
            let stride = stride as u64;
            stride >= row && stride * (rows - 1) + row <= len as u64
        };
        if width == 0 || height == 0 {
            return false;
        }
        match self.pixel_format {
            OwnedSurfacePixelFormat::Nv12 => {
                fits(self.luma.len(), self.luma_stride, width, height)
                    && fits(self.chroma.len(), self.chroma_stride, chroma_width * 2, chroma_height)
            }
            OwnedSurfacePixelFormat::Bgra => {
                fits(self.luma.len(), self.luma_stride, width * 4, height)
            }
        }
    }

    /// How this frame's pixels are laid out.
    pub fn pixel_format(&self) -> OwnedSurfacePixelFormat {
        self.pixel_format
    }

    /// The stream this frame belongs to.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Bumped by the producer for every new picture of the stream.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Size in pixels of the picture (of the luma plane for NV12).
    pub fn size(&self) -> Size<crate::DevicePixels> {
        self.size
    }

    /// Width in pixels of the luma plane.
    pub fn width(&self) -> u32 {
        self.size.width.0 as u32
    }

    /// Height in pixels of the luma plane.
    pub fn height(&self) -> u32 {
        self.size.height.0 as u32
    }

    /// The luma plane and its row stride in bytes (the BGRA plane of a BGRA frame; check
    /// [`OwnedSurfaceFrame::pixel_format`] first).
    pub fn luma(&self) -> (&[u8], u32) {
        (&self.luma, self.luma_stride)
    }

    /// The interleaved CbCr plane and its row stride in bytes (empty for a BGRA frame).
    pub fn chroma(&self) -> (&[u8], u32) {
        (&self.chroma, self.chroma_stride)
    }

    /// The BGRA plane and its row stride in bytes, `None` for an NV12 frame.
    pub fn bgra_pixels(&self) -> Option<(&[u8], u32)> {
        (self.pixel_format == OwnedSurfacePixelFormat::Bgra)
            .then(|| (&*self.luma, self.luma_stride))
    }

    /// The YCbCr to RGB matrix, column-major, applied to (Y, Cb, Cr, 1).
    pub fn ycbcr_to_rgb(&self) -> [[f32; 4]; 4] {
        self.ycbcr_to_rgb
    }
}

#[cfg(not(target_os = "macos"))]
impl PartialEq for OwnedSurfaceFrame {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.generation == other.generation
    }
}

#[cfg(not(target_os = "macos"))]
impl Eq for OwnedSurfaceFrame {}

#[cfg(not(target_os = "macos"))]
impl Debug for OwnedSurfaceFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedSurfaceFrame")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("size", &self.size)
            .field("pixel_format", &self.pixel_format)
            .finish_non_exhaustive()
    }
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
    pub(crate) swap_key: u64,
    pub(crate) swap_anchor: Point<P>,
    pub(crate) swap_opacity: f32,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
            swap_key: 0,
            swap_anchor: Default::default(),
            swap_opacity: 1.0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
            swap_key: self.swap_key,
            swap_anchor: self.swap_anchor.scale(factor),
            swap_opacity: self.swap_opacity,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl Path<ScaledPixels> {
    /// Takes the vertices and bounds of `local`, laid out from the origin, moved to this path's
    /// swap anchor (Riseonly 0047).
    fn take_local_geometry(&mut self, local: &Path<ScaledPixels>) {
        let anchor = self.swap_anchor;
        self.vertices = local
            .vertices
            .iter()
            .map(|vertex| {
                let mut vertex = vertex.clone();
                vertex.xy_position = vertex.xy_position + anchor;
                vertex.content_mask.bounds.origin = vertex.content_mask.bounds.origin + anchor;
                vertex
            })
            .collect();
        self.bounds = Bounds {
            origin: local.bounds.origin + anchor,
            size: local.bounds.size,
        };
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}
