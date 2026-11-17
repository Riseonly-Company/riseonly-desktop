use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui::{App, Bounds, Pixels, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, SelectionFeedbackGenerator};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::app::router::navigation_store::NavigationStoreSettlingPush;
use crate::app::router::navigation_types::RootRoute;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerHeroSkin,
};
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{SharePayload, ShareSheetStore};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::collaborators::post_collaborator_menu::PostCollaboratorMenu;
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsPresenter;
use crate::modules::post::components::download::post_download_policy::PostMediaFocus;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_media_gallery_builder::PostMediaGalleryBuilder;
use crate::modules::post::engine::rise_post_engine_models::{RisePostContentKind, RisePostSearchKind};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostCaptionExpansion, PostListHostedEnvironment, PostRowActionRouting, PostRowBodyKind, PreparedPostRow,
};
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::{
    PostInteractionsStore, PostInteractionsStoreShareReceiptChannel,
};
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::story::stores::story::story_actions::story_actions::StoryActionDoneHandler;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

/// what the tapped media page gives the router, a plain value
/// the page is leased during its own click so the router cant read it back
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostRowMediaPageSource {
    pub index: usize,
    pub decoded_aspect: Option<f32>,
    pub resolved_fills: bool,
    pub screen_frame: Bounds<Pixels>,
}

/// one entry of a native post row context menu, title and action
#[derive(Clone)]
pub struct PostRowMenuItem {
    pub action: HoldContextMenuAction,
    pub perform: Rc<dyn Fn(&mut App)>,
}

pub type PostRowCaptionExpansionToggled = Rc<dyn Fn(&str, &mut App)>;

pub type PostRowForPostID = Rc<dyn Fn(&str, &mut App) -> Option<Arc<PreparedPostRow>>>;
pub type PostRowHostedEnvironmentProvider = Rc<dyn Fn(&App) -> Option<PostListHostedEnvironment>>;

pub struct PostRowActionRouter {
    weak_self: Weak<Self>,
    row_for_post_id: PostRowForPostID,
    hosted_environment: PostRowHostedEnvironmentProvider,
    selection_generator: SelectionFeedbackGenerator,
    light_impact_generator: ImpactFeedbackGenerator,
    on_caption_expansion_toggled: RefCell<Option<PostRowCaptionExpansionToggled>>,
}

impl PostRowActionRouter {
    /// row_for_post_id gives the latest prepared row of a post
    /// hosted_environment holds navigation, media viewer state and open_url
    pub fn new(
        row_for_post_id: PostRowForPostID,
        hosted_environment: PostRowHostedEnvironmentProvider,
    ) -> Rc<Self> {
        Rc::new_cyclic(|weak_self| Self {
            weak_self: weak_self.clone(),
            row_for_post_id,
            hosted_environment,
            selection_generator: SelectionFeedbackGenerator::new(),
            light_impact_generator: ImpactFeedbackGenerator::new(ImpactStyle::Light),
            on_caption_expansion_toggled: RefCell::new(None),
        })
    }

    pub fn set_on_caption_expansion_toggled(&self, handler: Option<PostRowCaptionExpansionToggled>) {
        *self.on_caption_expansion_toggled.borrow_mut() = handler;
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(environment) = (self.hosted_environment)(cx) {
            environment
                .navigation_store
                .update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    /// opens the author page when the avatar is tapped
    fn open_author_page(&self, post: &GetPostFeedResponse, cx: &mut App) {
        self.push(RootRoute::UserPage { user_id: post.author.id.clone(), tag: Some(post.author.tag.clone()) }, cx);
    }

    fn open_stories(&self, user_id: &str, post: &GetPostFeedResponse, cx: &mut App) {
        let router = self.weak_self.clone();
        let post = post.clone();
        let on_unavailable: StoryActionDoneHandler = Rc::new(move |cx: &mut App| {
            if let Some(router) = router.upgrade() {
                router.open_author_page(&post, cx);
            }
        });
        StoryInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_stories_for_user_id(user_id, Some(on_unavailable), cx));
    }

    fn copy_link(post: &GetPostFeedResponse, cx: &mut App) {
        let link = SharePayload::Post(post.clone())
            .share_url()
            .map(|url| url.absolute_string().to_owned())
            .unwrap_or_default();
        UIPasteboard::general().set_string(&link, cx);
        Notifier::copied(cx);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.record_share(post, PostInteractionsStoreShareReceiptChannel::LinkCopy, cx);
        });
    }

    fn share(post: &GetPostFeedResponse, cx: &mut App) {
        let post = post.clone();
        ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
    }

    fn report(post: &GetPostFeedResponse, cx: &mut App) {
        let Ok(post_id) = post.id.parse::<i64>() else {
            return;
        };
        ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(ReportTarget::Post(post_id), None, None, cx));
    }

    /// menu action that looks up the latest post when picked
    fn make_action(
        &self,
        title: String,
        system_image: &str,
        destructive: bool,
        post_id: &str,
        perform: impl Fn(&Self, &GetPostFeedResponse, &mut App) + 'static,
    ) -> PostRowMenuItem {
        let router = self.weak_self.clone();
        let post_id = post_id.to_owned();
        PostRowMenuItem {
            action: HoldContextMenuAction::new(format!("{system_image}|{title}"), title)
                .system_image(system_image)
                .destructive(destructive),
            perform: Rc::new(move |cx: &mut App| {
                let Some(router) = router.upgrade() else {
                    return;
                };
                let Some(post) = router.post(&post_id, cx) else {
                    return;
                };
                perform(&router, &post, cx);
            }),
        }
    }

    fn normalized(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }
}

impl PostRowActionRouting for PostRowActionRouter {
    fn post(&self, post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        let post = (self.row_for_post_id)(post_id, cx).map(|row| row.post.clone());
        if post.is_none() {
            x251_mark(|| format!("x251 router_row none post={post_id}"));
        }
        post
    }

    fn toggle_like(&self, post_id: &str, cx: &mut App) {
        let post = self.post(post_id, cx);
        x251_mark(|| {
            format!("x251 router post={post_id} row={:?}", post.as_ref().map(|p| (p.is_liked, p.likes_count)))
        });
        let Some(post) = post else {
            return;
        };
        self.selection_haptic(cx);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&post, cx));
    }

    fn like_from_double_tap(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        self.light_impact_haptic(cx);
        if post.is_liked {
            return;
        }
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&post, cx));
    }

    fn open_likes(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        if post.likes_count <= 0 {
            return;
        }
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_post_likes(&post, cx));
    }

    fn toggle_favorite(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        self.selection_haptic(cx);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&post, cx));
    }

    fn open_comments(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&post, false, cx));
    }

    fn toggle_repost(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(&post, None, cx));
    }

    fn sparks_tap(&self, post_id: &str, cx: &mut App) -> bool {
        let Some(post) = self.post(post_id, cx) else {
            return false;
        };
        if post.is_temp {
            return false;
        }
        let target = SparksTarget::Post { post_id: post.id.clone() };
        let is_own = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.id == post.author_id);
        let store = SparksInteractionsStore::shared(cx);
        if is_own {
            store.update(cx, |store, cx| store.open_send_sheet(target, true, None, cx));
            return false;
        }
        store.update(cx, |store, cx| store.tap(&target, Some(&post.author_id), cx))
    }

    fn sparks_hold(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        if post.is_temp {
            return;
        }
        let is_own = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.id == post.author_id);
        let target = SparksTarget::Post { post_id: post.id.clone() };
        SparksInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_send_sheet(target, is_own, Some(&post.author_id), cx));
    }

    fn avatar_tapped(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        if !post.author.is_deleted
            && let Some(user_id) = Self::normalized(post.author.id.as_deref())
        {
            let stories = StoryServicesStore::shared(cx);
            let stories = stories.read(cx);
            if stories.has_active_stories(Some(&user_id), cx) || !stories.has_known_stories_state(Some(&user_id), cx) {
                self.selection_haptic(cx);
                self.open_stories(&user_id, &post, cx);
                return;
            }
        }
        self.open_author_page(&post, cx);
    }

    fn header_text_tapped(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        tracing::warn!(target: "riseonly::X2153", post_id, "header column opens author");
        self.open_author_page(&post, cx);
        let profile_user_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let target_post_id = post.rise_view_target_post_id();
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.profile_view(&profile_user_id, Some(target_post_id.as_str()), cx);
        });
    }

    fn open_profile(&self, user_id: &str, tag: &str, cx: &mut App) {
        self.push(RootRoute::UserPage { user_id: Some(user_id.to_owned()), tag: Some(tag.to_owned()) }, cx);
    }

    fn avatar_menu_items(&self, post_id: &str, cx: &mut App) -> Vec<PostRowMenuItem> {
        let Some(post) = self.post(post_id, cx) else {
            return Vec::new();
        };
        let mut elements = vec![self.make_action(
            tr("ctx_open_profile"),
            "person.crop.circle",
            false,
            post_id,
            |router, post, cx| router.open_author_page(post, cx),
        )];
        if !post.author.is_deleted
            && let Some(user_id) = Self::normalized(post.author.id.as_deref())
            && StoryServicesStore::shared(cx).read(cx).has_active_stories(Some(&user_id), cx)
        {
            elements.push(self.make_action(
                tr("ctx_view_story"),
                "play.circle",
                false,
                post_id,
                move |router, post, cx| router.open_stories(&user_id, post, cx),
            ));
        }
        elements
    }

    fn open_audio_page(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        let Some(track) = post.track_model() else {
            return;
        };
        self.push(RootRoute::AudioPage { track_id: track.id, source_selection: post.music_use_selection() }, cx);
    }

    fn open_hashtag(&self, tag: &str, post_id: &str, cx: &mut App) {
        let is_thread = self
            .post(post_id, cx)
            .is_some_and(|post| post.content_kind == RisePostContentKind::Thread.wire());
        let kind = if is_thread { RisePostSearchKind::Threads } else { RisePostSearchKind::Publications };
        self.push(RootRoute::HashtagSearch { tag: tag.to_owned(), kind }, cx);
    }

    fn open_mention(&self, url: &Url, _post_id: &str, window: &mut Window, cx: &mut App) {
        let Some(environment) = (self.hosted_environment)(cx) else {
            return;
        };
        environment.open_url.call_as_function(url, window, cx);
    }

    fn open_link(&self, url: &Url, window: &mut Window, cx: &mut App) {
        let Some(environment) = (self.hosted_environment)(cx) else {
            return;
        };
        let web = if url.scheme().is_none() {
            Url::from_string(&format!("https://{}", url.absolute_string()))
        } else {
            None
        };
        environment.open_url.call_as_function(web.as_ref().unwrap_or(url), window, cx);
    }

    fn toggle_caption_expansion(&self, post_id: &str, cx: &mut App) {
        let Some(row) = (self.row_for_post_id)(post_id, cx) else {
            return;
        };
        let presentation_id = row.seed.presentation_id.clone();
        PostCaptionExpansion::toggle(&presentation_id, cx);
        let handler = self.on_caption_expansion_toggled.borrow().clone();
        if let Some(handler) = handler {
            handler(&presentation_id, cx);
        }
    }

    fn open_tagged_people(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        let viewer_is_author = PostMediaCachePolicy::is_own_post(&post, cx);
        PostCollaboratorsPresenter::shared(cx)
            .update(cx, |presenter, cx| presenter.present_tagged_people(&post.id, viewer_is_author, cx));
    }

    fn open_media_viewer(
        &self,
        post_id: &str,
        slide_index: usize,
        source_view: Option<PostRowMediaPageSource>,
        cx: &mut App,
    ) {
        let Some(row) = (self.row_for_post_id)(post_id, cx) else {
            return;
        };
        let Some(environment) = (self.hosted_environment)(cx) else {
            return;
        };
        let post = row.post.clone();
        let slides = &row.seed.media.slides;
        if slide_index >= slides.len() {
            return;
        }

        let media_box = row.layout.as_ref().and_then(|layout| layout.media_box.as_ref());
        let slide_layouts = media_box.map(|media_box| media_box.slides.as_slice()).unwrap_or_default();
        let source_page = source_view.filter(|page| page.index == slide_index);
        let mut measured_aspects: HashMap<usize, f32> = HashMap::new();
        for (index, layout) in slide_layouts.iter().enumerate().take(slides.len()) {
            if let Some(aspect) = layout.natural_aspect
                && aspect.is_finite()
                && aspect > 0.0
            {
                measured_aspects.insert(index, aspect);
            }
        }
        if let Some(decoded) = source_page.and_then(|page| page.decoded_aspect)
            && decoded.is_finite()
            && decoded > 0.0
        {
            measured_aspects.insert(slide_index, decoded);
        }
        let items = PostMediaGalleryBuilder::items(&post, slides, &measured_aspects, cx);
        let Some(item) = items.get(slide_index) else {
            return;
        };

        let key = MediaViewerFrameKeys::post_image(&post.id, slide_index);
        let frame = MediaCellScreenFrameRegistry::frame(&key)
            .or_else(|| source_view.map(|page| page.screen_frame))
            .unwrap_or_default();
        let fills = if let Some(page) = source_page {
            page.resolved_fills
        } else if let Some(layout) = slide_layouts.get(slide_index) {
            layout.fills
        } else {
            let natural = measured_aspects.get(&slide_index).copied();
            let box_aspect = media_box
                .map(|media_box| media_box.box_aspect)
                .unwrap_or_else(|| PostMediaBox::display_aspect(natural));
            PostMediaBox::fills(natural, box_aspect)
        };
        let skin = MediaViewerHeroSkin::new(item.display_source(), !fills);
        let forward_post = post.clone();
        let report_post = post;
        let handlers = MediaViewerPresentationHandlers {
            on_forward: Some(Rc::new(move |_: &MediaViewerGalleryItem, cx: &mut App| {
                Self::share(&forward_post, cx);
            })),
            on_report: Some(Rc::new(move |_: &MediaViewerGalleryItem, cx: &mut App| {
                Self::report(&report_post, cx);
            })),
            ..Default::default()
        };
        let state = environment.media_viewer_presentation.clone();
        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                state,
                items,
                slide_index,
                frame,
                key,
                skin,
                MediaViewerChrome::POST,
                MediaViewerGalleryScope::Complete,
                handlers,
                cx,
            )
            .await;
        })
        .detach();
    }

    fn toggle_feed_sound(&self, cx: &mut App) {
        self.selection_haptic(cx);
        ShortsServicesStore::shared(cx).update(cx, |store, cx| {
            store.feed_audio_muted = !store.feed_audio_muted;
            cx.notify();
        });
    }

    fn open_riff(&self, post_id: &str, cx: &mut App) {
        let Some(post) = self.post(post_id, cx) else {
            return;
        };
        self.push(RootRoute::ShortViewer { post, origin: None, sound_track_id: None }, cx);
    }

    fn menu_items(&self, post_id: &str, cx: &mut App) -> Vec<PostRowMenuItem> {
        let Some(post) = self.post(post_id, cx) else {
            return Vec::new();
        };
        let is_mine = PostMediaCachePolicy::is_own_post(&post, cx);
        let acts_as_author = is_mine
            || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author()
            || GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.viewer_acts_as_author(&post.id, cx));
        let mut sections: Vec<Vec<PostRowMenuItem>> = Vec::new();

        let mut primary = vec![
            self.make_action(tr("ctx_copy"), "doc.on.doc", false, post_id, |_, post, cx| {
                Self::copy_link(post, cx);
            }),
            self.make_action(tr("ctx_share"), "square.and.arrow.up", false, post_id, |_, post, cx| {
                Self::share(post, cx);
            }),
        ];
        if post.allows_download_entry(cx) {
            primary.push(self.make_action(
                tr("post_download_action"),
                "arrow.down.to.line",
                false,
                post_id,
                |_, post, cx| {
                    let focus_index = PostMediaFocus::page(&post.id, cx);
                    PostDownloadSheetStore::shared(cx)
                        .update(cx, |store, cx| store.start(post.clone(), focus_index, false, 0.0, cx));
                },
            ));
        }
        primary.push(self.make_action(
            if post.is_reposted { tr("repost_delete_action") } else { tr("repost_action") },
            "arrow.2.squarepath",
            false,
            post_id,
            |_, post, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(post, None, cx));
            },
        ));
        sections.push(primary);

        if acts_as_author {
            sections.push(vec![
                self.make_action(tr("post_stats_menu_item"), "chart.bar", false, post_id, |router, post, cx| {
                    router.push(RootRoute::PostStats { post_id: post.rise_view_target_post_id() }, cx);
                }),
                self.make_action(tr("ctx_edit"), "pencil", false, post_id, |router, post, cx| {
                    router.push(RootRoute::EditPost { post: post.clone() }, cx);
                }),
                self.make_action(tr("ctx_delete"), "trash", true, post_id, |_, post, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(post, None, cx));
                }),
            ]);
        }
        if !is_mine {
            sections.push(vec![self.make_action(
                tr("shorts_more_not_interested"),
                "hand.thumbsdown",
                false,
                post_id,
                |_, post, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.not_interested(post, cx));
                },
            )]);
            sections.push(vec![self.make_action(
                tr("ctx_report"),
                "exclamationmark.bubble",
                true,
                post_id,
                |_, post, cx| Self::report(post, cx),
            )]);
        }

        let collaborator_items = PostCollaboratorMenu::items(&post, acts_as_author, cx);
        if !collaborator_items.is_empty() {
            sections.push(
                collaborator_items
                    .into_iter()
                    .map(|item| PostRowMenuItem {
                        action: HoldContextMenuAction::new(format!("collaborator_{}", item.id), item.title)
                            .system_image(item.system_image)
                            .destructive(item.destructive),
                        perform: item.action,
                    })
                    .collect(),
            );
        }
        sections
            .into_iter()
            .enumerate()
            .flat_map(|(index, section)| {
                section.into_iter().enumerate().map(move |(position, item)| {
                    if index == 0 || position != 0 {
                        return item;
                    }
                    PostRowMenuItem { action: item.action.section_break(true), perform: item.perform }
                })
            })
            .collect()
    }

    fn record_impression(&self, post_id: &str, cx: &mut App) {
        let Some(row) = (self.row_for_post_id)(post_id, cx) else {
            return;
        };
        let post = &row.post;
        let target_post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let surface = if row.seed.body == PostRowBodyKind::Reel {
            EngagementSurface::ShortsInline
        } else {
            EngagementSurface::Feed
        };
        let is_short = post.is_short;
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(&target_post_id, &author_id, surface, -1, is_short, 500, cx);
        });
    }

    fn selection_haptic(&self, _cx: &mut App) {
        let _ = self.selection_generator.selection_changed();
    }

    fn light_impact_haptic(&self, _cx: &mut App) {
        let _ = self.light_impact_generator.impact_occurred();
    }
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
