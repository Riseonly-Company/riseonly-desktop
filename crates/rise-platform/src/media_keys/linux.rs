use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::media_keys::*;
use crate::host_os::HostOs;

pub(super) struct Transport;

impl Transport {
    pub(super) fn install(_handler: CommandHandler) -> Result<Self, MediaKeysError> {
        Ok(Self)
    }

    pub(super) fn publish(&self, _track: &NowPlaying, _state: PlaybackState) {}

    pub(super) fn clear(&self) {}
}

fn backend(error: impl std::fmt::Display) -> MediaKeysError {
    MediaKeysError::Backend(error.to_string())
}

struct MprisRoot {
    identity: String,
    share: Arc<MediaShare>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl MprisRoot {
    fn raise(&self) {
        self.share.raise();
    }

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        self.share.can_raise()
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct MprisPlayer {
    share: Arc<MediaShare>,
}

impl MprisPlayer {
    fn send(&self, command: MediaCommand) {
        self.share.deliver(MediaCommandEvent::simple(command));
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MprisPlayer {
    fn play(&self) {
        self.send(MediaCommand::Play);
    }

    fn pause(&self) {
        self.send(MediaCommand::Pause);
    }

    fn play_pause(&self) {
        self.send(MediaCommand::Toggle);
    }

    fn next(&self) {
        self.send(MediaCommand::Next);
    }

    fn previous(&self) {
        self.send(MediaCommand::Previous);
    }

    fn stop(&self) {
        self.send(MPRIS_STOP_COMMAND);
    }

    fn seek(&self, _offset: i64) {}

    fn set_position(&self, track: OwnedObjectPath, position: i64) {
        let Ok(position) = u64::try_from(position) else {
            return;
        };

        if !self.share.is_current_track(track.as_str()) {
            return;
        }

        self.share
            .deliver(MediaCommandEvent::seek(Duration::from_micros(position)));
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.share
            .snapshot()
            .state
            .raw(HostOs::Linux)
            .text()
            .unwrap_or("Stopped")
            .to_owned()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.share.snapshot().now.map_or(1.0, |track| track.rate)
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let snapshot = self.share.snapshot();
        let Some(track) = snapshot.now else {
            return HashMap::new();
        };

        mpris_metadata(&track, snapshot.track)
            .into_iter()
            .filter_map(|(key, entry)| Some((key.to_owned(), mpris_value(entry)?)))
            .collect()
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.share
            .snapshot()
            .now
            .map_or(0, |track| micros(track.elapsed))
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

fn mpris_value(entry: MprisMetadata) -> Option<OwnedValue> {
    let value = match entry {
        MprisMetadata::Text(text) => Value::from(text),
        MprisMetadata::TextList(items) => Value::from(items),
        MprisMetadata::Path(path) => Value::from(ObjectPath::try_from(path).ok()?),
        MprisMetadata::Micros(value) => Value::from(value),
    };
    OwnedValue::try_from(value).ok()
}

pub(super) async fn serve(
    share: Arc<MediaShare>,
    identity: MediaIdentity,
) -> Result<(), MediaKeysError> {
    let connection = zbus::connection::Builder::session()
        .map_err(backend)?
        .name(identity.bus_name())
        .map_err(backend)?
        .serve_at(
            MPRIS_OBJECT_PATH,
            MprisRoot {
                identity: identity.identity.clone(),
                share: Arc::clone(&share),
            },
        )
        .map_err(backend)?
        .serve_at(
            MPRIS_OBJECT_PATH,
            MprisPlayer {
                share: Arc::clone(&share),
            },
        )
        .map_err(backend)?
        .build()
        .await
        .map_err(backend)?;

    share.set_attached(true);

    let mut seen = share.generation();
    let mut seen_seeks = share.snapshot().seeks;
    let mut raisable = share.can_raise();
    loop {
        seen = share.changed(seen).await;

        let player = connection
            .object_server()
            .interface::<_, MprisPlayer>(MPRIS_OBJECT_PATH)
            .await
            .map_err(backend)?;
        let emitter = player.signal_emitter();

        let interface = player.get().await;

        interface
            .playback_status_changed(emitter)
            .await
            .map_err(backend)?;
        interface.metadata_changed(emitter).await.map_err(backend)?;
        interface.rate_changed(emitter).await.map_err(backend)?;
        drop(interface);

        let snapshot = share.snapshot();
        if snapshot.seeks != seen_seeks {
            seen_seeks = snapshot.seeks;
            if let Some(track) = snapshot.now {
                MprisPlayer::seeked(emitter, micros(track.elapsed))
                    .await
                    .map_err(backend)?;
            }
        }

        if share.can_raise() != raisable {
            raisable = share.can_raise();
            let root = connection
                .object_server()
                .interface::<_, MprisRoot>(MPRIS_OBJECT_PATH)
                .await
                .map_err(backend)?;
            root.get()
                .await
                .can_raise_changed(root.signal_emitter())
                .await
                .map_err(backend)?;
        }
    }
}
