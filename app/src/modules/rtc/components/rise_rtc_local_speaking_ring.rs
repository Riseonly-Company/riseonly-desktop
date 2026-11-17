use gpui::prelude::*;
use gpui::{App, Context, Entity, Subscription, Window, div, px};

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::rtc::components::rise_rtc_speaking_ring::RiseRTCSpeakingRing;
use crate::modules::rtc::media::rise_rtc_media_presentation::RiseRTCMediaPresentation;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

const RING_WIDTH: f32 = 2.0;

pub struct RiseRTCLocalSpeakingRing {
    auth: Entity<AuthServicesStore>,
    media: Entity<RiseRTCMediaPresentation>,
    rtc: Entity<RTCServicesStore>,
    speaking: bool,
    _subscriptions: Vec<Subscription>,
}

impl RiseRTCLocalSpeakingRing {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let media = RiseRTCMediaPresentation::shared(cx);
        let rtc = RTCServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&media, |this, _, cx| this.refresh(cx)),
            cx.observe(&rtc, |this, _, cx| this.refresh(cx)),
            cx.observe(&auth, |this, _, cx| this.refresh(cx)),
        ];
        let mut ring = Self { auth, media, rtc, speaking: false, _subscriptions: subscriptions };
        ring.speaking = ring.resolve(cx);
        ring
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let speaking = self.resolve(cx);
        if speaking != self.speaking {
            self.speaking = speaking;
            cx.notify();
        }
    }

    /// the gated voice the call sends
    /// so the ring stays dark in silence, muted or deafened
    fn resolve(&self, cx: &App) -> bool {
        let Some(user_id) = self.auth.read(cx).active_account_id().filter(|id| !id.is_empty()) else {
            return false;
        };
        if !self.media.read(cx).is_speaking(user_id) {
            return false;
        }
        self.rtc.read(cx).active_room(cx).and_then(|room| room.self_participant.as_ref()).is_none_or(|participant| {
            participant.microphone_enabled
                && !participant.deafened
                && !participant.server_deafened
                && !participant.is_server_silenced()
        })
    }
}

impl Render for RiseRTCLocalSpeakingRing {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let ring = div().absolute().top_0().left_0().size_full();
        if !self.speaking {
            return ring;
        }
        ring.rounded_full().border(px(RING_WIDTH)).border_color(RiseRTCSpeakingRing::color())
    }
}
