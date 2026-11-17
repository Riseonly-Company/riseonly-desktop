use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_crypto::{hex_string, sha256_hex};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::recorder::map_recorder_policy::{MapRecorderFix, MapRecorderPolicy};
use crate::modules::map::recorder::map_self_location_feed::MapSelfLocation;
use crate::modules::map::recorder::map_track_codec::{MapTrackCodec, MapTrackCodecPoint, MapTrackGeometry};
use crate::modules::map::recorder::rise_map_recorder::MapRecorderConfigurationHome;

/// a closed stretch of movement waiting for upload
/// id is minted here and is the server's dedup key so uploading the same file twice changes nothing
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSealedSegment {
    pub id: String,
    pub started_at: i64,
    pub utc_offset_minutes: i64,
    pub mode: String,
    #[serde(with = "map_sealed_segment_points")]
    pub points: Vec<u8>,
}

mod map_sealed_segment_points {
    use rise_core::foundation::foundation_data::{base64_encoded_string, data_base64_encoded};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64_encoded_string(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        data_base64_encoded(&encoded).ok_or_else(|| serde::de::Error::custom("points are not base64"))
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRecordedStay {
    pub arrived_at: i64,
    pub departed_at: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub utc_offset_minutes: i64,
}

#[derive(Clone, Copy, Debug)]
struct MapRecorderJournalOpenSegment {
    started_at: i64,
    utc_offset_minutes: i64,
    last_fix: MapRecorderFix,
    last_observed_ms: i64,
    point_count: i64,
    meters: f64,
}

#[derive(Default)]
struct MapRecorderJournalState {
    open: Option<MapRecorderJournalOpenSegment>,
    did_recover: bool,
}

static MAP_RECORDER_JOURNAL_REGISTRY: LazyLock<Mutex<HashMap<String, Arc<MapRecorderJournal>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const LOG_TARGET: &str = "net.riseonly.MapRecorderJournal";

/// recorder disk per account under the app data directory
/// open segment is an append only file of 24 byte records so a fix is one small write
/// sealing makes one json doc to upload, run off main
pub struct MapRecorderJournal {
    files: Option<MapRecorderJournalFiles>,
    state: RiseIsolated<MapRecorderJournalState>,
}

/// per account files under RiseMapRecorder/<hex account> in the app data directory
struct MapRecorderJournalFiles {
    directory: PathBuf,
    sealed_directory: PathBuf,
    open_url: PathBuf,
    stays_url: PathBuf,
    location_url: PathBuf,
}

impl MapRecorderJournal {
    const RECORD_BYTES: usize = 24;

    fn maximum_sealed() -> usize {
        usize::try_from(RiseLimits::client_value("map_recorder_sealed_segments")).unwrap_or(0)
    }

    fn maximum_segment_points() -> i64 {
        RiseLimits::service_value("map_max_segment_points")
    }

    fn maximum_stays() -> usize {
        usize::try_from(RiseLimits::client_value("map_recorder_pending_stays")).unwrap_or(0)
    }

    /// one journal per account shared by the recorder that writes it and the uploader that drains it
    /// two journals over the same files would race
    pub fn for_account(account_id: &str) -> Arc<MapRecorderJournal> {
        let mut registry = MAP_RECORDER_JOURNAL_REGISTRY.lock();
        if let Some(journal) = registry.get(account_id) {
            return journal.clone();
        }
        let journal = Arc::new(MapRecorderJournal::new(account_id));
        if registry.len() >= 8 {
            registry.clear();
        }
        registry.insert(account_id.to_owned(), journal.clone());
        journal
    }

    fn new(account_id: &str) -> Self {
        let base = AppPaths::resolve(AppEnvironment::compiled().canon())
            .ok()
            .filter(|paths| paths.create_all().is_ok())
            .map(|paths| paths.data_dir().to_path_buf());
        let folder = hex_string(account_id.as_bytes());
        let files = base.map(|base| {
            let directory = base.join("RiseMapRecorder").join(folder);
            MapRecorderJournalFiles {
                sealed_directory: directory.join("sealed"),
                open_url: directory.join("open.seg"),
                stays_url: directory.join("stays.json"),
                location_url: directory.join("location.json"),
                directory,
            }
        });
        Self {
            files,
            state: RiseIsolated::new(MapRecorderJournalState::default()),
        }
    }

    /// writes one fix if the policy accepts it after the previous one
    /// defaults are records_movement true and excluded_zones empty and segment_gap_ms from the policy
    pub fn append(
        &self,
        fix: &MapRecorderFix,
        utc_offset_minutes: i64,
        records_movement: bool,
        excluded_zones: &[MapRecorderConfigurationHome],
        segment_gap_ms: i64,
    ) -> bool {
        let Some(files) = self.files.as_ref() else {
            return false;
        };
        self.state.with(|state| {
            files.recover_if_needed(state);
            let is_after_open = state.open.is_none_or(|open| fix.time_ms > open.last_observed_ms);
            if !(state.did_recover && MapRecorderPolicy::is_usable(fix) && is_after_open) {
                return false;
            }
            let mut sealed = false;
            if let Some(current) = state.open
                && excluded_zones
                    .iter()
                    .any(|zone| MapRecorderPolicy::crosses_zone(&current.last_fix, fix, zone))
            {
                sealed = files.seal(state, false);
                if !sealed {
                    return false;
                }
            }
            if let Some(current) = state.open
                && MapRecorderPolicy::closes_segment(
                    current.started_at,
                    current.last_observed_ms,
                    current.point_count,
                    fix,
                    segment_gap_ms,
                )
            {
                let continuing = MapRecorderPolicy::continues_segment(
                    &current.last_fix,
                    current.last_observed_ms,
                    fix,
                    segment_gap_ms,
                );
                sealed = files.seal(state, continuing);
                if !sealed {
                    return false;
                }
            }
            if !records_movement {
                if let Some(open) = state.open.as_mut() {
                    open.last_observed_ms = fix.time_ms;
                }
                return sealed;
            }
            if let Some(open) = state.open.as_mut()
                && MapRecorderPolicy::distance_meters(fix, &open.last_fix) < MapRecorderPolicy::minimum_step_meters()
            {
                open.last_observed_ms = fix.time_ms;
                return sealed;
            }
            if !MapRecorderPolicy::accepts(fix, state.open.as_ref().map(|open| &open.last_fix)) {
                return sealed;
            }
            if !files.write(state, fix) {
                return sealed;
            }

            match state.open.as_mut() {
                Some(current) => {
                    current.meters += MapRecorderPolicy::distance_meters(fix, &current.last_fix);
                    current.last_fix = *fix;
                    current.last_observed_ms = fix.time_ms;
                    current.point_count += 1;
                }
                None => {
                    state.open = Some(MapRecorderJournalOpenSegment {
                        started_at: fix.time_ms,
                        utc_offset_minutes,
                        last_fix: *fix,
                        last_observed_ms: fix.time_ms,
                        point_count: 1,
                        meters: 0.0,
                    });
                }
            }
            sealed
        })
    }

    /// closes the open segment
    /// a single point is not a line and is dropped like the server would
    /// continuing defaults to false
    pub fn seal_open_segment(&self, continuing: bool) -> bool {
        let Some(files) = self.files.as_ref() else {
            return false;
        };
        self.state.with(|state| files.seal(state, continuing))
    }

    pub fn save_location(&self, location: &MapSelfLocation) {
        let Some(files) = self.files.as_ref() else {
            return;
        };
        self.state.with(|_| {
            files.ensure_directories();
            let written = serde_json::to_vec(location)
                .map_err(std::io::Error::other)
                .and_then(|data| Self::write_atomically(&files.location_url, &data));
            if written.is_err() {
                tracing::error!(target: LOG_TARGET, "Location snapshot could not be persisted");
            }
        });
    }

    pub fn last_location(&self) -> Option<MapSelfLocation> {
        let files = self.files.as_ref()?;
        self.state.with(|_| {
            let data = std::fs::read(&files.location_url).ok()?;
            serde_json::from_slice(&data).ok()
        })
    }

    pub fn add_stay(&self, stay: MapRecordedStay) {
        let Some(files) = self.files.as_ref() else {
            return;
        };
        self.state.with(|_| {
            let mut stays = files.read_pending_stays();
            stays.retain(|pending| pending.arrived_at != stay.arrived_at);
            stays.push(stay);
            let maximum = Self::maximum_stays();
            if stays.len() > maximum {
                stays.drain(..stays.len() - maximum);
            }
            files.write_stays(&stays);
        });
    }

    /// oldest first so a long offline stretch uploads in the order it happened
    pub fn pending_segments(&self, limit: usize) -> Vec<MapSealedSegment> {
        let Some(files) = self.files.as_ref() else {
            return Vec::new();
        };
        self.state.with(|state| files.read_pending_segments(state, limit))
    }

    pub fn pending_stays(&self) -> Vec<MapRecordedStay> {
        let Some(files) = self.files.as_ref() else {
            return Vec::new();
        };
        self.state.with(|_| files.read_pending_stays())
    }

    /// ids the server accepted or refused for good leave the disk
    /// anything else stays queued, thats the contract of upload_track
    pub fn resolve(&self, settled_segment_ids: &HashSet<String>, uploaded_stays: &[MapRecordedStay]) -> bool {
        let Some(files) = self.files.as_ref() else {
            return false;
        };
        self.state.with(|state| {
            files.recover_if_needed(state);
            if !state.did_recover {
                return false;
            }
            for url in files.sealed_files() {
                if settled_segment_ids.contains(&Self::segment_id(&url)) && FileManager.remove_item(&url).is_err() {
                    return false;
                }
            }
            if uploaded_stays.is_empty() {
                return true;
            }
            let uploaded: HashSet<i64> = uploaded_stays.iter().map(|stay| stay.arrived_at).collect();
            let remaining: Vec<MapRecordedStay> = files
                .read_pending_stays()
                .into_iter()
                .filter(|stay| !uploaded.contains(&stay.arrived_at))
                .collect();
            files.write_stays(&remaining)
        })
    }

    pub fn unsent_lines(&self) -> Vec<Vec<MapTrackCodecPoint>> {
        let Some(files) = self.files.as_ref() else {
            return Vec::new();
        };
        self.state.with(|state| {
            files.recover_if_needed(state);
            let mut lines: Vec<Vec<MapTrackCodecPoint>> = files
                .read_pending_segments(state, Self::maximum_sealed())
                .into_iter()
                .filter_map(|segment| {
                    MapTrackCodec::decode_points(&segment.points, segment.started_at, Self::maximum_segment_points())
                })
                .collect();
            if state.open.is_some() {
                lines.push(files.read_open_points());
            }
            lines.retain(|line| line.len() >= 2);
            lines
        })
    }

    pub fn erase_all(&self) {
        let Some(files) = self.files.as_ref() else {
            return;
        };
        self.state.with(|state| {
            state.open = None;
            let _ = FileManager.remove_item(&files.directory);
        });
    }

    fn read_i32(bytes: &[u8], offset: usize) -> i32 {
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[offset..offset + 4]);
        i32::from_le_bytes(word)
    }

    fn read_i64(bytes: &[u8], offset: usize) -> i64 {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[offset..offset + 8]);
        i64::from_le_bytes(word)
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let directory = url.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
        let staging = directory.join(format!(".{}.tmp", Uuid::new_v4().uuid_string()));
        std::fs::write(&staging, data)?;
        if let Err(error) = std::fs::rename(&staging, url) {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
        Ok(())
    }

    fn segment_id(url: &Path) -> String {
        let name = url.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        match name.split_once('-') {
            Some((_, id)) => id.to_owned(),
            None => name,
        }
    }
}

impl MapRecorderJournalFiles {
    fn seal(&self, state: &mut MapRecorderJournalState, continuing: bool) -> bool {
        self.recover_if_needed(state);
        if !state.did_recover {
            return false;
        }
        let Some(current) = state.open else {
            return true;
        };
        if current.point_count < 2 {
            if !continuing {
                if FileManager.remove_item(&self.open_url).is_err() {
                    return false;
                }
                state.open = None;
            }
            return true;
        }
        let Ok(raw) = std::fs::read(&self.open_url) else {
            return false;
        };
        let points = self.read_open_points();
        let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else {
            return false;
        };
        if i64::try_from(points.len()).ok() != Some(current.point_count) {
            return false;
        }

        let seconds = ((last.time_ms - first.time_ms) as f64 / 1_000.0).max(1.0);
        let payload = MapTrackCodec::encode_points(&points, first.time_ms);
        let digest = sha256_hex(&raw);
        let sealed = MapSealedSegment {
            id: digest.chars().take(32).collect(),
            started_at: first.time_ms,
            utc_offset_minutes: current.utc_offset_minutes,
            mode: MapRecorderPolicy::motion(current.meters / seconds).raw_value().to_owned(),
            points: payload,
        };
        self.ensure_directories();
        let name = format!("{:020}-{}.json", sealed.started_at, sealed.id);
        let committed = self.commit_seal(&sealed, &name, &raw, continuing);
        match committed {
            Ok(()) => {
                state.open = if continuing {
                    Some(MapRecorderJournalOpenSegment {
                        started_at: last.time_ms,
                        utc_offset_minutes: current.utc_offset_minutes,
                        last_fix: current.last_fix,
                        last_observed_ms: current.last_observed_ms,
                        point_count: 1,
                        meters: 0.0,
                    })
                } else {
                    None
                };
                self.trim_sealed();
                true
            }
            Err(_) => {
                state.did_recover = false;
                tracing::error!(target: LOG_TARGET, "Track checkpoint could not be committed; journal retained");
                false
            }
        }
    }

    fn commit_seal(&self, sealed: &MapSealedSegment, name: &str, raw: &[u8], continuing: bool) -> std::io::Result<()> {
        let data = serde_json::to_vec(sealed)?;
        MapRecorderJournal::write_atomically(&self.sealed_directory.join(name), &data)?;
        if continuing {
            let tail = &raw[raw.len().saturating_sub(MapRecorderJournal::RECORD_BYTES)..];
            MapRecorderJournal::write_atomically(&self.open_url, tail)
        } else {
            FileManager.remove_item(&self.open_url).map_err(std::io::Error::other)
        }
    }

    fn read_pending_segments(&self, state: &mut MapRecorderJournalState, limit: usize) -> Vec<MapSealedSegment> {
        self.recover_if_needed(state);
        if !state.did_recover {
            return Vec::new();
        }
        self.sealed_files()
            .into_iter()
            .take(limit)
            .filter_map(|url| {
                let data = std::fs::read(&url).ok()?;
                serde_json::from_slice::<MapSealedSegment>(&data).ok()
            })
            .collect()
    }

    fn read_pending_stays(&self) -> Vec<MapRecordedStay> {
        let Ok(data) = std::fs::read(&self.stays_url) else {
            return Vec::new();
        };
        serde_json::from_slice(&data).unwrap_or_default()
    }

    fn recover_if_needed(&self, state: &mut MapRecorderJournalState) {
        if state.did_recover {
            return;
        }
        if FileManager.file_exists(&self.open_url) {
            let Ok(mut raw) = std::fs::read(&self.open_url) else {
                return;
            };
            raw.truncate(raw.len() / MapRecorderJournal::RECORD_BYTES * MapRecorderJournal::RECORD_BYTES);
            let sealed_end = self
                .sealed_files()
                .last()
                .and_then(|url| std::fs::read(url).ok())
                .and_then(|data| serde_json::from_slice::<MapSealedSegment>(&data).ok())
                .and_then(|sealed| {
                    MapTrackCodec::decode_points(
                        &sealed.points,
                        sealed.started_at,
                        MapRecorderJournal::maximum_segment_points(),
                    )
                })
                .and_then(|points| points.last().map(|point| point.time_ms));
            if let Some(end) = sealed_end {
                let mut offset = 0;
                while offset + MapRecorderJournal::RECORD_BYTES <= raw.len() {
                    let time = MapRecorderJournal::read_i64(&raw, offset + 8);
                    if time >= end {
                        break;
                    }
                    offset += MapRecorderJournal::RECORD_BYTES;
                }
                raw.drain(..offset);
            }
            if MapRecorderJournal::write_atomically(&self.open_url, &raw).is_err() {
                return;
            }
        }
        state.did_recover = true;
        state.open = None;
        let points = self.read_open_points();
        let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else {
            return;
        };
        let meters: f64 = points
            .windows(2)
            .map(|pair| {
                MapTrackGeometry::haversine_meters(
                    f64::from(pair[0].lat_e6) / 1e6,
                    f64::from(pair[0].lng_e6) / 1e6,
                    f64::from(pair[1].lat_e6) / 1e6,
                    f64::from(pair[1].lng_e6) / 1e6,
                )
            })
            .sum();
        state.open = Some(MapRecorderJournalOpenSegment {
            started_at: first.time_ms,
            utc_offset_minutes: i64::from(TimeZone::current().seconds_from_gmt(&RiseClock::now()) / 60),
            last_fix: MapRecorderFix {
                latitude: f64::from(last.lat_e6) / 1e6,
                longitude: f64::from(last.lng_e6) / 1e6,
                accuracy_meters: 0.0,
                speed_meters_per_second: 0.0,
                heading_degrees: -1.0,
                time_ms: last.time_ms,
            },
            last_observed_ms: last.time_ms,
            point_count: i64::try_from(points.len()).unwrap_or(i64::MAX),
            meters,
        });
    }

    fn write(&self, state: &mut MapRecorderJournalState, fix: &MapRecorderFix) -> bool {
        self.ensure_directories();
        let point = fix.point();
        let mut record = Vec::with_capacity(MapRecorderJournal::RECORD_BYTES);
        record.extend_from_slice(&point.lat_e6.to_le_bytes());
        record.extend_from_slice(&point.lng_e6.to_le_bytes());
        record.extend_from_slice(&point.time_ms.to_le_bytes());
        record.extend_from_slice(&(fix.speed_meters_per_second as f32).to_bits().to_le_bytes());
        record.extend_from_slice(&(fix.accuracy_meters as f32).to_bits().to_le_bytes());

        if !FileManager.file_exists(&self.open_url) {
            return FileManager.create_file(&self.open_url, Some(record.as_slice()));
        }
        let Ok(mut handle) = std::fs::OpenOptions::new().append(true).open(&self.open_url) else {
            return false;
        };
        let written = handle.write_all(&record).and_then(|()| handle.sync_all());
        if written.is_err() {
            state.did_recover = false;
            tracing::error!(target: LOG_TARGET, "Track point could not be committed");
            return false;
        }
        true
    }

    /// a torn last record left by a kill in the middle of a write is ignored
    fn read_open_points(&self) -> Vec<MapTrackCodecPoint> {
        let Ok(data) = std::fs::read(&self.open_url) else {
            return Vec::new();
        };
        let maximum = usize::try_from(MapRecorderPolicy::segment_maximum_points()).unwrap_or(0);
        let count = (data.len() / MapRecorderJournal::RECORD_BYTES).min(maximum);
        (0..count)
            .map(|index| {
                let base = index * MapRecorderJournal::RECORD_BYTES;
                MapTrackCodecPoint {
                    lat_e6: MapRecorderJournal::read_i32(&data, base),
                    lng_e6: MapRecorderJournal::read_i32(&data, base + 4),
                    time_ms: MapRecorderJournal::read_i64(&data, base + 8),
                }
            })
            .collect()
    }

    fn sealed_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.sealed_directory) else {
            return Vec::new();
        };
        let mut urls: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|url| url.extension().is_some_and(|extension| extension == "json"))
            .collect();
        urls.sort();
        urls
    }

    fn trim_sealed(&self) {
        let files = self.sealed_files();
        let maximum = MapRecorderJournal::maximum_sealed();
        if files.len() <= maximum {
            return;
        }
        for url in &files[..files.len() - maximum] {
            let _ = FileManager.remove_item(url);
        }
    }

    fn write_stays(&self, stays: &[MapRecordedStay]) -> bool {
        self.ensure_directories();
        let written = serde_json::to_vec(stays)
            .map_err(std::io::Error::other)
            .and_then(|data| MapRecorderJournal::write_atomically(&self.stays_url, &data));
        if written.is_err() {
            tracing::error!(target: LOG_TARGET, "Track stays could not be committed; retry deferred");
            return false;
        }
        true
    }

    fn ensure_directories(&self) {
        if FileManager.file_exists(&self.sealed_directory) {
            return;
        }
        let _ = FileManager.create_directory(&self.sealed_directory, true);
        let values = URLResourceValues { is_excluded_from_backup: Some(true) };
        let _ = set_resource_values(&self.directory, &values);
    }
}
