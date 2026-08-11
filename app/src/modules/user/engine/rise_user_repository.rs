use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use rise_engine::{Debouncer, MethodDescriptor, RiseWire, WireError};
use rise_widgets::PageStatus;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::core::rise_user_rpc as rpc;
use super::rise_user_engine_models::{ProfileKey, ProfileModel, Relationship};
use super::rise_user_presentation::{ProfileEntry, UserSnapshot};

/// The reference waits this long before committing a follow, so a user tapping
/// the button twice sends nothing at all rather than a pair of mutations.
pub const FOLLOW_DEBOUNCE: Duration = Duration::from_millis(600);

pub trait UserTransport: Send + Sync {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>>;
}

pub struct LiveUserTransport {
    wire: Arc<RiseWire>,
}

impl LiveUserTransport {
    pub fn new(wire: Arc<RiseWire>) -> Self {
        Self { wire }
    }
}

impl UserTransport for LiveUserTransport {
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

pub enum UserCommand {
    Activate { key: ProfileKey },
    Refresh { key: ProfileKey },
    ToggleFollow { key: ProfileKey },
    CommitFollow { key: ProfileKey, generation: u64 },
    SetViewer { user_id: Option<String> },
    Reset,
    Shutdown,
}

pub struct UserRepository {
    commands: mpsc::Sender<UserCommand>,
    snapshot: watch::Receiver<Arc<UserSnapshot>>,
}

impl UserRepository {
    pub fn spawn(
        runtime: &tokio::runtime::Handle,
        transport: Arc<dyn UserTransport>,
        follow_debounce: Duration,
    ) -> Self {
        let (commands, inbox) = mpsc::channel(64);
        let (publisher, snapshot) = watch::channel(Arc::new(UserSnapshot::default()));

        runtime.spawn(
            UserState {
                transport,
                publisher,
                revision: 0,
                viewer_id: None,
                profiles: BTreeMap::new(),
                generations: BTreeMap::new(),
                committed: BTreeMap::new(),
                follow_debouncers: BTreeMap::new(),
                follow_debounce,
                commands: commands.clone(),
            }
            .run(inbox),
        );

        Self { commands, snapshot }
    }

    pub fn snapshot(&self) -> Arc<UserSnapshot> {
        Arc::clone(&self.snapshot.borrow())
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<UserSnapshot>> {
        self.snapshot.clone()
    }

    pub fn dispatch(&self, command: UserCommand) {
        let _ = self.commands.try_send(command);
    }
}

struct UserState {
    transport: Arc<dyn UserTransport>,
    publisher: watch::Sender<Arc<UserSnapshot>>,
    revision: u64,
    viewer_id: Option<String>,
    profiles: BTreeMap<ProfileKey, ProfileEntry>,
    generations: BTreeMap<ProfileKey, u64>,
    committed: BTreeMap<ProfileKey, Relationship>,
    // One per profile: a single debouncer would let a follow on one profile
    // cancel the commit of a follow the user just made on another.
    follow_debouncers: BTreeMap<ProfileKey, Debouncer>,
    follow_debounce: Duration,
    commands: mpsc::Sender<UserCommand>,
}

impl UserState {
    async fn run(mut self, mut inbox: mpsc::Receiver<UserCommand>) {
        while let Some(command) = inbox.recv().await {
            match command {
                UserCommand::Activate { key } => self.activate(key).await,
                UserCommand::Refresh { key } => self.load(key).await,
                UserCommand::ToggleFollow { key } => self.toggle_follow(key),
                UserCommand::CommitFollow { key, generation } => {
                    self.commit_follow(key, generation).await
                }
                UserCommand::SetViewer { user_id } => {
                    if self.viewer_id != user_id {
                        self.viewer_id = user_id;
                        self.reset();
                    }
                }
                UserCommand::Reset => self.reset(),
                UserCommand::Shutdown => break,
            }
        }
    }

    fn entry_mut(&mut self, key: &ProfileKey) -> &mut ProfileEntry {
        self.profiles.entry(key.clone()).or_default()
    }

    fn generation(&self, key: &ProfileKey) -> u64 {
        self.generations.get(key).copied().unwrap_or_default()
    }

    fn bump_generation(&mut self, key: &ProfileKey) -> u64 {
        let generation = self.generation(key).wrapping_add(1);
        self.generations.insert(key.clone(), generation);
        generation
    }

    fn reset(&mut self) {
        let keys: Vec<ProfileKey> = self.profiles.keys().cloned().collect();
        for key in keys {
            self.bump_generation(&key);
        }
        self.profiles.clear();
        self.committed.clear();
        self.follow_debouncers.clear();
        self.publish();
    }

    async fn activate(&mut self, key: ProfileKey) {
        if self.entry_mut(&key).did_load {
            return;
        }
        self.load(key).await;
    }

    async fn load(&mut self, key: ProfileKey) {
        let Some(request) = self.request_for(&key) else {
            let entry = self.entry_mut(&key);
            entry.status = PageStatus::Failed;
            entry.error_key = Some("profile_load_error");
            self.publish();
            return;
        };

        {
            let entry = self.entry_mut(&key);
            if entry.status == PageStatus::Loading {
                return;
            }
            entry.status = PageStatus::Loading;
            entry.error_key = None;
        }
        self.publish();

        let generation = self.bump_generation(&key);
        let (descriptor, body) = request;
        let outcome = self.fetch(descriptor, body).await;

        // Generation moved under us — another refresh, or the account changed.
        if generation != self.generation(&key) {
            return;
        }

        match outcome {
            Ok(profile) => {
                if key.is_own() {
                    self.viewer_id = Some(profile.id.clone());
                }
                let follow_in_flight = self.entry_mut(&key).follow_in_flight;
                let entry = self.entry_mut(&key);
                // An in-flight follow is newer than any read that raced it.
                let relationship = follow_in_flight.then(|| entry.profile.relationship.clone());
                entry.profile = profile;
                if let Some(relationship) = relationship {
                    entry.profile.relationship = relationship;
                }
                entry.status = PageStatus::Loaded;
                entry.did_load = true;
                entry.error_key = None;
                if !follow_in_flight {
                    let committed = entry.profile.relationship.clone();
                    self.committed.insert(key.clone(), committed);
                }
            }
            Err(error_key) => {
                let entry = self.entry_mut(&key);
                entry.status = PageStatus::Failed;
                entry.error_key = Some(error_key);
            }
        }

        self.publish();
    }

    fn request_for(&self, key: &ProfileKey) -> Option<(&'static MethodDescriptor, Value)> {
        let viewer = self.viewer_id.clone().filter(|id| !id.is_empty());

        match key {
            ProfileKey::Own => Some((
                &rpc::GET_MY_PROFILE,
                body(rpc::GetMyProfileRequest { user_id: viewer? }),
            )),
            ProfileKey::Id(id) => Some((
                &rpc::GET_PROFILE_BY_ID,
                body(rpc::GetProfileByIdRequest {
                    profile_user_id: id.clone(),
                    user_id: viewer?,
                }),
            )),
            ProfileKey::Tag(tag) => Some((
                &rpc::GET_PROFILE_BY_TAG,
                body(rpc::GetProfileByTagRequest {
                    tag: tag.clone(),
                    user_id: viewer,
                }),
            )),
        }
    }

    async fn fetch(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> Result<ProfileModel, &'static str> {
        let payload = self
            .transport
            .call(descriptor, body)
            .await
            .map_err(|_| "profile_load_error")?;

        let response: rpc::ProfileResponse = serde_json::from_value(payload).unwrap_or_default();
        if rise_engine::remote_message(response.error.as_deref()).is_some() {
            return Err("profile_load_error");
        }

        response
            .profile
            .map(ProfileModel::from_dto)
            .ok_or("profile_load_error")
    }

    fn toggle_follow(&mut self, key: ProfileKey) {
        let Some(viewer) = self.viewer_id.clone().filter(|id| !id.is_empty()) else {
            return;
        };

        let Some(entry) = self.profiles.get_mut(&key) else {
            return;
        };
        if entry.profile.id.is_empty() || entry.profile.id == viewer {
            return;
        }

        if !entry.follow_in_flight {
            let committed = entry.profile.relationship.clone();
            self.committed.insert(key.clone(), committed);
        }

        let entry = self.profiles.get_mut(&key).expect("checked above");
        let following = !entry.profile.relationship.is_subbed;
        entry.profile.apply_follow(following);
        entry.follow_in_flight = true;

        let generation = self.bump_generation(&key);
        self.publish();

        let message = UserCommand::CommitFollow {
            key: key.clone(),
            generation,
        };
        self.follow_debouncers
            .entry(key)
            .or_insert_with(|| Debouncer::new(self.follow_debounce))
            .send(&self.commands, message);
    }

    async fn commit_follow(&mut self, key: ProfileKey, generation: u64) {
        self.follow_debouncers.remove(&key);

        if generation != self.generation(&key) {
            return;
        }

        let Some(viewer) = self.viewer_id.clone().filter(|id| !id.is_empty()) else {
            return;
        };
        let Some(entry) = self.profiles.get(&key) else {
            return;
        };

        let target_id = entry.profile.id.clone();
        let desired = entry.profile.relationship.is_subbed;
        let baseline = self.committed.get(&key).cloned().unwrap_or_default();

        // The user came back to where they started while the debounce was open.
        if baseline.is_subbed == desired {
            let entry = self.profiles.get_mut(&key).expect("checked above");
            entry.follow_in_flight = false;
            self.publish();
            return;
        }

        let (descriptor, request) = if desired {
            (
                &rpc::SEND_FRIEND_REQUEST,
                body(rpc::SendFriendRequestRequest {
                    user_id: viewer,
                    receiver_id: target_id,
                }),
            )
        } else {
            (
                &rpc::UNFOLLOW_USER,
                body(rpc::UnfollowUserRequest {
                    user_id: viewer,
                    target_id,
                }),
            )
        };

        let result = self.transport.call(descriptor, request).await;

        if generation != self.generation(&key) {
            return;
        }

        match result {
            Ok(payload) => self.reconcile(&key, desired, payload, baseline),
            Err(_) => self.rollback(&key, baseline),
        }

        if let Some(entry) = self.profiles.get_mut(&key) {
            entry.follow_in_flight = false;
        }
        self.publish();
    }

    fn reconcile(
        &mut self,
        key: &ProfileKey,
        followed: bool,
        payload: Value,
        baseline: Relationship,
    ) {
        if followed {
            let response: rpc::FriendRequestResponse =
                serde_json::from_value(payload).unwrap_or_default();

            if rise_engine::remote_message(response.error.as_deref()).is_some() {
                self.rollback(key, baseline);
                return;
            }

            if let Some(entry) = self.profiles.get_mut(key) {
                entry.profile.relationship.friend_request_id = response.id.filter(|id| *id > 0);
                let committed = entry.profile.relationship.clone();
                self.committed.insert(key.clone(), committed);
            }
            return;
        }

        let response: rpc::RelationshipResponse =
            serde_json::from_value(payload).unwrap_or_default();

        if rise_engine::remote_message(response.error.as_deref()).is_some() {
            self.rollback(key, baseline);
            return;
        }

        if let Some(entry) = self.profiles.get_mut(key) {
            entry.profile.reconcile(Relationship {
                is_friend: response.is_friend.unwrap_or(false),
                is_subbed: response.is_subbed.unwrap_or(false),
                is_subscriber: response
                    .is_subscriber
                    .unwrap_or(entry.profile.relationship.is_subscriber),
                friend_request_id: response.friend_request_id.filter(|id| *id > 0),
            });
            let committed = entry.profile.relationship.clone();
            self.committed.insert(key.clone(), committed);
        }
    }

    fn rollback(&mut self, key: &ProfileKey, baseline: Relationship) {
        if let Some(entry) = self.profiles.get_mut(key) {
            entry.profile.reconcile(baseline.clone());
        }
        self.committed.insert(key.clone(), baseline);
    }

    fn publish(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.publisher.send_replace(Arc::new(UserSnapshot {
            revision: self.revision,
            viewer_id: self.viewer_id.clone(),
            profiles: self.profiles.clone(),
        }));
    }
}

fn body<T: serde::Serialize>(request: T) -> Value {
    serde_json::to_value(request).unwrap_or(Value::Null)
}

#[cfg(test)]
#[path = "rise_user_repository_tests.rs"]
mod tests;
