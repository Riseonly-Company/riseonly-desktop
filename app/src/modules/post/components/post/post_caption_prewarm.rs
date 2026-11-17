use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{App, AsyncApp, Global, Task};
use rise_ui::rich_text::rich_text_markup::{RichBlock, RichTextMarkup};

use crate::core::ui::formatted_text_ui::{
    FormattedText, FormattedTextOptions, FormattedTextPreparationCachePolicy, FormattedTextStyle,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// formats post captions off main before their rows realize so the first rich text body reads
/// from cache
/// style is the one post_copy passes for a non preview row
pub enum PostCaptionPrewarm {}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PostCaptionPrewarmMark {
    post_id: String,
    content_hash: u64,
    style_hash: u64,
}

struct PostCaptionPrewarmJob {
    mark: PostCaptionPrewarmMark,
    text: String,
    style: FormattedTextStyle,
}

#[derive(Default)]
struct PostCaptionPrewarmState {
    marks: HashSet<PostCaptionPrewarmMark>,
    mark_order: Vec<PostCaptionPrewarmMark>,
    pending: Vec<PostCaptionPrewarmJob>,
    worker: Option<Task<()>>,
}

impl Global for PostCaptionPrewarmState {}

fn hash_value<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

impl PostCaptionPrewarm {
    const CAPTION_FONT_SIZE: f32 = 15.0;
    const POSTS_PER_CALL: usize = 16;
    const PENDING_LIMIT: usize = 48;
    const MARK_LIMIT: usize = 512;

    pub fn warm(posts: &[GetPostFeedResponse], cx: &mut App) {
        if !posts.iter().any(|post| !post.content.is_empty()) {
            return;
        }
        let style = FormattedTextStyle::app_standard(rise_ui::theme(cx), Self::CAPTION_FONT_SIZE);
        let style_hash = hash_value(&style);

        let state = cx.default_global::<PostCaptionPrewarmState>();
        let mut accepted = 0;
        for post in posts.iter().filter(|post| !post.content.is_empty()) {
            if accepted >= Self::POSTS_PER_CALL {
                break;
            }
            let mark = PostCaptionPrewarmMark {
                post_id: post.id.clone(),
                content_hash: hash_value(post.content.as_str()),
                style_hash,
            };
            if !state.marks.insert(mark.clone()) {
                continue;
            }
            state.mark_order.push(mark.clone());
            state.pending.push(PostCaptionPrewarmJob { mark, text: post.content.clone(), style: style.clone() });
            accepted += 1;
        }
        if accepted == 0 {
            return;
        }

        if state.pending.len() > Self::PENDING_LIMIT {
            let excess = state.pending.len() - Self::PENDING_LIMIT;
            for job in state.pending.drain(..excess) {
                state.marks.remove(&job.mark);
            }
        }
        if state.mark_order.len() > Self::MARK_LIMIT {
            let excess = state.mark_order.len() - Self::MARK_LIMIT;
            for mark in state.mark_order.drain(..excess) {
                state.marks.remove(&mark);
            }
        }
        Self::start_worker_if_needed(cx);
    }

    fn start_worker_if_needed(cx: &mut App) {
        let state = cx.default_global::<PostCaptionPrewarmState>();
        if state.worker.is_some() || state.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut state.pending);
        let worker = cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor()
                .spawn(async move {
                    for job in &batch {
                        Self::prepare(job);
                    }
                })
                .await;
            cx.update(Self::worker_finished);
        });
        cx.default_global::<PostCaptionPrewarmState>().worker = Some(worker);
    }

    fn worker_finished(cx: &mut App) {
        if let Some(worker) = cx.default_global::<PostCaptionPrewarmState>().worker.take() {
            worker.detach();
        }
        Self::start_worker_if_needed(cx);
    }

    /// same keys as the rich text view
    /// post caption options, no role mentions
    fn prepare(job: &PostCaptionPrewarmJob) {
        for block in RichTextMarkup::blocks(&job.text).iter() {
            let RichBlock::Paragraph(paragraph) = block else {
                continue;
            };
            if !FormattedTextPreparationCachePolicy::can_cache(paragraph) {
                continue;
            }
            let _ = FormattedText::format(paragraph, &job.style, &FormattedTextOptions::post_caption(), &[]);
        }
    }
}
