use std::collections::{HashMap, HashSet};

use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentReaction, CommentReactionCommand, CommentReactionEndpoint,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseCommentReactionState {
    pub reaction: CommentReaction,
    pub likes_count: i64,
    pub dislikes_count: i64,
}

/// one comment reaction between the first tap and the last ack
/// base is the last server confirmed state and desired is the last tap
/// request is cut from the difference on debounce and taps wait while one is in flight
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseCommentReactionSession {
    pub base: RiseCommentReactionState,
    pub desired: CommentReaction,
    pub generation: i64,
    pub inflight_generation: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseCommentReactionSessionStep {
    Settle(RiseCommentReactionState),
    Send(CommentReactionCommand),
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseCommentReactionSessionResolution {
    Finished(RiseCommentReactionState),
    Continues,
    Ignored,
}

impl RiseCommentReactionSession {
    pub fn derived(&self) -> RiseCommentReactionState {
        let mut likes = self.base.likes_count;
        let mut dislikes = self.base.dislikes_count;
        if self.base.reaction == CommentReaction::Like {
            likes = (likes - 1).max(0);
        }
        if self.base.reaction == CommentReaction::Dislike {
            dislikes = (dislikes - 1).max(0);
        }
        if self.desired == CommentReaction::Like {
            likes += 1;
        }
        if self.desired == CommentReaction::Dislike {
            dislikes += 1;
        }
        RiseCommentReactionState { reaction: self.desired, likes_count: likes, dislikes_count: dislikes }
    }

    pub fn next_step(&mut self, comment_id: &str) -> RiseCommentReactionSessionStep {
        if self.inflight_generation.is_some() {
            return RiseCommentReactionSessionStep::Wait;
        }
        if self.desired == self.base.reaction {
            return RiseCommentReactionSessionStep::Settle(self.base);
        }

        let (endpoint, enabled) = match self.desired {
            CommentReaction::Like => (CommentReactionEndpoint::Like, true),
            CommentReaction::Dislike => (CommentReactionEndpoint::Dislike, true),
            CommentReaction::None => {
                let endpoint = if self.base.reaction == CommentReaction::Like {
                    CommentReactionEndpoint::Like
                } else {
                    CommentReactionEndpoint::Dislike
                };
                (endpoint, false)
            }
        };
        self.inflight_generation = Some(self.generation);
        RiseCommentReactionSessionStep::Send(CommentReactionCommand {
            comment_id: comment_id.to_owned(),
            endpoint,
            enabled,
            generation: self.generation,
        })
    }

    pub fn acknowledge(
        &mut self,
        acknowledged: i64,
        state: RiseCommentReactionState,
    ) -> RiseCommentReactionSessionResolution {
        if self.inflight_generation != Some(acknowledged) {
            return RiseCommentReactionSessionResolution::Ignored;
        }
        self.base = state;
        self.inflight_generation = None;
        if self.generation == acknowledged {
            RiseCommentReactionSessionResolution::Finished(state)
        } else {
            RiseCommentReactionSessionResolution::Continues
        }
    }

    pub fn fail(&mut self, failed: i64) -> RiseCommentReactionSessionResolution {
        if self.inflight_generation != Some(failed) {
            return RiseCommentReactionSessionResolution::Ignored;
        }
        self.inflight_generation = None;
        if self.generation == failed {
            RiseCommentReactionSessionResolution::Finished(self.base)
        } else {
            RiseCommentReactionSessionResolution::Continues
        }
    }
}

/// where an optimistic reply sits in its thread
/// reply to a reply lands under its addressee, reply to the root goes to the tail in send order
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseCommentReplyPlacement {
    target_by_reply: HashMap<String, String>,
    order: Vec<String>,
    pub capacity: usize,
}

impl RiseCommentReplyPlacement {
    pub fn new(capacity: usize) -> Self {
        Self { target_by_reply: HashMap::new(), order: Vec::new(), capacity: capacity.max(1) }
    }

    pub fn target_by_reply(&self) -> &HashMap<String, String> {
        &self.target_by_reply
    }

    pub fn order(&self) -> &[String] {
        &self.order
    }

    pub fn remember(&mut self, reply_id: &str, target_id: &str) {
        self.target_by_reply.insert(reply_id.to_owned(), target_id.to_owned());
        self.order.retain(|id| id != reply_id);
        self.order.push(reply_id.to_owned());
        while self.order.len() > self.capacity {
            let evicted = self.order.remove(0);
            self.target_by_reply.remove(&evicted);
        }
    }

    /// ack changes the row identity not its place
    pub fn rename(&mut self, old_id: &str, new_id: &str) {
        if old_id == new_id {
            return;
        }
        let Some(target) = self.target_by_reply.remove(old_id) else {
            return;
        };
        self.target_by_reply.insert(new_id.to_owned(), target);
        self.order.retain(|id| id != new_id);
        if let Some(index) = self.order.iter().position(|id| id == old_id) {
            self.order[index] = new_id.to_owned();
        } else {
            self.order.push(new_id.to_owned());
        }
    }

    pub fn forget(&mut self, reply_id: &str) {
        self.target_by_reply.remove(reply_id);
        self.order.retain(|id| id != reply_id);
    }

    pub fn arranged(&self, ids: &[String], root_id: &str) -> Vec<String> {
        if self.target_by_reply.is_empty() {
            return ids.to_vec();
        }
        let present: HashSet<&str> = ids.iter().map(String::as_str).collect();

        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut tail: Vec<&str> = Vec::new();
        for reply_id in &self.order {
            if !present.contains(reply_id.as_str()) {
                continue;
            }
            let Some(target) = self.target_by_reply.get(reply_id) else {
                continue;
            };
            if target != root_id && !present.contains(target.as_str()) {
                continue;
            }
            if target == root_id {
                tail.push(reply_id);
            } else {
                children.entry(target).or_default().push(reply_id);
            }
        }

        let mut placed: HashSet<&str> = tail.iter().copied().collect();
        for replies in children.values() {
            placed.extend(replies.iter().copied());
        }
        if placed.is_empty() {
            return ids.to_vec();
        }

        let mut result: Vec<String> = Vec::with_capacity(ids.len());
        let mut visited: HashSet<&str> = HashSet::new();
        for id in ids {
            if !placed.contains(id.as_str()) {
                append_with_children(id, &children, &present, &mut visited, &mut result);
            }
        }
        for id in tail.iter().copied() {
            append_with_children(id, &children, &present, &mut visited, &mut result);
        }
        for id in ids {
            if !visited.contains(id.as_str()) {
                append_with_children(id, &children, &present, &mut visited, &mut result);
            }
        }
        result
    }
}

fn append_with_children<'a>(
    id: &'a str,
    children: &HashMap<&'a str, Vec<&'a str>>,
    present: &HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
    result: &mut Vec<String>,
) {
    if !visited.insert(id) {
        return;
    }
    result.push(id.to_owned());
    let Some(replies) = children.get(id) else {
        return;
    };
    for child in replies.iter().copied() {
        if present.contains(child) {
            append_with_children(child, children, present, visited, result);
        }
    }
}
