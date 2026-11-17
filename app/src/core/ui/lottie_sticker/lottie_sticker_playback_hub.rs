use gpui::App;

use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackCoordinator, AnimatedMediaPlaybackKind,
    AnimatedMediaPlaybackPolicy,
};

pub struct LottieStickerPlaybackHub {
    _private: (),
}

impl LottieStickerPlaybackHub {
    pub fn shared() -> &'static LottieStickerPlaybackHub {
        static SHARED: LottieStickerPlaybackHub = LottieStickerPlaybackHub { _private: () };
        &SHARED
    }

    pub fn max_concurrent_players(&self) -> i64 {
        AnimatedMediaPlaybackPolicy::nominal_budget() / AnimatedMediaPlaybackKind::Lottie.cost()
    }

    pub fn request(
        &self,
        id: AnimatedMediaParticipantId,
        kind: AnimatedMediaPlaybackKind,
        on_granted: impl Fn(&mut App) + 'static,
        on_revoked: impl Fn(&mut App) + 'static,
        cx: &mut App,
    ) {
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.request(
                id,
                kind,
                move |_, cx| on_granted(cx),
                move |_, cx| on_revoked(cx),
                cx,
            );
        });
    }

    pub fn release(&self, id: AnimatedMediaParticipantId, cx: &mut App) {
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.release(id, None, cx);
        });
    }

    pub fn reset_for_testing(&self, cx: &mut App) {
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, _| {
            coordinator.reset_for_testing();
        });
    }
}
