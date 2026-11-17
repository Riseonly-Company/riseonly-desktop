use std::sync::LazyLock;

use rise_canon::{RiseRequestBody, rise_request};

use crate::core::lib::canon_field_rule::RiseFieldRule;

/// music field rules by endpoint and pointer, checked against the contract
/// local track title and artist share one profile on create and edit
/// playlist title is split on purpose, required on create but empty on edit clears it
pub enum MusicFieldRules {}

const MUSIC_CREATE_LOCAL_TRACK_IN_PLAYLIST: &str =
    <rise_request::music::CreateLocalTrackInPlaylist as RiseRequestBody>::CONTRACT;
const MUSIC_CREATE_PLAYLIST: &str = <rise_request::music::CreatePlaylist as RiseRequestBody>::CONTRACT;
const MUSIC_UPDATE_PLAYLIST: &str = <rise_request::music::UpdatePlaylist as RiseRequestBody>::CONTRACT;
const MUSIC_REQUEST_ALBUM: &str = <rise_request::music::RequestAlbum as RiseRequestBody>::CONTRACT;

static LOCAL_TRACK_TITLE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_CREATE_LOCAL_TRACK_IN_PLAYLIST, "/title"));
static LOCAL_TRACK_ARTIST: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_CREATE_LOCAL_TRACK_IN_PLAYLIST, "/artist_name"));
static LOCAL_TRACK_DESCRIPTION: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_CREATE_LOCAL_TRACK_IN_PLAYLIST, "/description"));
static PLAYLIST_TITLE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_CREATE_PLAYLIST, "/title"));
static PLAYLIST_TITLE_ON_EDIT: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_UPDATE_PLAYLIST, "/title"));
static PLAYLIST_DESCRIPTION: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(MUSIC_CREATE_PLAYLIST, "/description"));
static ALBUM_TITLE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(MUSIC_REQUEST_ALBUM, "/title"));
static ALBUM_GENRE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(MUSIC_REQUEST_ALBUM, "/genre"));

impl MusicFieldRules {
    pub fn local_track_title() -> RiseFieldRule {
        *LOCAL_TRACK_TITLE
    }

    pub fn local_track_artist() -> RiseFieldRule {
        *LOCAL_TRACK_ARTIST
    }

    pub fn local_track_description() -> RiseFieldRule {
        *LOCAL_TRACK_DESCRIPTION
    }

    pub fn playlist_title() -> RiseFieldRule {
        *PLAYLIST_TITLE
    }

    pub fn playlist_title_on_edit() -> RiseFieldRule {
        *PLAYLIST_TITLE_ON_EDIT
    }

    pub fn playlist_description() -> RiseFieldRule {
        *PLAYLIST_DESCRIPTION
    }

    pub fn album_title() -> RiseFieldRule {
        *ALBUM_TITLE
    }

    pub fn album_genre() -> RiseFieldRule {
        *ALBUM_GENRE
    }
}
