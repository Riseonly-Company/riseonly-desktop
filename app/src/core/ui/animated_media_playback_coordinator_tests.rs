use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, TestAppContext, px, size};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::ui::animated_gif_view::AnimatedImagePlaybackPolicy;
use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackConditions, AnimatedMediaPlaybackCoordinator,
    AnimatedMediaPlaybackKind, AnimatedMediaPlaybackPolicy,
};
use crate::core::ui::lottie_sticker::lottie_sticker_playback_hub::LottieStickerPlaybackHub;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListMountEnvelopePolicy;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageAnimatedMediaScrollPolicy;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::{
    ChatMediaPlaybackRate, ChatVoiceMessagePlayer,
};
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

type ParticipantLog = Rc<RefCell<Vec<AnimatedMediaParticipantId>>>;

fn participant_log() -> ParticipantLog {
    Rc::new(RefCell::new(Vec::new()))
}

fn scroll_metrics(velocity: f32, is_dragging: bool, is_decelerating: bool) -> RiseManualListScrollMetrics {
    RiseManualListScrollMetrics {
        content_offset_y: px(0.0),
        minimum_content_offset_y: px(0.0),
        maximum_content_offset_y: px(1_000.0),
        viewport_size: size(px(390.0), px(844.0)),
        content_height: px(1_844.0),
        distance_from_top: px(0.0),
        distance_from_bottom: px(1_000.0),
        velocity_y: px(velocity),
        is_tracking: false,
        is_dragging,
        is_decelerating,
    }
}

#[test]
fn visible_chat_animations_have_a_viewport_sized_playback_budget() {
    assert_eq!(
        LottieStickerPlaybackHub::shared().max_concurrent_players(),
        AnimatedMediaPlaybackPolicy::nominal_budget() / AnimatedMediaPlaybackKind::Lottie.cost()
    );
    assert!(AnimatedImagePlaybackPolicy::maximum_visible_players() >= 12);
}

#[gpui::test]
fn newly_visible_sticker_queues_instead_of_preempting_a_playing_one(cx: &mut TestAppContext) {
    let hub = LottieStickerPlaybackHub::shared();
    cx.update(|cx| hub.reset_for_testing(cx));

    let owners: Vec<AnimatedMediaParticipantId> =
        (0..=hub.max_concurrent_players()).map(|_| AnimatedMediaParticipantId::next()).collect();
    let granted = participant_log();
    let revoked = participant_log();

    for id in owners.iter().copied() {
        let granted = granted.clone();
        let revoked = revoked.clone();
        cx.update(|cx| {
            hub.request(
                id,
                AnimatedMediaPlaybackKind::Lottie,
                move |_: &mut App| granted.borrow_mut().push(id),
                move |_: &mut App| revoked.borrow_mut().push(id),
                cx,
            );
        });
    }

    let first_owner = owners[0];
    let last_owner = owners[owners.len() - 1];
    assert!(revoked.borrow().is_empty());
    assert_eq!(granted.borrow().len() as i64, hub.max_concurrent_players());
    assert_eq!(granted.borrow().first().copied(), Some(first_owner));
    assert!(!granted.borrow().contains(&last_owner));

    cx.update(|cx| hub.release(first_owner, cx));
    assert_eq!(granted.borrow().last().copied(), Some(last_owner));

    cx.update(|cx| hub.reset_for_testing(cx));
}

#[test]
fn animation_is_never_traded_away_for_thermal_or_power_state() {
    let active = AnimatedMediaPlaybackConditions::new(true, false);
    assert_eq!(AnimatedMediaPlaybackPolicy::budget(&active), AnimatedMediaPlaybackPolicy::nominal_budget());
    assert_eq!(AnimatedMediaPlaybackPolicy::budget(&AnimatedMediaPlaybackConditions::new(false, false)), 0);
    assert_eq!(AnimatedMediaPlaybackPolicy::budget(&AnimatedMediaPlaybackConditions::new(true, true)), 0);
}

#[test]
fn budget_covers_a_full_picker_grid_of_simultaneous_stickers() {
    let players = AnimatedMediaPlaybackPolicy::nominal_budget() / AnimatedMediaPlaybackKind::Lottie.cost();
    assert!(players >= 48);
}

#[test]
fn scrolling_never_suspends_visible_animated_media() {
    let fast = scroll_metrics(RiseManualListMountEnvelopePolicy::FAST_SCROLL_VELOCITY_THRESHOLD, true, false);
    assert!(!RiseMessageAnimatedMediaScrollPolicy::should_suspend_playback(false, &fast));

    let decelerating = scroll_metrics(20.0, false, true);
    assert!(!RiseMessageAnimatedMediaScrollPolicy::should_suspend_playback(true, &decelerating));

    let settled = scroll_metrics(0.0, false, false);
    assert!(!RiseMessageAnimatedMediaScrollPolicy::should_suspend_playback(true, &settled));
}

#[gpui::test]
fn shared_weighted_budget_revokes_and_regrants_across_media_kinds(cx: &mut TestAppContext) {
    let coordinator = cx.update(|cx| AnimatedMediaPlaybackCoordinator::shared(cx));
    cx.update(|cx| {
        coordinator.update(cx, |coordinator, cx| {
            coordinator.reset_for_testing();
            coordinator.set_budget_for_testing(8, cx);
        });
    });

    let first_id = AnimatedMediaParticipantId::next();
    let second_id = AnimatedMediaParticipantId::next();
    let granted = participant_log();
    let revoked = participant_log();

    for id in [first_id, second_id] {
        let granted = granted.clone();
        let revoked = revoked.clone();
        cx.update(|cx| {
            coordinator.update(cx, |coordinator, cx| {
                coordinator.request(
                    id,
                    AnimatedMediaPlaybackKind::VideoSticker,
                    move |_: u64, _: &mut App| granted.borrow_mut().push(id),
                    move |_: u64, _: &mut App| revoked.borrow_mut().push(id),
                    cx,
                );
            });
        });
    }

    assert_eq!(cx.update(|cx| coordinator.read(cx).active_cost_for_testing()), 8);
    cx.update(|cx| coordinator.update(cx, |coordinator, cx| coordinator.set_budget_for_testing(4, cx)));
    assert_eq!(cx.update(|cx| coordinator.read(cx).active_cost_for_testing()), 4);
    assert_eq!(*revoked.borrow(), vec![first_id]);
    assert_eq!(cx.update(|cx| coordinator.read(cx).waiting_count_for_testing()), 1);

    cx.update(|cx| coordinator.update(cx, |coordinator, cx| coordinator.release(second_id, None, cx)));
    assert_eq!(cx.update(|cx| coordinator.read(cx).active_cost_for_testing()), 4);
    assert_eq!(*granted.borrow(), vec![first_id, second_id, first_id]);
    assert_eq!(cx.update(|cx| coordinator.read(cx).waiting_count_for_testing()), 0);

    cx.update(|cx| coordinator.update(cx, |coordinator, _| coordinator.reset_for_testing()));
}

#[test]
fn playback_rate_cycles_through_supported_values() {
    assert_eq!(ChatMediaPlaybackRate::next(1.0), 1.5);
    assert_eq!(ChatMediaPlaybackRate::next(1.5), 2.0);
    assert_eq!(ChatMediaPlaybackRate::next(2.0), 1.0);
    assert_eq!(ChatMediaPlaybackRate::next(1.25), 1.0);
    assert_eq!(ChatMediaPlaybackRate::label(1.0), "1x");
    assert_eq!(ChatMediaPlaybackRate::label(1.5), "1.5x");
}

#[gpui::test]
fn external_audio_owners_release_independently(cx: &mut TestAppContext) {
    let first = MusicExternalAudioOwner::unique("test.first");
    let second = MusicExternalAudioOwner::unique("test.second");
    let player = cx.update(|cx| MusicPlayerState::shared(cx));

    cx.update(|cx| {
        player.update(cx, |player, cx| {
            player.suspend_for_external_audio(first.clone(), true, cx);
            player.suspend_for_external_audio(second.clone(), true, cx);
        });
    });

    cx.update(|cx| {
        let player = player.read(cx);
        assert!(player.is_external_audio_owner_active(&first));
        assert!(player.is_external_audio_owner_active(&second));
        assert!(player.is_suspended_for_external_audio());
    });

    cx.update(|cx| player.update(cx, |player, cx| player.resume_after_external_audio(first.clone(), cx)));

    cx.update(|cx| {
        let player = player.read(cx);
        assert!(!player.is_external_audio_owner_active(&first));
        assert!(player.is_external_audio_owner_active(&second));
        assert!(player.is_suspended_for_external_audio());
    });

    cx.update(|cx| player.update(cx, |player, cx| player.resume_after_external_audio(second.clone(), cx)));

    cx.update(|cx| assert!(!player.read(cx).is_external_audio_owner_active(&second)));
}

#[gpui::test]
fn user_music_control_takes_over_chat_playback_but_not_recording_owner(cx: &mut TestAppContext) {
    let player = cx.update(|cx| MusicPlayerState::shared(cx));
    cx.update(|cx| {
        player.update(cx, |player, cx| {
            player.suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VOICE_MESSAGE, true, cx);
        });
    });

    let took_over =
        cx.update(|cx| player.update(cx, |player, cx| player.take_over_external_chat_playback_for_user_control(true, cx)));
    assert!(took_over);
    cx.update(|cx| {
        assert!(!player.read(cx).is_external_audio_owner_active(&MusicExternalAudioOwner::CHAT_VOICE_MESSAGE));
    });

    let recording_owner = MusicExternalAudioOwner::unique("test.recording");
    cx.update(|cx| {
        player.update(cx, |player, cx| player.suspend_for_external_audio(recording_owner.clone(), false, cx));
    });

    let took_over =
        cx.update(|cx| player.update(cx, |player, cx| player.take_over_external_chat_playback_for_user_control(true, cx)));
    assert!(!took_over);
    cx.update(|cx| assert!(player.read(cx).is_external_audio_owner_active(&recording_owner)));

    cx.update(|cx| player.update(cx, |player, cx| player.resume_after_external_audio(recording_owner.clone(), cx)));
}

#[gpui::test]
fn voice_session_publishes_header_metadata_before_remote_load_completes(cx: &mut TestAppContext) {
    let player = cx.update(|cx| ChatVoiceMessagePlayer::shared(cx));
    let message_id = Uuid::new_v4().uuid_string();
    let chat_id = Uuid::new_v4().uuid_string();
    let mut item = MessageMediaItem::new(
        Uuid::new_v4().uuid_string(),
        MessageMediaKind::Voice,
        format!("https://127.0.0.1:1/{message_id}.m4a"),
    );
    item.duration = Some(42.0);

    cx.update(|cx| {
        player.update(cx, |player, cx| {
            player.play(
                message_id.clone(),
                chat_id.clone(),
                "Fallback chat".to_owned(),
                Some("Sender".to_owned()),
                item,
                cx,
            );
        });
    });

    cx.update(|cx| {
        let player = player.read(cx);
        assert_eq!(player.playing_message_id(), Some(message_id.as_str()));
        assert_eq!(player.active_chat_id(), chat_id);
        assert_eq!(player.sender_name(), "Sender");
        assert_eq!(player.duration_seconds(), 42.0);
        assert!(player.is_loading());
    });

    cx.update(|cx| player.update(cx, |player, cx| player.stop(cx)));
}
