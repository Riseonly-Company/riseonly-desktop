use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, IntoElement, Render, RenderOnce, SharedString,
    Subscription, Window, black,
};
use rise_canon::rise_proto::stickers::ProtoPackRecord;
use rise_canon::rise_service_text::stickers_service;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationCover, PlaylistCollaborationErrorState,
};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::{RisePackInvitePreview, RisePackJoinResult};
use crate::modules::stickers::pages::pack_collaboration::pack_collaboration_components::{
    PackCollaborationUserLabel, RisePackUserRef,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;

const SCRIM_OPACITY: f32 = 0.5;
const SHEET_CORNER_RADIUS: f32 = 34.0;
const SHEET_BOTTOM_PADDING: f32 = 14.0;
const SHEET_MAX_WIDTH: f32 = 560.0;
const DRAG_MINIMUM_DISTANCE: f32 = 10.0;
const DRAG_DISMISS_DISTANCE: f32 = 120.0;
const PRESENTATION_RESPONSE: f64 = 0.36;
const PRESENTATION_DAMPING: f64 = 0.88;
const DRAG_RETURN_RESPONSE: f64 = 0.3;
const DRAG_RETURN_DAMPING: f64 = 0.86;
const COVER_SIZE: f32 = 88.0;

/// callback with user id and tag to open a profile
pub type PackInviteOpenProfile = Rc<dyn Fn(String, String, &mut Window, &mut App)>;

#[derive(Clone, PartialEq)]
pub enum PackInviteSheetStorePhase {
    Loading,
    Preview(RisePackInvitePreview),
    Failed(String),
}

/// state of the pack invite preview that a k link opens over any screen
pub struct PackInviteSheetStore {
    token: Option<String>,
    phase: PackInviteSheetStorePhase,
    is_submitting: bool,
    submission_error: Option<String>,
    generation: u64,
}

impl PackInviteSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            token: None,
            phase: PackInviteSheetStorePhase::Loading,
            is_submitting: false,
            submission_error: None,
            generation: 0,
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn phase(&self) -> &PackInviteSheetStorePhase {
        &self.phase
    }

    pub fn is_submitting(&self) -> bool {
        self.is_submitting
    }

    pub fn submission_error(&self) -> Option<&str> {
        self.submission_error.as_deref()
    }

    pub fn is_presented(&self) -> bool {
        self.token.is_some()
    }

    pub fn present(&mut self, raw: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(raw).to_owned();
        if trimmed.is_empty() {
            return;
        }
        self.token = Some(trimmed);
        self.load(cx);
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.token = None;
        self.phase = PackInviteSheetStorePhase::Loading;
        self.is_submitting = false;
        self.submission_error = None;
        cx.notify();
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        if self.is_submitting {
            return;
        }
        let PackInviteSheetStorePhase::Preview(preview) = &self.phase else {
            return;
        };
        let preview = preview.clone();
        let stamp = self.generation;
        self.is_submitting = true;
        self.submission_error = None;
        cx.notify();
        let success = cx.weak_entity();
        let failure = cx.weak_entity();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.join_pack_by_link(
                &token,
                move |result: RisePackJoinResult, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = success.update(cx, |store, cx| {
                            if store.generation != stamp {
                                return;
                            }
                            store.is_submitting = false;
                            cx.notify();
                            store.finish(result, preview, cx);
                        });
                    });
                },
                move |message: String, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = failure.update(cx, |store, cx| {
                            if store.generation != stamp {
                                return;
                            }
                            store.is_submitting = false;
                            store.submission_error = Some(message);
                            cx.notify();
                        });
                    });
                },
                cx,
            );
        });
    }

    pub fn open_pack(&mut self, preview: &RisePackInvitePreview, cx: &mut Context<Self>) {
        let Some(pack) = preview.pack.as_ref().filter(|pack| !pack.id.is_empty()) else {
            return;
        };
        let pack_id = pack.id.clone();
        let kind = Self::kind(pack);
        self.dismiss(cx);
        StickerPackSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present(&pack_id, kind, cx));
    }

    pub fn kind(pack: &ProtoPackRecord) -> PackKind {
        PackKind::from_wire(&pack.pack_kind.to_lowercase()).unwrap_or(PackKind::Sticker)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.generation += 1;
        let stamp = self.generation;
        self.phase = PackInviteSheetStorePhase::Loading;
        self.is_submitting = false;
        self.submission_error = None;
        cx.notify();
        let success = cx.weak_entity();
        let failure = cx.weak_entity();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_invite_preview(
                &token,
                move |preview: RisePackInvitePreview, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = success.update(cx, |store, cx| store.preview_arrived(stamp, preview, cx));
                    });
                },
                move |message: String, cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = failure.update(cx, |store, cx| {
                            if store.generation != stamp {
                                return;
                            }
                            store.phase = PackInviteSheetStorePhase::Failed(message);
                            cx.notify();
                        });
                    });
                },
                cx,
            );
        });
    }

    fn preview_arrived(&mut self, stamp: u64, preview: RisePackInvitePreview, cx: &mut Context<Self>) {
        if self.generation != stamp {
            return;
        }
        if let Some(pack) = preview.pack.as_ref() {
            let pack_id = pack.id.clone();
            let owner_id = preview.owner_id.clone();
            StickerPackSheetStore::shared(cx).update(cx, |sheet, cx| sheet.note_owner(&pack_id, &owner_id, cx));
        }
        self.phase = PackInviteSheetStorePhase::Preview(preview);
        cx.notify();
    }

    fn finish(&mut self, result: RisePackJoinResult, preview: RisePackInvitePreview, cx: &mut Context<Self>) {
        if result.result == stickers_service::STICKER_PACK_JOIN_RESULT_REQUESTED {
            let mut waiting = preview;
            waiting.my_status = stickers_service::STICKER_PACK_INVITE_STATUS_PENDING.to_owned();
            self.phase = PackInviteSheetStorePhase::Preview(waiting);
            cx.notify();
            let _ = notification(NotificationKind::Success);
            Notifier::send(
                tr("sticker_pack_preview_requested"),
                NotifierSendOptions {
                    content: Some(tr("sticker_pack_preview_requested_hint")),
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        let mut joined = preview;
        if let Some(pack) = result.pack {
            joined.pack = Some(pack);
        }
        if result.result == stickers_service::STICKER_PACK_JOIN_RESULT_JOINED {
            let _ = notification(NotificationKind::Success);
            Notifier::send(
                tr("sticker_pack_preview_joined"),
                NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
        self.open_pack(&joined, cx);
    }

    fn snapshot(&self) -> Option<PackInviteSheetSnapshot> {
        self.is_presented().then(|| PackInviteSheetSnapshot {
            phase: self.phase.clone(),
            is_submitting: self.is_submitting,
            submission_error: self.submission_error.clone(),
        })
    }
}

/// what a leaving sheet keeps drawing while it slides out, the store is already reset by then
#[derive(Clone, PartialEq)]
struct PackInviteSheetSnapshot {
    phase: PackInviteSheetStorePhase,
    is_submitting: bool,
    submission_error: Option<String>,
}

/// bottom sheet host mounted once in the root view, next to the playlist invite sheet host
pub struct PackInviteSheetHost {
    on_open_profile: PackInviteOpenProfile,
    store: Entity<PackInviteSheetStore>,
    scrim: Presence<()>,
    panel: Presence<PackInviteSheetSnapshot>,
    drag_offset: Animated<f32>,
    focus_handle: FocusHandle,
    focus_before_presentation: Option<FocusHandle>,
    _store_observation: Subscription,
}

impl PackInviteSheetHost {
    pub fn new(
        on_open_profile: impl Fn(String, String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = PackInviteSheetStore::shared(cx);
        let store_observation = cx.observe_in(&store, window, |this, _, window, cx| this.store_changed(window, cx));
        let mut host = Self {
            on_open_profile: Rc::new(on_open_profile),
            store,
            scrim: Presence::new(None).with_transition(Transition::opacity()),
            panel: Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom)),
            drag_offset: Animated::new(0.0),
            focus_handle: cx.focus_handle(),
            focus_before_presentation: None,
            _store_observation: store_observation,
        };
        host.store_changed(window, cx);
        host
    }

    fn store_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.store.read(cx).snapshot();
        let presentable = snapshot.is_some();
        let was_presentable = self.panel.is_present();
        let now = RiseClock::monotonic();
        let animation = spring(PRESENTATION_RESPONSE, PRESENTATION_DAMPING);
        self.scrim.set_in(presentable.then_some(()), animation, now, cx);
        self.panel.set_in(snapshot, animation, now, cx);
        if presentable != was_presentable {
            self.drag_offset.jump(0.0);
            if presentable {
                self.focus_before_presentation = window.focused(cx);
                window.focus(&self.focus_handle, cx);
            } else if let Some(focus) = self.focus_before_presentation.take() {
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    fn dismiss(cx: &mut App) {
        PackInviteSheetStore::shared(cx).update(cx, |store, cx| store.dismiss(cx));
    }

    fn drag_gesture(&self, cx: &mut Context<Self>, is_submitting: bool) -> DragGesture {
        let changed = cx.entity();
        let ended = cx.entity();
        DragGesture::new(DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value, _, cx| {
                if is_submitting {
                    return;
                }
                changed.update(cx, |host, cx| {
                    host.drag_offset.jump(f32::from(value.translation.height).max(0.0));
                    cx.notify();
                });
            })
            .on_ended(move |value, _, cx| {
                if is_submitting {
                    return;
                }
                if f32::from(value.translation.height) > DRAG_DISMISS_DISTANCE {
                    Self::dismiss(cx);
                    return;
                }
                ended.update(cx, |host, cx| {
                    let now = RiseClock::monotonic();
                    host.drag_offset.set_in(0.0, spring(DRAG_RETURN_RESPONSE, DRAG_RETURN_DAMPING), now, cx);
                    cx.notify();
                });
            })
    }

    fn panel_content(
        &self,
        snapshot: &PackInviteSheetSnapshot,
        presentable: bool,
        drag_offset: f32,
        theme: &AppTheme,
    ) -> AnyElement {
        let on_open_profile = Rc::clone(&self.on_open_profile);
        vstack(0.0)
            .id("pack-invite-sheet-panel")
            .when(presentable, |panel| panel.occlude())
            .w_full()
            .max_w(px(SHEET_MAX_WIDTH))
            .relative()
            .top(px(drag_offset))
            .pb(px(SHEET_BOTTOM_PADDING))
            .bg(theme.bg._100)
            .rounded(px(SHEET_CORNER_RADIUS))
            .child(div().mt(px(8.0)).w(px(38.0)).h(px(5.0)).capsule().bg(theme.text.secondary.opacity(0.35)))
            .child(PackInviteSheetContent::new(
                snapshot.clone(),
                Rc::new(move |user_id: String, tag: String, window: &mut Window, cx: &mut App| {
                    Self::dismiss(cx);
                    on_open_profile(user_id, tag, window, cx);
                }),
            ))
            .when(!presentable, |panel| panel.child(div().absolute().inset_0().occlude()))
            .into_any_element()
    }
}

impl Render for PackInviteSheetHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let presentable = self.panel.is_present();
        let is_submitting = self.panel.value(now).is_some_and(|snapshot| snapshot.is_submitting);
        let drag_offset = self.drag_offset.frame_value(now, window).max(0.0);
        let gesture = self.drag_gesture(cx, is_submitting);

        let scrim = self.scrim.render_transitioned(now, window, |_| {
            div()
                .id("pack-invite-sheet-scrim")
                .absolute()
                .inset_0()
                .bg(black().opacity(SCRIM_OPACITY))
                .when(presentable, |scrim| scrim.occlude().on_click(|_, _, cx| Self::dismiss(cx)))
        });
        let panel = self.panel.render_transitioned(now, window, |snapshot| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .px(px(16.0))
                .child(self.panel_content(snapshot, presentable, drag_offset, &theme).gesture("pack-invite-sheet-drag", gesture))
        });

        div()
            .id("pack-invite-sheet-host")
            .absolute()
            .inset_0()
            .size_full()
            .when(presentable, |host| {
                host.track_focus(&self.focus_handle).layer_keys(LayerCancel::dismiss(|_, cx| Self::dismiss(cx)))
            })
            .children(scrim)
            .children(panel)
    }
}

#[derive(IntoElement)]
struct PackInviteSheetContent {
    snapshot: PackInviteSheetSnapshot,
    on_open_profile: PackInviteOpenProfile,
}

impl PackInviteSheetContent {
    fn new(snapshot: PackInviteSheetSnapshot, on_open_profile: PackInviteOpenProfile) -> Self {
        Self { snapshot, on_open_profile }
    }

    fn preview_state(&self, preview: &RisePackInvitePreview, theme: &AppTheme) -> AnyElement {
        let pack = preview.pack.as_ref();
        let is_emoji = pack.is_some_and(|pack| matches!(PackInviteSheetStore::kind(pack), PackKind::Emoji));
        let source = pack.map(|pack| pack.preview_url.clone()).unwrap_or_default();
        let title = pack.map(|pack| pack.title.clone()).unwrap_or_default();
        vstack(14.0)
            .w_full()
            .child(div().pt(px(6.0)).child(PackInviteCover { source, title: title.clone(), is_emoji, size: COVER_SIZE }))
            .child(
                vstack(7.0)
                    .child(
                        div().px(px(10.0)).py(px(5.0)).bg(theme.primary._100.opacity(0.13)).capsule().child(
                            MainText::new(tr(if is_emoji { "emoji_pack_preview_type" } else { "sticker_pack_preview_type" }))
                                .px(12.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.primary._100),
                        ),
                    )
                    .child(
                        MainText::new(title)
                            .px(21.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary)
                            .number_of_lines(2),
                    )
                    .child(
                        MainText::new(tr_plural("sticker_pack_preview_coauthors", preview.coauthor_count))
                            .px(14.0)
                            .color(theme.text.secondary),
                    ),
            )
            .children(preview.owner.as_ref().map(|owner| self.owner_row(owner, theme)))
            .child(div().w_full().pt(px(2.0)).child(self.action_area(preview, theme)))
            .into_any_element()
    }

    fn owner_row(&self, owner: &RisePackUserRef, theme: &AppTheme) -> AnyElement {
        let on_open_profile = Rc::clone(&self.on_open_profile);
        let owner_id = owner.id.clone();
        let owner_tag = owner.tag.clone();
        plain("pack-invite-owner")
            .on_press(move |window, cx| on_open_profile(owner_id.clone(), owner_tag.clone(), window, cx))
            .child(
                hstack(8.0)
                    .px(px(12.0))
                    .py(px(7.0))
                    .bg(theme.bg._200)
                    .capsule()
                    .child(
                        UserLogoUi::new("pack-invite-owner-logo")
                            .source(SharedString::from(owner.logo.clone()))
                            .name(SharedString::from(owner.name.clone()))
                            .user_id(SharedString::from(owner.id.clone()))
                            .size(26.0)
                            .show_border(false),
                    )
                    .child(MainText::new(tr("music_playlist_preview_by_owner")).px(13.0).color(theme.text.secondary))
                    .child(
                        MainText::new(PackCollaborationUserLabel::display_name(Some(owner)))
                            .px(13.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(10.0), theme.text.secondary.opacity(0.7), false)),
            )
            .into_any_element()
    }

    fn action_area(&self, preview: &RisePackInvitePreview, theme: &AppTheme) -> AnyElement {
        let submission_error = self.snapshot.submission_error.as_ref().filter(|error| !error.is_empty()).map(|error| {
            div().py(px(6.0)).child(MainText::new(error.clone()).px(14.0).tac(TextAlignT::Center).color(theme.semantic.error_100))
        });
        let status = preview.my_status.as_str();
        let action = if status == stickers_service::STICKER_PACK_INVITE_STATUS_OWNER
            || status == stickers_service::STICKER_PACK_INVITE_STATUS_COAUTHOR
        {
            let open_preview = preview.clone();
            vstack(10.0)
                .w_full()
                .child(
                    MainText::new(tr("sticker_pack_preview_already"))
                        .px(13.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                )
                .child(self.primary_button("pack-invite-open", tr("sticker_pack_preview_open"), theme, move |cx| {
                    PackInviteSheetStore::shared(cx).update(cx, |store, cx| store.open_pack(&open_preview, cx));
                }))
                .into_any_element()
        } else if status == stickers_service::STICKER_PACK_INVITE_STATUS_PENDING {
            div()
                .w_full()
                .py(px(10.0))
                .child(
                    MainText::new(tr("music_playlist_preview_pending"))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(3),
                )
                .into_any_element()
        } else {
            let hint = preview.creates_join_request.then(|| {
                MainText::new(tr("sticker_pack_preview_request_hint"))
                    .px(13.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(3)
            });
            let title = tr(if preview.creates_join_request { "sticker_pack_preview_request" } else { "sticker_pack_preview_join" });
            vstack(10.0)
                .w_full()
                .children(hint)
                .child(self.primary_button("pack-invite-join", title, theme, |cx| {
                    PackInviteSheetStore::shared(cx).update(cx, |store, cx| store.submit(cx));
                }))
                .into_any_element()
        };
        vstack(0.0).w_full().children(submission_error).child(action).into_any_element()
    }

    fn primary_button(
        &self,
        id: &'static str,
        title: String,
        theme: &AppTheme,
        action: impl Fn(&mut App) + 'static,
    ) -> AnyElement {
        let is_submitting = self.snapshot.is_submitting;
        let label = if is_submitting {
            progress_view().tint(theme.content_on(theme.primary._100)).into_any_element()
        } else {
            MainText::new(title)
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.content_on(theme.primary._100))
                .into_any_element()
        };
        plain(id)
            .w_full()
            .opacity(if is_submitting { 0.85 } else { 1.0 })
            .disabled(is_submitting)
            .on_press(move |_, cx| {
                if is_submitting {
                    return;
                }
                action(cx);
            })
            .child(
                div()
                    .w_full()
                    .h(px(52.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_rect(15.0)
                    .bg(theme.primary._100)
                    .child(label),
            )
            .into_any_element()
    }

    fn loading_state() -> AnyElement {
        let skeleton = pass_through()
            .w_full()
            .min_h(px(220.0))
            .pt(px(6.0))
            .child(
                vstack(14.0)
                    .w_full()
                    .child(SkeletonBlock::new(88.0, 88.0, 20.0))
                    .child(vstack(9.0).child(SkeletonLine::new(170.0, 16.0)).child(SkeletonLine::new(108.0, 11.0)))
                    .child(div().pt(px(2.0)).child(SkeletonBlock::new(150.0, 40.0, 20.0)))
                    .child(div().w_full().pt(px(4.0)).child(SkeletonBlock::new(None::<f32>, 52.0, 15.0)))
                    .skeleton_shimmer(None),
            );
        div()
            .id("pack-invite-loading")
            .aria_label(SharedString::from(tr("sticker_pack_preview_title")))
            .w_full()
            .child(skeleton)
            .into_any_element()
    }

    fn failed_state(message: &str) -> AnyElement {
        let retry: ButtonAction = Rc::new(|_: &mut Window, cx: &mut App| {
            PackInviteSheetStore::shared(cx).update(cx, |store, cx| store.retry(cx));
        });
        div()
            .w_full()
            .min_h(px(220.0))
            .child(PlaylistCollaborationErrorState::new("pack-invite-error", message.to_owned(), Some(retry)))
            .into_any_element()
    }
}

impl RenderOnce for PackInviteSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let content = match &self.snapshot.phase {
            PackInviteSheetStorePhase::Loading => Self::loading_state(),
            PackInviteSheetStorePhase::Preview(preview) => self.preview_state(preview, &theme),
            PackInviteSheetStorePhase::Failed(message) => Self::failed_state(message),
        };
        vstack(0.0).w_full().px(px(22.0)).pt(px(10.0)).pb(px(8.0)).child(content)
    }
}

#[derive(IntoElement)]
struct PackInviteCover {
    source: String,
    title: String,
    is_emoji: bool,
    size: f32,
}

impl RenderOnce for PackInviteCover {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        if !trimming_whitespaces_and_newlines(&self.source).is_empty() {
            return PlaylistCollaborationCover::new("pack-invite-cover", self.source).size(self.size).into_any_element();
        }
        let symbol = if self.is_emoji { "face.smiling.fill" } else { "square.grid.2x2.fill" };
        div()
            .id("pack-invite-cover-mark")
            .aria_label(SharedString::from(self.title))
            .size(px(self.size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_rect(self.size * 0.22)
            .bg(theme.bg._300)
            .children(IconUi::sized(symbol, px(self.size * 0.38), theme.text.secondary, false))
            .into_any_element()
    }
}
