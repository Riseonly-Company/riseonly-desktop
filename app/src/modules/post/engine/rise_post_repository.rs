use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use rise_engine::{MethodDescriptor, RiseWire, WireError};
use rise_widgets::PageStatus;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::rise_post_engine_models::{PostEntry, PostToggle};
use super::rise_post_rpc::{self as rpc, PostScope, UserFeedKind};

const PAGE_SIZE: u32 = 20;

pub trait PostTransport: Send + Sync {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>>;
}

pub struct LivePostTransport {
    wire: Arc<RiseWire>,
}

impl LivePostTransport {
    pub fn new(wire: Arc<RiseWire>) -> Self {
        Self { wire }
    }
}

impl PostTransport for LivePostTransport {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let wire = Arc::clone(&self.wire);
        Box::pin(async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default();
            wire.call(descriptor, body, now).await
        })
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct FeedPage {
    pub items: Vec<PostEntry>,
    pub status: PageStatus,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub error_key: Option<&'static str>,
    pub did_load: bool,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct PostSnapshot {
    pub revision: u64,
    pub pages: BTreeMap<PostScope, FeedPage>,
}

impl PostSnapshot {
    pub fn page(&self, scope: &PostScope) -> Option<&FeedPage> {
        self.pages.get(scope)
    }
}

pub enum PostCommand {
    Activate {
        scope: PostScope,
    },
    Refresh {
        scope: PostScope,
    },
    LoadMore {
        scope: PostScope,
    },
    Toggle {
        post_id: String,
        toggle: PostToggle,
        enabled: bool,
    },
    SetIdentity {
        user_id: Option<String>,
    },
    Reset,
    Shutdown,
}

pub struct PostRepository {
    commands: mpsc::Sender<PostCommand>,
    snapshot: watch::Receiver<Arc<PostSnapshot>>,
}

impl PostRepository {
    pub fn spawn(runtime: &tokio::runtime::Handle, transport: Arc<dyn PostTransport>) -> Self {
        let (commands, inbox) = mpsc::channel(64);
        let (publisher, snapshot) = watch::channel(Arc::new(PostSnapshot::default()));

        runtime.spawn(
            PostState {
                transport,
                publisher,
                revision: 0,
                pages: BTreeMap::new(),
                cursors: BTreeMap::new(),
                generations: BTreeMap::new(),
                user_id: None,
                request_seed: 0,
            }
            .run(inbox),
        );

        Self { commands, snapshot }
    }

    pub fn snapshot(&self) -> Arc<PostSnapshot> {
        Arc::clone(&self.snapshot.borrow())
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<PostSnapshot>> {
        self.snapshot.clone()
    }

    pub fn dispatch(&self, command: PostCommand) {
        let _ = self.commands.try_send(command);
    }
}

#[derive(Clone, Debug, Default)]
struct Cursor {
    relative_id: Option<String>,
    feed_session_id: Option<String>,
}

impl Cursor {
    fn numeric_relative_id(&self) -> Option<i64> {
        self.relative_id
            .as_deref()
            .and_then(|id| id.trim().parse().ok())
    }
}

struct PostState {
    transport: Arc<dyn PostTransport>,
    publisher: watch::Sender<Arc<PostSnapshot>>,
    revision: u64,
    pages: BTreeMap<PostScope, FeedPage>,
    cursors: BTreeMap<PostScope, Cursor>,
    generations: BTreeMap<PostScope, u64>,
    user_id: Option<String>,
    request_seed: u64,
}

impl PostState {
    async fn run(mut self, mut inbox: mpsc::Receiver<PostCommand>) {
        while let Some(command) = inbox.recv().await {
            match command {
                PostCommand::Activate { scope } => self.activate(scope).await,
                PostCommand::Refresh { scope } => self.load_first(scope).await,
                PostCommand::LoadMore { scope } => self.load_more(scope).await,
                PostCommand::Toggle {
                    post_id,
                    toggle,
                    enabled,
                } => self.toggle(post_id, toggle, enabled).await,
                PostCommand::SetIdentity { user_id } => {
                    if self.user_id != user_id {
                        self.user_id = user_id;
                        self.reset();
                    }
                }
                PostCommand::Reset => self.reset(),
                PostCommand::Shutdown => break,
            }
        }
    }

    fn page_mut(&mut self, scope: &PostScope) -> &mut FeedPage {
        self.pages.entry(scope.clone()).or_default()
    }

    fn generation(&self, scope: &PostScope) -> u64 {
        self.generations.get(scope).copied().unwrap_or_default()
    }

    fn bump_generation(&mut self, scope: &PostScope) -> u64 {
        let generation = self.generation(scope).wrapping_add(1);
        self.generations.insert(scope.clone(), generation);
        generation
    }

    fn reset(&mut self) {
        let scopes: Vec<PostScope> = self.pages.keys().cloned().collect();
        for scope in scopes {
            self.bump_generation(&scope);
        }
        self.pages.clear();
        self.cursors.clear();
        self.publish();
    }

    async fn activate(&mut self, scope: PostScope) {
        if self.page_mut(&scope).did_load {
            return;
        }
        self.load_first(scope).await;
    }

    async fn load_first(&mut self, scope: PostScope) {
        {
            let page = self.page_mut(&scope);
            if page.status == PageStatus::Loading {
                return;
            }
            page.status = PageStatus::Loading;
            page.error_key = None;
        }
        self.publish();

        let generation = self.bump_generation(&scope);
        let outcome = self.fetch(&scope, &Cursor::default(), true).await;

        // Generation moved under us — another refresh, or an account switch.
        if generation != self.generation(&scope) {
            return;
        }

        match outcome {
            Ok(page) => {
                self.cursors.insert(
                    scope.clone(),
                    Cursor {
                        relative_id: page.cursor,
                        feed_session_id: page.feed_session_id,
                    },
                );
                let target = self.page_mut(&scope);
                target.items = page.rows;
                target.has_more = page.has_more;
                target.status = PageStatus::Loaded;
                target.did_load = true;
                target.error_key = None;
            }
            Err(key) => {
                let target = self.page_mut(&scope);
                target.status = PageStatus::Failed;
                target.error_key = Some(key);
            }
        }

        self.publish();
    }

    async fn load_more(&mut self, scope: PostScope) {
        let cursor = {
            let Some(cursor) = self.cursors.get(&scope).cloned() else {
                return;
            };
            let page = self.pages.entry(scope.clone()).or_default();
            if page.is_loading_more || !page.has_more || page.status == PageStatus::Loading {
                return;
            }
            if cursor.relative_id.is_none() {
                return;
            }
            page.is_loading_more = true;
            cursor
        };
        self.publish();

        let generation = self.generation(&scope);
        let outcome = self.fetch(&scope, &cursor, false).await;

        if generation != self.generation(&scope) {
            return;
        }

        match outcome {
            Ok(page) => {
                self.cursors.insert(
                    scope.clone(),
                    Cursor {
                        relative_id: page.cursor,
                        feed_session_id: page.feed_session_id.or(cursor.feed_session_id),
                    },
                );

                let target = self.page_mut(&scope);
                let known: std::collections::BTreeSet<String> =
                    target.items.iter().map(|item| item.id.clone()).collect();

                // Repeated rows across pages are duplicate ElementIds, which gpui drops silently.
                target
                    .items
                    .extend(page.rows.into_iter().filter(|row| !known.contains(&row.id)));
                target.has_more = page.has_more;
                target.error_key = None;
            }
            Err(key) => self.page_mut(&scope).error_key = Some(key),
        }

        self.page_mut(&scope).is_loading_more = false;
        self.publish();
    }

    fn request(&self, scope: &PostScope, cursor: &Cursor, first: bool) -> Value {
        let body = match scope {
            PostScope::Feed(_) => serde_json::to_value(rpc::GetFeedRequest {
                relative_id: cursor.relative_id.clone(),
                limit: PAGE_SIZE,
                new_feed: first,
                up: false,
                feed_session_id: cursor.feed_session_id.clone(),
            }),
            PostScope::User {
                tag,
                kind: UserFeedKind::Reposts,
            } => serde_json::to_value(rpc::GetUserRepostsRequest {
                tag: tag.clone(),
                user_id: self.user_id.clone(),
                limit: PAGE_SIZE,
                relative_id: cursor.numeric_relative_id(),
                up: false,
            }),
            PostScope::User { tag, .. } => serde_json::to_value(rpc::GetUserPostsRequest {
                tag: tag.clone(),
                user_id: self.user_id.clone(),
                limit: PAGE_SIZE,
                relative_id: cursor.numeric_relative_id(),
                up: false,
                new_feed: first,
                feed_session_id: cursor.feed_session_id.clone(),
            }),
        };

        body.unwrap_or(Value::Null)
    }

    async fn fetch(
        &self,
        scope: &PostScope,
        cursor: &Cursor,
        first: bool,
    ) -> Result<Page, &'static str> {
        let body = self.request(scope, cursor, first);
        let payload = self
            .transport
            .call(scope.descriptor(), body)
            .await
            .map_err(|_| "feed_load_error")?;

        // A page the DTOs cannot read is a failure with a retry, never a page of
        // no posts: `unwrap_or_default` here spelled every wire drift "empty feed".
        let response: rpc::FeedResponse =
            serde_json::from_value(payload).map_err(|_| "feed_load_error")?;
        if rise_engine::remote_message(response.error.as_deref()).is_some() {
            return Err("feed_load_error");
        }

        Ok(Page {
            rows: response
                .rows()
                .iter()
                .filter_map(PostEntry::from_dto)
                .collect(),
            cursor: response.relative_id.clone().map(|id| id.into_string()),
            has_more: response.more(),
            feed_session_id: response.feed_session_id.clone(),
        })
    }

    async fn toggle(&mut self, post_id: String, toggle: PostToggle, enabled: bool) {
        let Some(user_id) = self.user_id.clone().filter(|id| !id.is_empty()) else {
            return;
        };

        let previous = self.snapshot_of(&post_id);
        if previous.is_empty() {
            return;
        }

        self.for_each_matching(&post_id, |entry| entry.apply_toggle(toggle, enabled));
        self.publish();

        self.request_seed = self.request_seed.wrapping_add(1);
        let client_request_id = format!("{post_id}:{:?}:{}", toggle, self.request_seed);

        let Ok(numeric) = post_id.parse::<i64>() else {
            self.restore(&post_id, previous);
            self.publish();
            return;
        };

        let (descriptor, body) = match toggle {
            PostToggle::Like => (
                &rpc::SET_POST_LIKE,
                serde_json::to_value(rpc::SetPostLikeRequest {
                    user_id,
                    post_id: numeric,
                    liked: enabled,
                    client_request_id,
                }),
            ),
            PostToggle::Favorite => (
                &rpc::SET_POST_FAVORITE,
                serde_json::to_value(rpc::SetPostFavoriteRequest {
                    user_id,
                    post_id: numeric,
                    favorited: enabled,
                    client_request_id,
                }),
            ),
            PostToggle::Repost if enabled => (
                &rpc::CREATE_REPOST,
                serde_json::to_value(rpc::CreateRepostRequest {
                    user_id,
                    post_id: numeric,
                    content: None,
                    client_request_id,
                }),
            ),
            PostToggle::Repost => (
                &rpc::DELETE_REPOST,
                serde_json::to_value(rpc::DeleteRepostRequest {
                    user_id,
                    post_id: numeric,
                    client_request_id,
                }),
            ),
        };

        let result = self
            .transport
            .call(descriptor, body.unwrap_or(Value::Null))
            .await;

        match result {
            Ok(payload) => {
                let response: rpc::InteractionResponse =
                    serde_json::from_value(payload).unwrap_or_default();

                if rise_engine::remote_message(response.error.as_deref()).is_some() {
                    self.restore(&post_id, previous);
                } else {
                    self.for_each_matching(&post_id, |entry| {
                        entry.reconcile(
                            response.is_liked,
                            response.is_favorited,
                            response.is_reposted,
                            response.likes_count,
                            response.favorites_count,
                            response.repost_count,
                        )
                    });
                }
            }
            Err(_) => self.restore(&post_id, previous),
        }

        self.publish();
    }

    fn snapshot_of(&self, post_id: &str) -> Vec<(PostScope, usize, PostEntry)> {
        let mut found = Vec::new();
        for (scope, page) in &self.pages {
            for (index, entry) in page.items.iter().enumerate() {
                if entry.id == post_id {
                    found.push((scope.clone(), index, entry.clone()));
                }
            }
        }
        found
    }

    fn for_each_matching(&mut self, post_id: &str, mut apply: impl FnMut(&mut PostEntry)) {
        for page in self.pages.values_mut() {
            for entry in page.items.iter_mut() {
                if entry.id == post_id {
                    apply(entry);
                }
            }
        }
    }

    fn restore(&mut self, post_id: &str, previous: Vec<(PostScope, usize, PostEntry)>) {
        for (scope, index, entry) in previous {
            if let Some(page) = self.pages.get_mut(&scope)
                && let Some(slot) = page.items.get_mut(index)
                // A refresh may have moved the row; position alone would restore onto another post.
                && slot.id == post_id
            {
                *slot = entry;
            }
        }
    }

    fn publish(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.publisher.send_replace(Arc::new(PostSnapshot {
            revision: self.revision,
            pages: self.pages.clone(),
        }));
    }
}

struct Page {
    rows: Vec<PostEntry>,
    cursor: Option<String>,
    has_more: bool,
    feed_session_id: Option<String>,
}

#[cfg(test)]
#[path = "rise_post_repository_tests.rs"]
mod tests;
