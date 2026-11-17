use parking_lot::Mutex;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};
use crate::core::lib::rise_call_app_volume::RiseCallAppVolume;

/// whether a call owns the process audio
/// no shared session on desktop but it still gates voice and circles and music
/// players may play in a call, recorders stay blocked
pub enum RiseCallAudioOwnership {}

struct CallAudioState {
    call_owns_session: bool,
    music_joined_call: bool,
}

static STATE: Mutex<CallAudioState> =
    Mutex::new(CallAudioState { call_owns_session: false, music_joined_call: false });

impl RiseCallAudioOwnership {
    pub const DID_TAKE_SESSION_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("riseonly.callAudio.didTakeSession");

    pub fn is_call_active() -> bool {
        STATE.lock().call_owns_session
    }

    /// music yields to a call when it starts
    /// it plays inside the call only after the user starts it there, a new call or the end resets that
    pub fn blocks_music() -> bool {
        let state = STATE.lock();
        state.call_owns_session && !state.music_joined_call
    }

    pub fn allow_music_in_call() {
        let mut state = STATE.lock();
        if state.call_owns_session {
            state.music_joined_call = true;
        }
    }

    pub fn set_call_active(active: bool) {
        let (took_session, changed) = {
            let mut state = STATE.lock();
            let took_session = active && !state.call_owns_session;
            let changed = active != state.call_owns_session;
            state.call_owns_session = active;
            if changed {
                state.music_joined_call = false;
            }
            (took_session, changed)
        };
        if changed {
            RiseCallAppVolume::post_change();
        }
        if !took_session {
            return;
        }
        RiseAppEvents::default_center().post(Self::DID_TAKE_SESSION_NOTIFICATION);
    }
}
