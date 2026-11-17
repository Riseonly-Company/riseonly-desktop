use std::num::NonZeroIsize;
use std::sync::Arc;

use ::windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use ::windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
    SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use ::windows::Storage::StorageFile;
use ::windows::Storage::Streams::RandomAccessStreamReference;
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use ::windows::core::{HSTRING, Ref, factory};

use super::media_keys::*;
use crate::host_os::HostOs;

fn backend(error: impl std::fmt::Display) -> MediaKeysError {
    MediaKeysError::Backend(error.to_string())
}

pub(super) struct Transport;

impl Transport {
    pub(super) fn install(_handler: CommandHandler) -> Result<Self, MediaKeysError> {
        Ok(Self)
    }

    pub(super) fn publish(&self, _track: &NowPlaying, _state: PlaybackState) {}

    pub(super) fn clear(&self) {}
}

fn command_for(button: SystemMediaTransportControlsButton) -> Option<MediaCommand> {
    match button {
        SystemMediaTransportControlsButton::Play => Some(MediaCommand::Play),
        SystemMediaTransportControlsButton::Pause => Some(MediaCommand::Pause),
        SystemMediaTransportControlsButton::Next => Some(MediaCommand::Next),
        SystemMediaTransportControlsButton::Previous => Some(MediaCommand::Previous),
        _ => None,
    }
}

fn timespan(duration: std::time::Duration) -> TimeSpan {
    TimeSpan {
        Duration: timespan_ticks(duration),
    }
}

fn playback_status(state: PlaybackState) -> Result<MediaPlaybackStatus, MediaKeysError> {
    state
        .raw(HostOs::Windows)
        .integer()
        .and_then(|value| i32::try_from(value).ok())
        .map(MediaPlaybackStatus)
        .ok_or_else(|| backend(format!("{state:?} has no MediaPlaybackStatus")))
}

/// cover art as the card wants it
/// a file goes through StorageFile since CreateFromUri takes only http https ms-appx and ms-appdata
async fn thumbnail(artwork: &Artwork) -> ::windows::core::Result<RandomAccessStreamReference> {
    match artwork {
        Artwork::Url(url) => {
            let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))?;
            RandomAccessStreamReference::CreateFromUri(&uri)
        }
        Artwork::File(path) => {
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_path()))?.await?;
            RandomAccessStreamReference::CreateFromFile(&file)
        }
    }
}

/// controls bound to one window with the handlers that feed the share
struct Binding {
    window: NonZeroIsize,
    controls: SystemMediaTransportControls,
    buttons: i64,
    seeks: i64,
    shown_track: Option<u64>,
}

impl Binding {
    fn new(window: NonZeroIsize, share: &Arc<MediaShare>) -> Result<Self, MediaKeysError> {
        let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()
            .map_err(backend)?;
        let hwnd = HWND(window.get() as *mut core::ffi::c_void);
        let controls: SystemMediaTransportControls =
            unsafe { interop.GetForWindow(hwnd) }.map_err(backend)?;

        controls.SetIsEnabled(true).map_err(backend)?;
        controls.SetIsPlayEnabled(true).map_err(backend)?;
        controls.SetIsPauseEnabled(true).map_err(backend)?;
        controls.SetIsNextEnabled(true).map_err(backend)?;
        controls.SetIsPreviousEnabled(true).map_err(backend)?;
        controls.SetIsStopEnabled(false).map_err(backend)?;

        let pressed = Arc::clone(share);
        let buttons = controls
            .ButtonPressed(&TypedEventHandler::new(
                move |_: Ref<SystemMediaTransportControls>,
                      args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                    if let Some(command) = command_for(args.ok()?.Button()?) {
                        pressed.deliver(MediaCommandEvent::simple(command));
                    }
                    Ok(())
                },
            ))
            .map_err(backend)?;

        let scrubbed = Arc::clone(share);
        let seeks = match controls.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            move |_: Ref<SystemMediaTransportControls>,
                  args: Ref<PlaybackPositionChangeRequestedEventArgs>| {
                let requested = args.ok()?.RequestedPlaybackPosition()?;
                scrubbed.deliver(MediaCommandEvent::seek(duration_from_ticks(requested.Duration)));
                Ok(())
            },
        )) {
            Ok(token) => token,
            Err(error) => {
                if let Err(unbind) = controls.RemoveButtonPressed(buttons) {
                    tracing::warn!(error = %unbind, "media transport unbind");
                }
                return Err(backend(error));
            }
        };

        Ok(Self {
            window,
            controls,
            buttons,
            seeks,
            shown_track: None,
        })
    }

    async fn publish(&mut self, snapshot: &MediaSnapshot) -> Result<(), MediaKeysError> {
        let Some(track) = &snapshot.now else {
            let updater = self.controls.DisplayUpdater().map_err(backend)?;
            updater.ClearAll().map_err(backend)?;
            updater.Update().map_err(backend)?;
            self.controls
                .SetPlaybackStatus(MediaPlaybackStatus::Closed)
                .map_err(backend)?;
            self.shown_track = None;
            return Ok(());
        };

        if self.shown_track != Some(snapshot.track) {
            self.show(track).await?;
            self.shown_track = Some(snapshot.track);
        }

        let timeline = SystemMediaTransportControlsTimelineProperties::new().map_err(backend)?;
        timeline.SetStartTime(timespan(std::time::Duration::ZERO)).map_err(backend)?;
        timeline.SetMinSeekTime(timespan(std::time::Duration::ZERO)).map_err(backend)?;
        timeline.SetEndTime(timespan(track.duration)).map_err(backend)?;
        timeline.SetMaxSeekTime(timespan(track.duration)).map_err(backend)?;
        timeline.SetPosition(timespan(track.elapsed)).map_err(backend)?;
        self.controls
            .UpdateTimelineProperties(&timeline)
            .map_err(backend)?;
        self.controls
            .SetPlaybackRate(snapshot.state.effective_rate(track.rate))
            .map_err(backend)?;
        self.controls
            .SetPlaybackStatus(playback_status(snapshot.state)?)
            .map_err(backend)?;
        Ok(())
    }

    async fn show(&self, track: &NowPlaying) -> Result<(), MediaKeysError> {
        let updater = self.controls.DisplayUpdater().map_err(backend)?;
        updater.ClearAll().map_err(backend)?;
        updater.SetType(MediaPlaybackType::Music).map_err(backend)?;
        let music = updater.MusicProperties().map_err(backend)?;
        music
            .SetTitle(&HSTRING::from(track.title.as_str()))
            .map_err(backend)?;
        music
            .SetArtist(&HSTRING::from(track.artist.as_str()))
            .map_err(backend)?;
        music
            .SetAlbumTitle(&HSTRING::from(track.album.as_str()))
            .map_err(backend)?;
        if let Some(artwork) = &track.artwork {
            match thumbnail(artwork).await {
                Ok(reference) => updater.SetThumbnail(&reference).map_err(backend)?,
                Err(error) => {
                    tracing::warn!(%error, artwork = %artwork.as_uri(), "media card thumbnail")
                }
            }
        }
        updater.Update().map_err(backend)
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        let removed = [
            self.controls.RemoveButtonPressed(self.buttons),
            self.controls.RemovePlaybackPositionChangeRequested(self.seeks),
            self.controls.SetIsEnabled(false),
        ];
        for result in removed {
            if let Err(error) = result {
                tracing::warn!(%error, "media transport unbind");
            }
        }
    }
}

pub(super) async fn serve(
    share: Arc<MediaShare>,
    _identity: MediaIdentity,
) -> Result<(), MediaKeysError> {
    let mut binding: Option<Binding> = None;
    loop {
        if let Some(window) = share.window()
            && binding.as_ref().is_none_or(|bound| bound.window != window)
        {
            if binding.take().is_some() {
                share.set_attached(false);
            }
            binding = Some(Binding::new(window, &share)?);
            share.set_attached(true);
        }

        let snapshot = share.snapshot();
        if let Some(bound) = binding.as_mut()
            && let Err(error) = bound.publish(&snapshot).await
        {
            share.set_attached(false);
            return Err(error);
        }
        share.changed(snapshot.generation).await;
    }
}
