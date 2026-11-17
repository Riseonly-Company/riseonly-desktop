use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui::{App, AppContext, Context};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::guides::shared::guide_sandbox_wallpaper::{GuideSandboxArtColor, GuideSandboxWallpaperArt};
use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_types::StoryAudience;
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, StoryAuthorDTO, StoryDTO};

/// reader's own stories for the highlights and batch action lessons
/// while armed the grid, archive and highlight picker read from here
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuideSandboxStoriesScene {
    pub stories: Vec<StoryDTO>,
    pub archived_ids: HashSet<i64>,
    pub highlights: Vec<HighlightDTO>,
    pub highlight_story_ids: HashMap<i64, Vec<i64>>,
    pub created_highlights: i64,
}

impl GuideSandbox {
    pub const FIRST_STORY_ID: i64 = -910_000_001;
    pub const FIRST_HIGHLIGHT_ID: i64 = -920_000_001;

    const STORY_MEDIA_TYPE_IMAGE: &'static str = "image";
    const STORY_DAY_MS: i64 = 86_400_000;

    pub fn owns_story(&self, id: i64) -> bool {
        self.stories_scene.as_ref().is_some_and(|scene| scene.stories.iter().any(|story| story.id == id))
    }

    pub fn owns_stories(&self, ids: &[i64]) -> bool {
        !ids.is_empty() && ids.iter().all(|id| self.owns_story(*id))
    }

    pub fn owns_highlight(&self, id: i64) -> bool {
        self.stories_scene
            .as_ref()
            .is_some_and(|scene| scene.highlights.iter().any(|highlight| highlight.id == id))
    }

    /// only the reader's own grids and only while the scene is armed
    fn owns_story_owner(&self, user_id: &str, cx: &App) -> bool {
        if !self.is_armed || self.stories_scene.is_none() {
            return false;
        }
        let viewer = AuthServicesStore::try_read(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .unwrap_or_default();
        !user_id.is_empty() && user_id == viewer
    }

    pub fn arm_stories(&mut self, cx: &mut Context<Self>) {
        self.stories_scene = Some(GuideSandboxStoriesScene::default());
        cx.notify();
        let render = cx.background_spawn(async move { GuideSandboxStoryArt::render() });
        let task = cx.spawn(async move |this, cx| {
            let files = render.await;
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed || sandbox.stories_scene.is_none() {
                    return;
                }
                let stories = Self::invent_stories(&files, cx);
                if let Some(scene) = sandbox.stories_scene.as_mut() {
                    scene.stories = stories;
                }
                cx.notify();
            })
            .ok();
        });
        self.scene_tasks.push(task);
    }

    pub fn withdraw_stories(&mut self, cx: &mut Context<Self>) {
        if self.stories_scene.is_none() {
            return;
        }
        StoryGridSelection::shared(cx).update(cx, |selection, cx| selection.end(cx));
        GuideSandboxStoryArt::discard();
    }

    fn invent_stories(files: &[Url], cx: &App) -> Vec<StoryDTO> {
        let author = Self::viewer_story_author(cx);
        let now = (RiseClock::now().time_interval_since_1970() * 1000.0) as i64;
        files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let offset = index as i64;
                let created = now - (offset + 1) * Self::STORY_DAY_MS;
                StoryDTO {
                    allow_screenshots: true,
                    audience: StoryAudience::Everyone.wire().to_owned(),
                    author_id: author.id.clone(),
                    author: Some(author.clone()),
                    caption: None,
                    created_at_ms: created,
                    duration_days: 1,
                    expires_at_ms: created + Self::STORY_DAY_MS,
                    file_id: file.absolute_string().to_owned(),
                    has_viewed: Some(true),
                    id: Self::FIRST_STORY_ID - offset,
                    is_liked: false,
                    like_count: 3 + offset,
                    media_type: Self::STORY_MEDIA_TYPE_IMAGE.to_owned(),
                    post_to_profile: true,
                    view_count: 18 + offset * 7,
                    ..StoryDTO::default()
                }
            })
            .collect()
    }

    fn viewer_story_author(cx: &App) -> StoryAuthorDTO {
        let auth = AuthServicesStore::try_read(cx);
        let profile = auth.and_then(AuthServicesStore::profile);
        StoryAuthorDTO {
            id: auth.and_then(AuthServicesStore::effective_user_id_for_wire).unwrap_or_default(),
            is_online: false,
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            last_seen_ms: None,
            logo: profile.map(|profile| profile.more.logo.clone()).unwrap_or_default(),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            name_color: None,
            premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            ..StoryAuthorDTO::default()
        }
    }

    /// none for anyone else's grid, the store reads its cache as usual
    pub fn profile_stories(&self, owner_id: &str, scope: StoryProfileScope, cx: &App) -> Option<Vec<StoryDTO>> {
        if !self.owns_story_owner(owner_id, cx) {
            return None;
        }
        let scene = self.stories_scene.as_ref()?;
        Some(
            scene
                .stories
                .iter()
                .filter(|story| scene.archived_ids.contains(&story.id) == (scope == StoryProfileScope::Archive))
                .cloned()
                .collect(),
        )
    }

    pub fn user_stories(&self, owner_id: &str, cx: &App) -> Option<Vec<StoryDTO>> {
        if !self.owns_story_owner(owner_id, cx) {
            return None;
        }
        self.stories_scene.as_ref().map(|scene| scene.stories.clone())
    }

    /// lesson highlights first then the reader's own, shelf keeps its real circles
    pub fn highlights(&self, owner_id: &str, real: Vec<HighlightDTO>, cx: &App) -> Vec<HighlightDTO> {
        if !self.owns_story_owner(owner_id, cx) {
            return real;
        }
        let Some(scene) = self.stories_scene.as_ref() else {
            return real;
        };
        scene.highlights.iter().cloned().chain(real).collect()
    }

    pub fn highlight_stories(&self, highlight_id: i64) -> Option<Vec<StoryDTO>> {
        let scene = self.stories_scene.as_ref()?;
        let ids = scene.highlight_story_ids.get(&highlight_id)?;
        Some(
            ids.iter()
                .filter_map(|id| scene.stories.iter().find(|story| story.id == *id).cloned())
                .collect(),
        )
    }

    pub fn archive_stories(&mut self, ids: &[i64], archived: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.stories_scene.as_mut() else {
            return;
        };
        for id in ids {
            if archived {
                scene.archived_ids.insert(*id);
            } else {
                scene.archived_ids.remove(id);
            }
        }
        cx.notify();
        if archived {
            GuideSignalBus::emit(RiseGuideSignal::StoriesArchived);
        }
    }

    pub fn delete_stories(&mut self, ids: &[i64], cx: &mut Context<Self>) {
        let Some(scene) = self.stories_scene.as_mut() else {
            return;
        };
        let gone: HashSet<i64> = ids.iter().copied().collect();
        scene.stories.retain(|story| !gone.contains(&story.id));
        for story_ids in scene.highlight_story_ids.values_mut() {
            story_ids.retain(|id| !gone.contains(id));
        }
        cx.notify();
    }

    pub fn create_highlight(
        &mut self,
        title: String,
        story_ids: Vec<i64>,
        cx: &mut Context<Self>,
    ) -> Option<HighlightDTO> {
        let owner_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let scene = self.stories_scene.as_mut()?;
        let id = Self::FIRST_HIGHLIGHT_ID - scene.created_highlights;
        scene.created_highlights += 1;
        let highlight = HighlightDTO {
            cover_story: scene.stories.iter().find(|story| Some(&story.id) == story_ids.first()).cloned(),
            created_at_ms: (RiseClock::now().time_interval_since_1970() * 1000.0) as i64,
            id,
            owner_id,
            stories_count: story_ids.len() as i64,
            title,
            ..HighlightDTO::default()
        };
        scene.highlights.insert(0, highlight.clone());
        scene.highlight_story_ids.insert(id, story_ids);
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::HighlightCreated);
        Some(highlight)
    }

    pub fn update_highlight(
        &mut self,
        id: i64,
        title: Option<String>,
        cover_story_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(scene) = self.stories_scene.as_mut() else {
            return;
        };
        let Some(index) = scene.highlights.iter().position(|highlight| highlight.id == id) else {
            return;
        };
        let old = scene.highlights[index].clone();
        let cover = cover_story_id
            .and_then(|cover| scene.stories.iter().find(|story| story.id == cover).cloned())
            .or(old.cover_story);
        scene.highlights[index] = HighlightDTO {
            cover_story: cover,
            created_at_ms: old.created_at_ms,
            id: old.id,
            owner_id: old.owner_id,
            stories_count: old.stories_count,
            title: title.unwrap_or(old.title),
            ..HighlightDTO::default()
        };
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::HighlightSaved);
    }

    pub fn add_stories(&mut self, ids: &[i64], highlight_id: i64, cx: &mut Context<Self>) {
        let Some(story_ids) = self
            .stories_scene
            .as_mut()
            .and_then(|scene| scene.highlight_story_ids.get_mut(&highlight_id))
        else {
            return;
        };
        for id in ids {
            if !story_ids.contains(id) {
                story_ids.push(*id);
            }
        }
        self.refresh_highlight_count(highlight_id, cx);
    }

    pub fn remove_story(&mut self, story_id: i64, highlight_id: i64, cx: &mut Context<Self>) {
        if let Some(story_ids) = self
            .stories_scene
            .as_mut()
            .and_then(|scene| scene.highlight_story_ids.get_mut(&highlight_id))
        {
            story_ids.retain(|id| *id != story_id);
        }
        self.refresh_highlight_count(highlight_id, cx);
    }

    pub fn delete_highlight(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(scene) = self.stories_scene.as_mut() else {
            return;
        };
        scene.highlights.retain(|highlight| highlight.id != id);
        scene.highlight_story_ids.remove(&id);
        cx.notify();
    }

    fn refresh_highlight_count(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(scene) = self.stories_scene.as_mut() else {
            return;
        };
        let Some(index) = scene.highlights.iter().position(|highlight| highlight.id == id) else {
            return;
        };
        let old = scene.highlights[index].clone();
        let stories_count = scene.highlight_story_ids.get(&id).map_or(0, |ids| ids.len() as i64);
        scene.highlights[index] = HighlightDTO {
            cover_story: old.cover_story,
            created_at_ms: old.created_at_ms,
            id: old.id,
            owner_id: old.owner_id,
            stories_count,
            title: old.title,
            ..HighlightDTO::default()
        };
        cx.notify();
    }

    /// panda heads the close friends suggestions in that lesson, as someone the reader wrote to recently
    /// none outside it
    pub fn close_friends_suggestion(&self) -> Option<StoryAuthorDTO> {
        if !self.owns_close_friends() {
            return None;
        }
        Some(StoryAuthorDTO {
            id: Self::panda_user_id().to_owned(),
            is_online: false,
            is_premium: false,
            last_seen_ms: None,
            logo: String::new(),
            name: tr("guide_scene_sender"),
            name_color: None,
            premium_badge_emoji: None,
            tag: tr("guide_scene_sender_tag"),
            ..StoryAuthorDTO::default()
        })
    }

    /// the list is saved as one replace of the whole set so the lesson keeps it and sends nothing
    pub fn owns_close_friends(&self) -> bool {
        self.is_armed && self.scenes.contains(&*Self::close_friends_scene_id())
    }
}

/// four portrait pictures for the fake stories
/// evening and sea and forest and night
pub enum GuideSandboxStoryArt {}

impl GuideSandboxStoryArt {
    const WIDTH: u32 = 720;
    const HEIGHT: u32 = 1280;
    const SEED_OFFSET: u64 = 7;

    const PALETTES: [[GuideSandboxArtColor; 3]; 4] = [
        [
            GuideSandboxArtColor::new(1.00, 0.72, 0.42),
            GuideSandboxArtColor::new(0.96, 0.42, 0.47),
            GuideSandboxArtColor::new(0.45, 0.22, 0.52),
        ],
        [
            GuideSandboxArtColor::new(0.55, 0.88, 0.93),
            GuideSandboxArtColor::new(0.20, 0.55, 0.84),
            GuideSandboxArtColor::new(0.09, 0.20, 0.47),
        ],
        [
            GuideSandboxArtColor::new(0.80, 0.92, 0.55),
            GuideSandboxArtColor::new(0.30, 0.66, 0.40),
            GuideSandboxArtColor::new(0.08, 0.30, 0.27),
        ],
        [
            GuideSandboxArtColor::new(0.42, 0.36, 0.80),
            GuideSandboxArtColor::new(0.18, 0.14, 0.45),
            GuideSandboxArtColor::new(0.05, 0.05, 0.16),
        ],
    ];

    fn folder() -> PathBuf {
        temporary_directory().join("guide-sandbox-stories")
    }

    pub fn render() -> Vec<Url> {
        let file_manager = FileManager;
        let folder = Self::folder();
        file_manager.create_directory(&folder, true).ok();
        let mut urls = Vec::new();
        for (index, palette) in Self::PALETTES.iter().enumerate() {
            let path = folder.join(format!("story-{}.jpg", index + 1));
            if !file_manager.file_exists(&path) {
                let image =
                    GuideSandboxWallpaperArt::draw(palette, index as u64 + Self::SEED_OFFSET, Self::WIDTH, Self::HEIGHT);
                if !GuideSandboxWallpaperArt::write_jpeg(&image, &path) {
                    continue;
                }
            }
            if let Some(url) = Url::from_file_path(&path) {
                urls.push(url);
            }
        }
        urls
    }

    pub fn discard() {
        FileManager.remove_item(&Self::folder()).ok();
    }
}
