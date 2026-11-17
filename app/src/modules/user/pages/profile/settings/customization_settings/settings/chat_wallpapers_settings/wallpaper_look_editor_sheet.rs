use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Subscription, Window, black, px, relative, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore, alpha};
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::blur::BlurModifiers;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::WallpaperSampleBubbles;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatWallpaperCatalogEntry};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::RiseChatWallpaper;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_interactions::theme_wallpapers_interactions::ThemeWallpapersInteractionsStore;
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_services::theme_wallpapers_services::ThemeWallpapersServicesStore;

const FULL_BLUR_RADIUS: f32 = 26.0;
const FULL_BLUR_SCALE_GROWTH: f32 = 0.12;
const MAXIMUM_DECODE_PIXEL_DIMENSION: i64 = 2_048;

#[derive(Clone)]
pub enum WallpaperLookEditorSheetTarget {
    Draft(RiseChatWallpaperDraft),
    Own { id: String },
    ChatEntry { entry: ChatWallpaperCatalogEntry, chat_id: String, topic_id: Option<String> },
}

pub type WallpaperLookEditorSheetUpload = Rc<dyn Fn(ChatWallpaperLook, &mut Window, &mut App)>;
pub type WallpaperLookEditorSheetDiscard = Rc<dyn Fn(&mut Window, &mut App)>;

/// sheet state,
/// dismiss_requested carries a dismissal from callbacks with no window and the next render
/// does it
struct WallpaperLookEditorState {
    dim: f64,
    blur: f64,
    did_seed: bool,
    did_finish: bool,
    is_applying: bool,
    apply_error: Option<String>,
    dismiss_requested: bool,
    _subscriptions: Vec<Subscription>,
}

impl WallpaperLookEditorState {
    fn new(cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        let services = ThemeWallpapersServicesStore::shared(cx);
        let chat_services = ChatServicesStore::shared(cx);
        Self {
            dim: ChatWallpaperLook::DEFAULT_DIM_PERCENT as f64,
            blur: 0.0,
            did_seed: false,
            did_finish: false,
            is_applying: false,
            apply_error: None,
            dismiss_requested: false,
            _subscriptions: vec![
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&chat_services, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn look(&self) -> ChatWallpaperLook {
        ChatWallpaperLook::new(self.dim.round() as i64, self.blur.round() as i64).clamped()
    }

    /// answer of a chat call, done closes the sheet and a refusal stays with its text
    fn finish_chat_call(&mut self, ok: bool, message: Option<String>, cx: &mut Context<Self>) {
        self.is_applying = false;
        if ok {
            self.did_finish = true;
            self.dismiss_requested = true;
        } else {
            self.apply_error = Some(message.unwrap_or_else(|| tr("wallpaper_chat_apply_failed")));
        }
        cx.notify();
    }
}

fn own_wallpaper(target: &WallpaperLookEditorSheetTarget, cx: &mut App) -> Option<RiseChatWallpaper> {
    let WallpaperLookEditorSheetTarget::Own { id } = target else {
        return None;
    };
    let services = ThemeWallpapersServicesStore::shared(cx);
    services.read(cx).wallpaper(id, cx)
}

fn image_source(target: &WallpaperLookEditorSheetTarget, own: Option<&RiseChatWallpaper>) -> Option<String> {
    match target {
        WallpaperLookEditorSheetTarget::Draft(draft) => Some(draft.local_url.absolute_string().to_owned()),
        WallpaperLookEditorSheetTarget::Own { .. } => own.map(|wallpaper| wallpaper.display_url().to_owned()),
        WallpaperLookEditorSheetTarget::ChatEntry { entry, .. } => Some(entry.display_url().to_owned()),
    }
}

fn preview_chat(target: &WallpaperLookEditorSheetTarget, cx: &mut App) -> Option<ChatInfo> {
    let WallpaperLookEditorSheetTarget::ChatEntry { chat_id, .. } = target else {
        return None;
    };
    let services = ChatServicesStore::shared(cx);
    services.read(cx).chat_info(chat_id, cx)
}

fn can_delete(target: &WallpaperLookEditorSheetTarget, own: Option<&RiseChatWallpaper>) -> bool {
    match target {
        WallpaperLookEditorSheetTarget::Draft(_) => false,
        WallpaperLookEditorSheetTarget::Own { .. } => {
            own.is_some_and(|wallpaper| !wallpaper.is_built_in && !wallpaper.is_pending)
        }
        WallpaperLookEditorSheetTarget::ChatEntry { .. } => true,
    }
}

fn seed_if_needed(
    target: &WallpaperLookEditorSheetTarget,
    state: &Entity<WallpaperLookEditorState>,
    cx: &mut App,
) {
    if state.read(cx).did_seed {
        return;
    }
    let seed = match target {
        WallpaperLookEditorSheetTarget::Draft(_) => ChatWallpaperLook::DEFAULT_LOOK,
        WallpaperLookEditorSheetTarget::Own { .. } => {
            let Some(wallpaper) = own_wallpaper(target, cx) else {
                return;
            };
            wallpaper.look()
        }
        WallpaperLookEditorSheetTarget::ChatEntry { entry, .. } => entry.look,
    };
    state.update(cx, |state, cx| {
        state.did_seed = true;
        state.dim = seed.dim_percent.min(ChatWallpaperLook::MAX_DIM_PERCENT) as f64;
        state.blur = seed.blur_percent as f64;
        cx.notify();
    });
}

fn delete_target(
    target: &WallpaperLookEditorSheetTarget,
    state: &Entity<WallpaperLookEditorState>,
    cx: &mut App,
) {
    match target {
        WallpaperLookEditorSheetTarget::Draft(_) => {}
        WallpaperLookEditorSheetTarget::Own { id } => {
            state.update(cx, |state, cx| {
                state.did_finish = true;
                state.dismiss_requested = true;
                cx.notify();
            });
            ThemeWallpapersInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.delete(id, cx));
        }
        WallpaperLookEditorSheetTarget::ChatEntry { entry, chat_id, topic_id } => {
            state.update(cx, |state, cx| {
                state.is_applying = true;
                state.apply_error = None;
                cx.notify();
            });
            let answered = state.clone();
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.delete_chat_wallpaper_entry(
                    chat_id,
                    topic_id.as_deref(),
                    &entry.id,
                    Some(Box::new(move |ok, message, cx: &mut App| {
                        answered.update(cx, |state, cx| state.finish_chat_call(ok, message, cx));
                    })),
                    cx,
                );
            });
        }
    }
}

#[derive(IntoElement)]
pub struct WallpaperLookEditorSheet {
    target: WallpaperLookEditorSheetTarget,
    on_upload: Option<WallpaperLookEditorSheetUpload>,
    on_discard: Option<WallpaperLookEditorSheetDiscard>,
}

impl WallpaperLookEditorSheet {
    pub fn new(target: WallpaperLookEditorSheetTarget) -> Self {
        Self { target, on_upload: None, on_discard: None }
    }

    /// draft only, called with the chosen look and the caller uploads
    pub fn on_upload(mut self, on_upload: impl Fn(ChatWallpaperLook, &mut Window, &mut App) + 'static) -> Self {
        self.on_upload = Some(Rc::new(on_upload));
        self
    }

    /// draft only, the sheet went away without uploading
    pub fn on_discard(mut self, on_discard: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_discard = Some(Rc::new(on_discard));
        self
    }

    fn delete_button(
        target: WallpaperLookEditorSheetTarget,
        state: Entity<WallpaperLookEditorState>,
        is_applying: bool,
    ) -> impl IntoElement {
        plain("wallpaper-look-delete")
            .disabled(is_applying)
            .on_press(move |_, cx| {
                let target = target.clone();
                let state = state.clone();
                NativeConfirmAlert::present_delete(
                    &tr("chat_wallpaper_delete"),
                    "",
                    Some(tr("chat_wallpaper_delete")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| delete_target(&target, &state, cx),
                    cx,
                );
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("trash", px(16.0), white(), false))
                    .liquid_glass_nav_circle(Some(1.0)),
            )
    }

    /// picture fills and is cropped, scales up as blur grows so softened edges stay off screen
    fn preview(image_source: Option<String>, blur: f64, dim: f64) -> impl IntoElement {
        let fraction = (blur / ChatWallpaperLook::MAX_PERCENT as f64) as f32;
        let growth = fraction * FULL_BLUR_SCALE_GROWTH;
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(black())
            .children(image_source.map(|source| {
                div()
                    .absolute()
                    .left(relative(-growth / 2.0))
                    .top(relative(-growth / 2.0))
                    .w(relative(1.0 + growth))
                    .h(relative(1.0 + growth))
                    .child(
                        div()
                            .size_full()
                            .child(
                                CleverImageUi::new(source)
                                    .content_mode(ContentMode::Fill)
                                    .maximum_decode_pixel_dimension(MAXIMUM_DECODE_PIXEL_DIMENSION)
                                    .shows_failure_icon(false),
                            )
                            .blur(fraction * FULL_BLUR_RADIUS),
                    )
            }))
            .child(div().absolute().inset_0().bg(alpha(black(), (dim / 100.0) as f32)))
    }

    fn controls(state: &Entity<WallpaperLookEditorState>, blur: f64, dim: f64, theme: &AppTheme) -> impl IntoElement {
        vstack(14.0)
            .w_full()
            .px(px(16.0))
            .py(px(14.0))
            .bg(alpha(theme.bg._100, Material::UltraThin.painted_opacity()))
            .clip_shape(Shape::continuous(18.0))
            .child(Self::slider(
                "wallpaper-look-blur",
                tr("wallpaper_blur"),
                blur,
                Binding::entity(
                    state.clone(),
                    |state: &WallpaperLookEditorState| state.blur,
                    |state: &mut WallpaperLookEditorState, value: f64, cx| {
                        state.blur = value.min(ChatWallpaperLook::MAX_PERCENT as f64);
                        cx.notify();
                    },
                ),
            ))
            .child(Self::slider(
                "wallpaper-look-dim",
                tr("chat_wallpaper_dim"),
                dim,
                Binding::entity(
                    state.clone(),
                    |state: &WallpaperLookEditorState| state.dim,
                    |state: &mut WallpaperLookEditorState, value: f64, cx| {
                        state.dim = value.min(ChatWallpaperLook::MAX_DIM_PERCENT as f64);
                        cx.notify();
                    },
                ),
            ))
            .guide_anchor(RiseGuideAnchor::WallpaperEditorLook)
    }

    /// both tracks span 0 to 100 percent so the label is the thumb position. dimming stops at its ceiling
    fn slider(id: &'static str, title: String, value: f64, binding: Binding<f64>) -> impl IntoElement {
        vstack(6.0)
            .w_full()
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .child(MainText::new(title).px(14.0).color(white()))
                    .child(spacer())
                    .child(MainText::new(format!("{}%", value.round() as i64)).px(13.0).color(alpha(white(), 0.85))),
            )
            .child(
                slider(id, binding, 0.0..=ChatWallpaperLook::MAX_PERCENT as f64)
                    .step(1.0)
                    .tint(white()),
            )
    }

    fn add_button(
        action_title: String,
        is_applying: bool,
        on_press: impl Fn(&mut Window, &mut App) + 'static,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let on_primary = theme.content_on(theme.primary._100);
        let content = div()
            .w_full()
            .min_h(px(52.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.primary._100)
            .clip_shape(Shape::Capsule);
        let content = if is_applying {
            content.child(LoaderUi::new().size(LoaderUiSize::Small).color(Some(on_primary)))
        } else {
            content.child(MainText::new(action_title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(on_primary))
        };
        plain("wallpaper-look-apply")
            .w_full()
            .disabled(is_applying)
            .on_press(on_press)
            .child(content)
            .guide_anchor(RiseGuideAnchor::WallpaperEditorApply)
    }
}

fn apply(
    target: &WallpaperLookEditorSheetTarget,
    on_upload: Option<&WallpaperLookEditorSheetUpload>,
    state: &Entity<WallpaperLookEditorState>,
    dismiss_action: &DismissAction,
    window: &mut Window,
    cx: &mut App,
) {
    if state.read(cx).is_applying {
        return;
    }
    let look = state.read(cx).look();
    match target {
        WallpaperLookEditorSheetTarget::Draft(_) => {
            state.update(cx, |state, _| state.did_finish = true);
            if let Some(on_upload) = on_upload {
                on_upload(look, window, cx);
            }
            dismiss_action.call_as_function(window, cx);
        }
        WallpaperLookEditorSheetTarget::Own { id } => {
            state.update(cx, |state, _| state.did_finish = true);
            ThemeWallpapersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.update_look(id, look, cx);
                interactions.select(id, cx);
            });
            dismiss_action.call_as_function(window, cx);
        }
        WallpaperLookEditorSheetTarget::ChatEntry { entry, chat_id, topic_id } => {
            let mut entry = entry.clone();
            entry.look = look;
            state.update(cx, |state, cx| {
                state.is_applying = true;
                state.apply_error = None;
                cx.notify();
            });
            let answered = state.clone();
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.set_chat_wallpaper(
                    chat_id,
                    topic_id.as_deref(),
                    entry,
                    Some(Box::new(move |ok, message, cx: &mut App| {
                        answered.update(cx, |state, cx| state.finish_chat_call(ok, message, cx));
                    })),
                    cx,
                );
            });
        }
    }
}

impl RenderOnce for WallpaperLookEditorSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("wallpaper-look-editor-state", cx, |_, cx| WallpaperLookEditorState::new(cx));
        let dismiss_action = dismiss(cx);
        let target = self.target;
        let on_upload = self.on_upload;
        let on_discard = self.on_discard;

        let own = own_wallpaper(&target, cx);
        let source = image_source(&target, own.as_ref());
        let chat = preview_chat(&target, cx);
        let shows_delete = can_delete(&target, own.as_ref());
        let is_own_target = matches!(target, WallpaperLookEditorSheetTarget::Own { .. });
        let is_draft = matches!(target, WallpaperLookEditorSheetTarget::Draft(_));

        let (blur, dim, is_applying, apply_error, dismiss_requested) = {
            let state = state.read(cx);
            (state.blur, state.dim, state.is_applying, state.apply_error.clone(), state.dismiss_requested)
        };

        on_change("wallpaper-look-blur-change", blur, false, window, cx, |_, _, _, _| {
            GuideSignalBus::emit(RiseGuideSignal::WallpaperLookChanged);
        });
        on_change("wallpaper-look-dim-change", dim, false, window, cx, |_, _, _, _| {
            GuideSignalBus::emit(RiseGuideSignal::WallpaperLookChanged);
        });
        let gone_dismiss = dismiss_action.clone();
        on_change("wallpaper-look-own-gone", own.is_none(), false, window, cx, move |_, gone, window, cx| {
            if is_own_target && *gone {
                gone_dismiss.call_as_function(window, cx);
            }
        });
        let requested_dismiss = dismiss_action.clone();
        on_change("wallpaper-look-dismiss", dismiss_requested, false, window, cx, move |_, requested, window, cx| {
            if *requested {
                requested_dismiss.call_as_function(window, cx);
            }
        });

        let action_title = if is_draft { tr("wallpaper_upload_action") } else { tr("ctx_select") };
        let pressed_target = target.clone();
        let pressed_state = state.clone();
        let on_press = move |window: &mut Window, cx: &mut App| {
            apply(&pressed_target, on_upload.as_ref(), &pressed_state, &dismiss_action, window, cx);
        };

        let appeared_target = target.clone();
        let appeared_state = state.clone();
        let disappeared_state = state.clone();

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(black())
            .child(Self::preview(source, blur, dim))
            .when(shows_delete, |sheet| {
                sheet.child(
                    div()
                        .absolute()
                        .top(px(18.0))
                        .right(px(14.0))
                        .child(Self::delete_button(target.clone(), state.clone(), is_applying)),
                )
            })
            .child(
                vstack(12.0)
                    .absolute()
                    .inset_0()
                    .px(px(12.0))
                    .pb(px(20.0))
                    .child(spacer_min_h(0.0))
                    .child(div().w_full().flex_1().min_h_0().px(px(6.0)).child(WallpaperSampleBubbles::new().chat(chat)))
                    .child(Self::controls(&state, blur, dim, &theme))
                    .children(apply_error.filter(|error| !error.is_empty()).map(|error| {
                        div()
                            .px(px(12.0))
                            .py(px(8.0))
                            .bg(alpha(black(), 0.55))
                            .clip_shape(Shape::continuous(12.0))
                            .child(MainText::new(error).px(12.0).color(white()).number_of_lines(3))
                    }))
                    .child(Self::add_button(action_title, is_applying, on_press, &theme)),
            )
            .on_appear("wallpaper-look-appear", move |_, cx| {
                seed_if_needed(&appeared_target, &appeared_state, cx);
                GuideFacts::set(RiseGuideSignal::WallpaperEditorOpen, true, cx);
            })
            .on_disappear("wallpaper-look-disappear", move |window, cx| {
                GuideFacts::set(RiseGuideSignal::WallpaperEditorOpen, false, cx);
                if is_draft && !disappeared_state.read(cx).did_finish {
                    if let (Some(on_discard), Some(window)) = (&on_discard, window) {
                        on_discard(window, cx);
                    }
                }
            })
    }
}
