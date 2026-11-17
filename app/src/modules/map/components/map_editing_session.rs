use std::collections::HashSet;

use gpui::Keystroke;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::engine::rise_map_journeys_repository::MapJourneyPoint;
pub use crate::modules::map::stores::map_actions::map_tracks_types::MapTrackPeriod;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapEditingMode {
    Journey,
    Eraser,
}

/// journey outline or eraser strokes being edited on the map
/// lives as an entity while the editing panel is open
/// callers change fields in update and call notify themselves
pub struct MapEditingSession {
    pub id: Uuid,
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub mode: MapEditingMode,
    pub render_revision: u64,
    pub polygon: Vec<MapJourneyPoint>,
    pub own_grids: Vec<MapPhotoGrid>,
    pub candidates: Vec<MapPhotoGrid>,
    pub selected: HashSet<String>,
    pub photo_revision: u64,
    pub stroke_cancellation: u64,
    pub pending_save: Option<Vec<MapEraseStroke>>,
    pub strokes: Vec<MapEraseStroke>,
    pub undone: Vec<MapEraseStroke>,
    pub saving: bool,
    pub error: Option<String>,
    pub brush_fraction: f64,
    pub adjusting_brush: bool,
    pub journey_band: Option<f32>,
}

impl MapEditingSession {
    pub const OUTLINED_JOURNEY_INSET: f32 = 380.0;

    pub fn new(mode: MapEditingMode) -> Self {
        Self {
            id: Uuid::new_v4(),
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            mode,
            render_revision: 0,
            polygon: Vec::new(),
            own_grids: Vec::new(),
            candidates: Vec::new(),
            selected: HashSet::new(),
            photo_revision: 0,
            stroke_cancellation: 0,
            pending_save: None,
            strokes: Vec::new(),
            undone: Vec::new(),
            saving: false,
            error: None,
            brush_fraction: 0.35,
            adjusting_brush: false,
            journey_band: None,
        }
    }

    pub fn drawing(&self) -> bool {
        self.mode == MapEditingMode::Journey && self.polygon.is_empty()
    }

    pub fn brush_diameter(&self) -> f64 {
        12.0 + self.brush_fraction * 84.0
    }

    pub fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.account_id.as_deref(), self.account_epoch.as_deref())
    }

    fn bump_render(&mut self) {
        self.render_revision = self.render_revision.wrapping_add(1);
    }

    pub fn set_polygon(&mut self, polygon: Vec<MapJourneyPoint>) {
        self.polygon = polygon;
        self.bump_render();
    }

    pub fn set_selected(&mut self, selected: HashSet<String>) {
        self.selected = selected;
        self.bump_render();
    }

    pub fn set_stroke_cancellation(&mut self, value: u64) {
        self.stroke_cancellation = value;
        self.bump_render();
    }

    pub fn set_pending_save(&mut self, value: Option<Vec<MapEraseStroke>>) {
        self.pending_save = value;
        self.bump_render();
    }

    pub fn set_strokes(&mut self, strokes: Vec<MapEraseStroke>) {
        self.strokes = strokes;
        self.bump_render();
    }

    pub fn set_saving(&mut self, saving: bool) {
        self.saving = saving;
        self.bump_render();
    }

    pub fn set_brush_fraction(&mut self, fraction: f64) {
        self.brush_fraction = fraction;
        self.bump_render();
    }

    pub fn reset_outline(&mut self) {
        self.set_polygon(Vec::new());
        self.candidates = Vec::new();
        self.set_selected(HashSet::new());
        self.error = None;
        self.photo_revision = self.photo_revision.wrapping_add(1);
    }

    pub fn append(&mut self, stroke: MapEraseStroke) {
        let below_limit = i64::try_from(self.strokes.len()).unwrap_or(i64::MAX) < MapEraseStroke::maximum_strokes();
        if self.saving || self.pending_save.is_some() || !self.is_current() || !below_limit {
            self.error = Some(tr("map_eraser_limit"));
            return;
        }
        self.strokes.push(stroke);
        self.bump_render();
        self.undone = Vec::new();
        self.error = None;
    }

    pub fn undo(&mut self) {
        if self.saving || self.pending_save.is_some() || !self.is_current() {
            return;
        }
        let Some(last) = self.strokes.pop() else {
            return;
        };
        self.bump_render();
        self.undone.push(last);
        self.error = None;
    }

    pub fn redo(&mut self) {
        if self.saving || self.pending_save.is_some() || !self.is_current() {
            return;
        }
        let Some(last) = self.undone.pop() else {
            return;
        };
        self.strokes.push(last);
        self.bump_render();
        self.error = None;
    }

    pub fn handle_undo_keystroke(&mut self, keystroke: &Keystroke) -> bool {
        if !keystroke.modifiers.secondary() || keystroke.key != "z" {
            return false;
        }
        if keystroke.modifiers.shift {
            self.redo();
        } else {
            self.undo();
        }
        true
    }
}

pub trait MapTrackPeriodTitle {
    fn title_key(self) -> &'static str;
}

impl MapTrackPeriodTitle for MapTrackPeriod {
    fn title_key(self) -> &'static str {
        match self {
            MapTrackPeriod::Day => "map_period_day",
            MapTrackPeriod::Week => "map_period_week",
            MapTrackPeriod::Month => "map_period_month",
            MapTrackPeriod::HalfYear => "map_period_half_year",
            MapTrackPeriod::Year => "map_period_year",
            MapTrackPeriod::All => "map_period_all",
        }
    }
}
