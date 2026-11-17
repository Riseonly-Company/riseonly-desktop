use gpui::{
    AnyElement, App, AsyncApp, Context, Entity, ExternalPaths, ImageSource, ObjectFit, Subscription, Window, black, img, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{ChatBackdropLayers, ChatBackdropLayout};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_image_layer::ChatWallpaperBitmapCache;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::user::engine::rise_user_profile_banner_upload::{
    RiseProfileBannerDraft, RiseProfileBannerUploadError, RiseUserProfileBannerUpload,
};
use crate::modules::user::pages::profile::settings::customization_settings::settings::chat_wallpapers_settings::chat_wallpapers_settings::ChatWallpapersSettings;
use crate::modules::user::pages::profile::settings::profile_settings::profile_avatar_picker_block::ProfileAvatarPickerBlock;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const BANNER_HEIGHT: f32 = 168.0;
const BANNER_RADIUS: f32 = 26.0;
const AVATAR_HANG: f32 = 44.0;
const PREVIEW_PIXEL_DIMENSION: i64 = 1_536;

struct ProfileBannerPickerBlockState {
    picker_open: bool,
    pending_local_url: Option<Url>,
    pending_preview: Option<CleverImage>,
    is_uploading: bool,
    upload_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProfileBannerPickerBlockState {
    fn new(cx: &mut Context<Self>) -> Self {
        let sandbox = GuideSandbox::shared(cx);
        Self {
            picker_open: false,
            pending_local_url: None,
            pending_preview: None,
            is_uploading: false,
            upload_error: None,
            _subscriptions: vec![cx.observe(&sandbox, |_, _, cx| cx.notify())],
        }
    }

    fn set_picker_open(&mut self, picker_open: bool, cx: &mut Context<Self>) {
        self.picker_open = picker_open;
        cx.notify();
    }
}

/// profile editing header, banner plus avatar hung over its bottom edge
/// removal asks first since the banner has no album to restore from
/// upload isnt tied to save
#[derive(IntoElement)]
pub struct ProfileBannerPickerBlock {
    banner_source: Option<String>,
    avatar: AnyElement,
}

impl ProfileBannerPickerBlock {
    pub fn new(banner_source: Option<String>, avatar: impl IntoElement) -> Self {
        Self { banner_source, avatar: avatar.into_any_element() }
    }

    fn has_banner(&self, cx: &mut App) -> bool {
        let sandbox = GuideSandbox::shared(cx);
        let sandbox = sandbox.read(cx);
        if sandbox.owns_profile_look_scene() {
            return sandbox.practice_banner_url().is_some();
        }
        trimming_whitespaces_and_newlines(self.banner_source.as_deref().unwrap_or_default()).to_lowercase().starts_with("http")
    }

    fn local_file_image(source: String, maximum_pixel_dimension: i64) -> AnyElement {
        div()
            .size_full()
            .rounded(px(BANNER_RADIUS))
            .bg(black())
            .overflow_hidden()
            .child(
                CleverImageUi::new(source)
                    .content_mode(ContentMode::Fill)
                    .corner_radius(BANNER_RADIUS)
                    .persist_to_disk(false)
                    .shows_progress_indicator(false)
                    .maximum_decode_pixel_dimension(maximum_pixel_dimension)
                    .shows_failure_icon(false),
            )
            .into_any_element()
    }

    /// paints through the same layer as the profile header so the preview matches. uploading
    /// banner is drawn from the exported file
    fn banner_card(&self, state: &Entity<ProfileBannerPickerBlockState>, cx: &mut App) -> AnyElement {
        let sandbox = GuideSandbox::shared(cx);
        if let Some(practice) = sandbox.read(cx).practice_banner_url() {
            return Self::local_file_image(practice.absolute_string().to_owned(), PREVIEW_PIXEL_DIMENSION);
        }
        let state = state.read(cx);
        if let Some(pending_local_url) = &state.pending_local_url {
            if let Some(pending_preview) = &state.pending_preview {
                return div()
                    .size_full()
                    .rounded(px(BANNER_RADIUS))
                    .bg(black())
                    .overflow_hidden()
                    .child(
                        img(ImageSource::Render(pending_preview.clone()))
                            .size_full()
                            .rounded(px(BANNER_RADIUS))
                            .object_fit(ObjectFit::Cover),
                    )
                    .into_any_element();
            }
            return Self::local_file_image(pending_local_url.absolute_string().to_owned(), PREVIEW_PIXEL_DIMENSION);
        }
        ChatBackdropLayers::new(ChatBackdropLayout::ProfileBanner)
            .banner_url(self.banner_source.clone())
            .corner_radius(BANNER_RADIUS)
            .into_any_element()
    }

    fn picker(state: &Entity<ProfileBannerPickerBlockState>, cx: &mut App) -> impl IntoElement + use<> {
        let complete_state = state.clone();
        pass_through().absolute().inset_0().child(
            MediaPickerSheet::new(
                "profile-banner-picker-sheet",
                Binding::entity(
                    state.clone(),
                    |state: &ProfileBannerPickerBlockState| state.picker_open,
                    |state: &mut ProfileBannerPickerBlockState, picker_open, cx| state.set_picker_open(picker_open, cx),
                ),
                MediaPickerConfiguration::profile_banner(),
                theme_store(cx),
                move |result: MediaPickerResult, _, cx| {
                    complete_state.update(cx, |state, cx| state.set_picker_open(false, cx));
                    if result.was_cancelled {
                        return;
                    }
                    let Some(item) =
                        result.items.iter().find(|item| item.kind == MediaPickerItemKind::Image && !item.is_file)
                    else {
                        return;
                    };
                    let content_type = ChatWallpapersSettings::image_content_type(item);
                    let draft = RiseProfileBannerDraft {
                        local_url: item.url.clone(),
                        filename: ChatWallpapersSettings::filename(item, content_type),
                        content_type: content_type.to_owned(),
                    };
                    let upload_state = complete_state.clone();
                    cx.defer(move |cx| Self::upload(upload_state, draft, cx));
                },
            )
            .experimental_bottom_sheet(true),
        )
    }

    fn upload(state: Entity<ProfileBannerPickerBlockState>, draft: RiseProfileBannerDraft, cx: &mut App) {
        let sandbox = GuideSandbox::shared(cx);
        if sandbox.read(cx).owns_profile_look_scene() {
            sandbox.update(cx, |sandbox, cx| sandbox.adopt_practice_banner(Some(draft.local_url), cx));
            return;
        }
        let account_id = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|id| !id.is_empty());
        let Some(account_id) = account_id else {
            return;
        };
        state.update(cx, |state, cx| {
            state.upload_error = None;
            state.pending_local_url = Some(draft.local_url.clone());
            state.pending_preview = None;
            state.is_uploading = true;
            cx.notify();
        });

        let preview_url = draft.local_url.clone();
        let preview_state = state.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let preview = Self::decode_preview(&preview_url, cx).await;
            preview_state.update(cx, |state, cx| {
                if state.pending_local_url.as_ref() == Some(&preview_url) {
                    state.pending_preview = preview;
                    cx.notify();
                }
            });
        })
        .detach();

        cx.spawn(async move |cx: &mut AsyncApp| {
            let upload_error = match RiseUserProfileBannerUpload::upload(&draft, &account_id, cx).await {
                Ok(remote_url) => {
                    let saved_url = remote_url.clone();
                    cx.update(|cx| {
                        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                            interactions.save_profile_edits(None, None, None, None, None, None, Some(saved_url), cx);
                        });
                    });
                    let _ = CleverImageBitmapLoader::load(
                        &remote_url,
                        true,
                        true,
                        SaiAccountCacheScope::active_user_id(),
                        SaiAccountCacheScope::active_epoch(),
                        Some(ChatWallpaperBitmapCache::MAXIMUM_PIXEL_DIMENSION),
                        CleverImageCachePolicy::Standard,
                        CleverImageWorkLane::Visible,
                        cx,
                    )
                    .await;
                    None
                }
                Err(error) => {
                    let message = error.downcast_ref::<RiseProfileBannerUploadError>().map(ToString::to_string).unwrap_or_default();
                    let message = trimming_whitespaces_and_newlines(&message);
                    Some(if message.is_empty() { tr("profile_banner_upload_failed") } else { message.to_owned() })
                }
            };
            state.update(cx, |state, cx| {
                state.is_uploading = false;
                state.pending_local_url = None;
                state.pending_preview = None;
                if upload_error.is_some() {
                    state.upload_error = upload_error;
                }
                cx.notify();
            });
            RiseUserProfileBannerUpload::discard_staged_file(&draft.local_url);
        })
        .detach();
    }

    async fn decode_preview(url: &Url, cx: &AsyncApp) -> Option<CleverImage> {
        let path = url.to_file_path()?;
        CleverImageBitmapLoader::prepare_bitmap(
            move || {
                let data = std::fs::read(&path).ok()?;
                CleverImageBitmapDecoder::decode(&data, Some(PREVIEW_PIXEL_DIMENSION))
            },
            cx,
        )
        .await
    }

    fn remove_banner(state: &Entity<ProfileBannerPickerBlockState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.upload_error = None;
            cx.notify();
        });
        let sandbox = GuideSandbox::shared(cx);
        if sandbox.read(cx).owns_profile_look_scene() {
            sandbox.update(cx, |sandbox, cx| sandbox.adopt_practice_banner(None, cx));
            return;
        }
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.save_profile_edits(None, None, None, None, None, None, Some(String::new()), cx);
        });
    }

    /// desktop only, an image file dropped on the card goes where the sheet result goes
    fn import_external_paths(
        state: Entity<ProfileBannerPickerBlockState>,
        paths: &ExternalPaths,
        cx: &mut App,
    ) {
        if state.read(cx).is_uploading {
            return;
        }
        let Some((local_url, filename, content_type)) =
            paths.paths().iter().find_map(|path| ProfileAvatarPickerBlock::local_image_file(path))
        else {
            return;
        };
        Self::upload(state, RiseProfileBannerDraft { local_url, filename, content_type }, cx);
    }
}

impl RenderOnce for ProfileBannerPickerBlock {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("profile-banner-picker-block-state", cx, |_, cx| {
            ProfileBannerPickerBlockState::new(cx)
        });
        let has_banner = self.has_banner(cx);
        let banner_card = self.banner_card(&state, cx);
        let picker = Self::picker(&state, cx);
        let (is_uploading, upload_error) = {
            let state = state.read(cx);
            (state.is_uploading, state.upload_error.clone().filter(|error| !error.is_empty()))
        };

        let press_state = state.clone();
        let remove_state = state.clone();
        let drop_state = state.clone();

        vstack(10.0)
            .relative()
            .w_full()
            .child(
                div().w_full().pl(px(12.0)).pr(px(12.0)).pb(px(AVATAR_HANG)).child(
                    div()
                        .id("profile-banner-card")
                        .relative()
                        .w_full()
                        .h(px(BANNER_HEIGHT))
                        .child(
                            div()
                                .id("profile-banner-select-hint")
                                .w_full()
                                .h(px(BANNER_HEIGHT))
                                .tooltip_label(tr("profile_banner_select"))
                                .child(
                                    plain("profile-banner-select")
                                        .disabled(is_uploading)
                                        .on_press(move |_, cx| {
                                            press_state.update(cx, |state, cx| {
                                                if !state.is_uploading {
                                                    state.set_picker_open(true, cx);
                                                }
                                            });
                                        })
                                        .w_full()
                                        .h(px(BANNER_HEIGHT))
                                        .child(
                                            Shape::continuous(BANNER_RADIUS)
                                                .round(div())
                                                .size_full()
                                                .overflow_hidden()
                                                .child(banner_card),
                                        )
                                        .guide_anchor(RiseGuideAnchor::ProfileBanner),
                                ),
                        )
                        .when(is_uploading, |card| {
                            card.child(
                                Shape::continuous(BANNER_RADIUS)
                                    .round(pass_through())
                                    .absolute()
                                    .inset_0()
                                    .overflow_hidden()
                                    .bg(black().opacity(0.35))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(LoaderUi::new().size(LoaderUiSize::Small).color(Some(white()))),
                            )
                        })
                        .when(has_banner && !is_uploading, |card| {
                            card.child(
                                div()
                                    .id("profile-banner-remove-hint")
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .occlude()
                                    .tooltip_label(tr("profile_banner_remove"))
                                    .child(
                                        plain("profile-banner-remove")
                                            .on_press(move |_, cx| {
                                                let _ = selection_changed();
                                                let confirmed_state = remove_state.clone();
                                                NativeConfirmAlert::present_delete(
                                                    &tr("profile_banner_remove"),
                                                    "",
                                                    None,
                                                    None,
                                                    false,
                                                    None,
                                                    None,
                                                    move |cx| Self::remove_banner(&confirmed_state, cx),
                                                    cx,
                                                );
                                            })
                                            .p(px(10.0))
                                            .child(
                                                div()
                                                    .size(px(34.0))
                                                    .rounded_full()
                                                    .bg(black().opacity(0.45))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .children(IconUi::sized("trash", px(14.0), white(), false)),
                                            ),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(px(-AVATAR_HANG))
                                .flex()
                                .justify_center()
                                .child(self.avatar),
                        )
                        .on_drop(move |paths: &ExternalPaths, _, cx| {
                            Self::import_external_paths(drop_state.clone(), paths, cx);
                        }),
                ),
            )
            .when_some(upload_error, |block, upload_error| {
                block.child(
                    div().w_full().pl(px(18.0)).pr(px(18.0)).child(
                        MainText::new(upload_error)
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(theme.semantic.error_100),
                    ),
                )
            })
            .child(picker)
    }
}
