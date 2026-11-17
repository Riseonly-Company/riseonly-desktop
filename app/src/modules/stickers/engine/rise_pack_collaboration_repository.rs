use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Weak};

use futures::FutureExt as _;
use gpui::{App, AsyncApp};
use rise_canon::{rise_proto, rise_request, rise_response};
use rise_core::RiseIsolated;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::rise_pack_collaboration_presentation::{RisePackCollaborationPresentation, RisePackCollaborationSnapshot};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};

pub type RisePackInviteLink = rise_proto::stickers::ProtoPackInviteLink;
pub type RisePackInviteStats = rise_proto::stickers::ProtoPackInviteLinkStats;
pub type RisePackCoauthor = rise_proto::stickers::ProtoPackCoauthor;
pub type RisePackRights = rise_proto::stickers::ProtoPackRights;
pub type RisePackJoinRequest = rise_proto::stickers::ProtoPackJoinRequest;
pub type RisePackInvitePreview = rise_proto::stickers::ProtoGetPackInvitePreviewResponse;
pub type RisePackJoinResult = rise_proto::stickers::ProtoJoinPackByLinkResponse;

/// refusal the stickers service reported in the body,
/// code is the canon error code the screens map to text
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePackCollaborationFailure {
    pub code: String,
}

impl fmt::Display for RisePackCollaborationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RisePackCollaborationFailure(code: {})", self.code)
    }
}

impl std::error::Error for RisePackCollaborationFailure {}

/// one page of a cursor list, next request passes next_relative_id and none means the end
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePackPage<Item> {
    pub items: Vec<Item>,
    pub next_relative_id: Option<String>,
    pub total_count: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RisePackInviteLinksPage {
    pub links: RisePackPage<RisePackInviteLink>,
    pub stats: Option<RisePackInviteStats>,
}

#[derive(Default)]
struct RisePackCollaborationState {
    links: HashMap<String, RisePackInviteLinksPage>,
    coauthors: HashMap<String, RisePackPage<RisePackCoauthor>>,
    requests: HashMap<String, RisePackPage<RisePackJoinRequest>>,
    joiners: HashMap<String, RisePackPage<RisePackCoauthor>>,
    revision: u64,
    publish_pending: bool,
    is_shutdown: bool,
}

impl RisePackCollaborationState {
    fn forget(&mut self, pack_id: &str) {
        if let Some(held) = self.links.get(pack_id) {
            let link_ids: Vec<String> = held.links.items.iter().map(|link| link.id.clone()).collect();
            for link_id in link_ids {
                self.joiners.remove(&link_id);
            }
        }
        self.links.remove(pack_id);
        self.coauthors.remove(pack_id);
        self.requests.remove(pack_id);
    }

    fn upsert_link(&mut self, link: &RisePackInviteLink, pack_id: &str, front: bool) -> bool {
        let Some(page) = self.links.get_mut(pack_id) else { return false };
        let items = &mut page.links.items;
        if let Some(index) = items.iter().position(|item| item.id == link.id) {
            items[index] = link.clone();
        } else if front {
            items.insert(0, link.clone());
        } else {
            items.push(link.clone());
        }
        if link.is_primary {
            for item in items.iter_mut().filter(|item| item.id != link.id) {
                item.is_primary = false;
            }
        }
        true
    }
}

/// pack coauthors over the stickers service, invite links and join requests
/// lists live in memory per pack and writes update them in place
pub struct RisePackCollaborationRepository {
    pub account_id: String,
    this: Weak<RisePackCollaborationRepository>,
    rpc: RiseConversationRPC,
    state: RiseIsolated<RisePackCollaborationState>,
}

impl RisePackCollaborationRepository {
    pub const PAGE_SIZE: i64 = 50;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            account_id,
            this: this.clone(),
            rpc: RiseConversationRPC::new(engine),
            state: RiseIsolated::new(RisePackCollaborationState::default()),
        })
    }

    async fn on_main<R: Send + 'static>(work: impl FnOnce(&mut App) -> R + Send + 'static) -> Option<R> {
        let hop = RiseEngineMainHop::installed()?;
        hop.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
    }

    pub async fn activate(&self) {
        let account_id = self.account_id.clone();
        Self::on_main(move |cx| {
            RisePackCollaborationPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.activate(&account_id, cx));
        })
        .await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.links.clear();
            s.coauthors.clear();
            s.requests.clear();
            s.joiners.clear();
        });
        let account_id = self.account_id.clone();
        Self::on_main(move |cx| {
            RisePackCollaborationPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.deactivate(&account_id, cx));
        })
        .await;
    }

    pub async fn invite_links(&self, pack_id: &str, more: bool) -> anyhow::Result<RisePackInviteLinksPage> {
        let held = self.state.with(|s| s.links.get(pack_id).cloned());
        if more && let Some(held) = &held && held.links.next_relative_id.is_none() {
            return Ok(held.clone());
        }
        let relative_id = if more { held.as_ref().and_then(|held| held.links.next_relative_id.clone()) } else { None };
        let response: rise_response::stickers::GetPackInviteLinks = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_invite_links(),
                rise_request::stickers::GetPackInviteLinks {
                    include_revoked: Some(true),
                    limit: Some(Self::PAGE_SIZE),
                    pack_id: Some(pack_id.to_owned()),
                    relative_id,
                },
            )
            .await?;
        Self::check(&response.error)?;
        let mut page = RisePackInviteLinksPage {
            links: RisePackPage {
                items: response.invite_links.clone(),
                next_relative_id: response.next_relative_id.clone(),
                total_count: None,
            },
            stats: response.stats.clone(),
        };
        if more && let Some(held) = &held {
            page.links.items = Self::merge(&held.links.items, response.invite_links, |link| link.id.clone());
        }
        self.state.with(|s| {
            s.links.insert(pack_id.to_owned(), page.clone());
        });
        self.schedule_publish();
        Ok(page)
    }

    pub async fn create_invite_link(
        &self,
        pack_id: &str,
        name: Option<String>,
        expires_at_ms: Option<i64>,
        usage_limit: Option<i64>,
        creates_join_request: bool,
    ) -> anyhow::Result<RisePackInviteLink> {
        let response: rise_response::stickers::CreatePackInviteLink = self
            .rpc
            .call(
                RiseConversationEndpoints::create_pack_invite_link(),
                rise_request::stickers::CreatePackInviteLink {
                    creates_join_request: Some(creates_join_request),
                    expires_at_ms,
                    name,
                    pack_id: Some(pack_id.to_owned()),
                    usage_limit,
                },
            )
            .await?;
        let Some(link) = response else {
            return Err(anyhow::Error::new(RisePackCollaborationFailure { code: String::new() }));
        };
        self.upsert_link(&link, pack_id, true);
        self.refresh_stats(pack_id).await;
        Ok(link)
    }

    pub async fn edit_invite_link(
        &self,
        link: &RisePackInviteLink,
        name: Option<String>,
        expires_at_ms: Option<i64>,
        usage_limit: Option<i64>,
        creates_join_request: bool,
    ) -> anyhow::Result<RisePackInviteLink> {
        let response: rise_response::stickers::EditPackInviteLink = self
            .rpc
            .call(
                RiseConversationEndpoints::edit_pack_invite_link(),
                rise_request::stickers::EditPackInviteLink {
                    clear_expires_at: expires_at_ms.is_none().then_some(true),
                    clear_usage_limit: usage_limit.is_none().then_some(true),
                    creates_join_request: Some(creates_join_request),
                    expires_at_ms,
                    link_id: Some(link.id.clone()),
                    name,
                    usage_limit,
                },
            )
            .await?;
        let Some(edited) = response else {
            return Err(anyhow::Error::new(RisePackCollaborationFailure { code: String::new() }));
        };
        self.upsert_link(&edited, &link.pack_id, false);
        self.refresh_stats(&link.pack_id).await;
        Ok(edited)
    }

    pub async fn revoke_invite_link(&self, link: &RisePackInviteLink) -> anyhow::Result<()> {
        let response: rise_response::stickers::RevokePackInviteLink = self
            .rpc
            .call(
                RiseConversationEndpoints::revoke_pack_invite_link(),
                rise_request::stickers::RevokePackInviteLink { link_id: Some(link.id.clone()) },
            )
            .await?;
        Self::check(&response.error)?;
        let changed = self.state.with(|s| {
            let mut changed = false;
            if let Some(revoked) = &response.invite_link {
                changed |= s.upsert_link(revoked, &link.pack_id, false);
            }
            if let Some(primary) = &response.new_primary_link {
                changed |= s.upsert_link(primary, &link.pack_id, true);
            }
            changed
        });
        if changed {
            self.schedule_publish();
        }
        self.refresh_stats(&link.pack_id).await;
        Ok(())
    }

    async fn refresh_stats(&self, pack_id: &str) {
        if !self.state.with(|s| s.links.contains_key(pack_id)) {
            return;
        }
        let response: anyhow::Result<rise_response::stickers::GetPackInviteLinks> = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_invite_links(),
                rise_request::stickers::GetPackInviteLinks {
                    include_revoked: Some(false),
                    limit: Some(1),
                    pack_id: Some(pack_id.to_owned()),
                    relative_id: None,
                },
            )
            .await;
        let Ok(response) = response else { return };
        if !response.error.is_empty() {
            return;
        }
        let Some(stats) = response.stats else { return };
        let changed = self.state.with(|s| {
            let Some(page) = s.links.get_mut(pack_id) else { return false };
            page.stats = Some(stats);
            true
        });
        if changed {
            self.schedule_publish();
        }
    }

    pub async fn link_joiners(&self, link_id: &str, more: bool) -> anyhow::Result<RisePackPage<RisePackCoauthor>> {
        let held = self.state.with(|s| s.joiners.get(link_id).cloned());
        if more && let Some(held) = &held && held.next_relative_id.is_none() {
            return Ok(held.clone());
        }
        let relative_id = if more { held.as_ref().and_then(|held| held.next_relative_id.clone()) } else { None };
        let response: rise_response::stickers::GetPackInviteLinkJoiners = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_invite_link_joiners(),
                rise_request::stickers::GetPackInviteLinkJoiners {
                    limit: Some(Self::PAGE_SIZE),
                    link_id: Some(link_id.to_owned()),
                    relative_id,
                },
            )
            .await?;
        Self::check(&response.error)?;
        let mut page = RisePackPage {
            items: response.joiners.clone(),
            next_relative_id: response.next_relative_id.clone(),
            total_count: None,
        };
        if more && let Some(held) = &held {
            page.items = Self::merge(&held.items, response.joiners, |joiner| joiner.user_id.clone());
        }
        self.state.with(|s| {
            s.joiners.insert(link_id.to_owned(), page.clone());
        });
        self.schedule_publish();
        Ok(page)
    }

    pub async fn invite_preview(&self, code: &str) -> anyhow::Result<RisePackInvitePreview> {
        let response: rise_response::stickers::GetPackInvitePreview = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_invite_preview(),
                rise_request::stickers::GetPackInvitePreview { code: Some(code.to_owned()) },
            )
            .await?;
        Self::check(&response.error)?;
        Ok(response)
    }

    pub async fn join(&self, code: &str) -> anyhow::Result<RisePackJoinResult> {
        let response: rise_response::stickers::JoinPackByLink = self
            .rpc
            .call(
                RiseConversationEndpoints::join_pack_by_link(),
                rise_request::stickers::JoinPackByLink { code: Some(code.to_owned()) },
            )
            .await?;
        Self::check(&response.error)?;
        Ok(response)
    }

    pub async fn join_requests(
        &self,
        pack_id: &str,
        more: bool,
    ) -> anyhow::Result<RisePackPage<RisePackJoinRequest>> {
        let held = self.state.with(|s| s.requests.get(pack_id).cloned());
        if more && let Some(held) = &held && held.next_relative_id.is_none() {
            return Ok(held.clone());
        }
        let relative_id = if more { held.as_ref().and_then(|held| held.next_relative_id.clone()) } else { None };
        let response: rise_response::stickers::GetPackJoinRequests = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_join_requests(),
                rise_request::stickers::GetPackJoinRequests {
                    limit: Some(Self::PAGE_SIZE),
                    pack_id: Some(pack_id.to_owned()),
                    relative_id,
                },
            )
            .await?;
        Self::check(&response.error)?;
        let mut page = RisePackPage {
            items: response.requests.clone(),
            next_relative_id: response.next_relative_id.clone(),
            total_count: Some(response.total_count),
        };
        if more && let Some(held) = &held {
            page.items = Self::merge(&held.items, response.requests, |request| request.id.clone());
        }
        self.state.with(|s| {
            s.requests.insert(pack_id.to_owned(), page.clone());
        });
        self.schedule_publish();
        Ok(page)
    }

    pub async fn review_join_request(&self, request: &RisePackJoinRequest, approve: bool) -> anyhow::Result<()> {
        let _: rise_response::stickers::ReviewPackJoinRequest = self
            .rpc
            .call(
                RiseConversationEndpoints::review_pack_join_request(),
                rise_request::stickers::ReviewPackJoinRequest {
                    approve: Some(approve),
                    request_id: Some(request.id.clone()),
                },
            )
            .await?;
        let changed = self.state.with(|s| {
            let mut changed = false;
            if let Some(page) = s.requests.get_mut(&request.pack_id) {
                page.items.retain(|item| item.id != request.id);
                page.total_count = page.total_count.map(|total| (total - 1).max(0));
                changed = true;
            }
            if approve && s.coauthors.remove(&request.pack_id).is_some() {
                changed = true;
            }
            changed
        });
        if changed {
            self.schedule_publish();
        }
        Ok(())
    }

    pub async fn coauthors(&self, pack_id: &str, more: bool) -> anyhow::Result<RisePackPage<RisePackCoauthor>> {
        let held = self.state.with(|s| s.coauthors.get(pack_id).cloned());
        if more && let Some(held) = &held && held.next_relative_id.is_none() {
            return Ok(held.clone());
        }
        let relative_id = if more { held.as_ref().and_then(|held| held.next_relative_id.clone()) } else { None };
        let response: rise_response::stickers::GetPackCoauthors = self
            .rpc
            .call(
                RiseConversationEndpoints::get_pack_coauthors(),
                rise_request::stickers::GetPackCoauthors {
                    limit: Some(Self::PAGE_SIZE),
                    pack_id: Some(pack_id.to_owned()),
                    relative_id,
                },
            )
            .await?;
        Self::check(&response.error)?;
        let mut page = RisePackPage {
            items: response.coauthors.clone(),
            next_relative_id: response.next_relative_id.clone(),
            total_count: Some(response.total_count),
        };
        if more && let Some(held) = &held {
            page.items = Self::merge(&held.items, response.coauthors, |coauthor| coauthor.user_id.clone());
        }
        self.state.with(|s| {
            s.coauthors.insert(pack_id.to_owned(), page.clone());
        });
        self.schedule_publish();
        Ok(page)
    }

    pub async fn update_rights(
        &self,
        pack_id: &str,
        coauthor_id: &str,
        rights: &RisePackRights,
    ) -> anyhow::Result<RisePackCoauthor> {
        let response: rise_response::stickers::UpdatePackCoauthorRights = self
            .rpc
            .call(
                RiseConversationEndpoints::update_pack_coauthor_rights(),
                rise_request::stickers::UpdatePackCoauthorRights {
                    coauthor_id: Some(coauthor_id.to_owned()),
                    pack_id: Some(pack_id.to_owned()),
                    rights: Some(rise_request::stickers::update_pack_coauthor_rights::Rights {
                        can_add_stickers: Some(rights.can_add_stickers),
                        can_delete_stickers: Some(rights.can_delete_stickers),
                        can_edit_pack: Some(rights.can_edit_pack),
                        can_edit_stickers: Some(rights.can_edit_stickers),
                    }),
                },
            )
            .await?;
        let Some(updated) = response else {
            return Err(anyhow::Error::new(RisePackCollaborationFailure { code: String::new() }));
        };
        let changed = self.state.with(|s| {
            let Some(page) = s.coauthors.get_mut(pack_id) else { return false };
            let Some(index) = page.items.iter().position(|item| item.user_id == coauthor_id) else { return false };
            page.items[index] = updated.clone();
            true
        });
        if changed {
            self.schedule_publish();
        }
        Ok(updated)
    }

    pub async fn remove_coauthor(&self, pack_id: &str, coauthor_id: &str) -> anyhow::Result<()> {
        let response: rise_response::stickers::RemovePackCoauthor = self
            .rpc
            .call(
                RiseConversationEndpoints::remove_pack_coauthor(),
                rise_request::stickers::RemovePackCoauthor {
                    coauthor_id: Some(coauthor_id.to_owned()),
                    pack_id: Some(pack_id.to_owned()),
                },
            )
            .await?;
        Self::check(&response.error)?;
        let changed = self.state.with(|s| {
            let Some(page) = s.coauthors.get_mut(pack_id) else { return false };
            page.items.retain(|item| item.user_id != coauthor_id);
            page.total_count = page.total_count.map(|total| (total - 1).max(0));
            true
        });
        if changed {
            self.schedule_publish();
        }
        Ok(())
    }

    pub async fn leave(&self, pack_id: &str) -> anyhow::Result<()> {
        let response: rise_response::stickers::LeavePack = self
            .rpc
            .call(
                RiseConversationEndpoints::leave_pack(),
                rise_request::stickers::LeavePack { pack_id: Some(pack_id.to_owned()) },
            )
            .await?;
        Self::check(&response.error)?;
        self.forget(pack_id);
        Ok(())
    }

    pub async fn transfer_ownership(&self, pack_id: &str, new_owner_id: &str) -> anyhow::Result<()> {
        let response: rise_response::stickers::TransferPackOwnership = self
            .rpc
            .call(
                RiseConversationEndpoints::transfer_pack_ownership(),
                rise_request::stickers::TransferPackOwnership {
                    new_owner_id: Some(new_owner_id.to_owned()),
                    pack_id: Some(pack_id.to_owned()),
                },
            )
            .await?;
        Self::check(&response.error)?;
        self.forget(pack_id);
        Ok(())
    }

    fn schedule_publish(&self) {
        let scheduled = self.state.with(|s| {
            if s.publish_pending || s.is_shutdown {
                return false;
            }
            s.publish_pending = true;
            true
        });
        if !scheduled {
            return;
        }
        let this = self.this.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            if let Some(repository) = this.upgrade() {
                repository.flush_publish().await;
            }
        }));
    }

    async fn flush_publish(&self) {
        let value = self.state.with(|s| {
            s.publish_pending = false;
            if s.is_shutdown {
                return None;
            }
            s.revision = s.revision.wrapping_add(1);
            Some(RisePackCollaborationSnapshot {
                account_id: self.account_id.clone(),
                revision: s.revision,
                links: s.links.clone(),
                coauthors: s.coauthors.clone(),
                requests: s.requests.clone(),
                joiners: s.joiners.clone(),
            })
        });
        let Some(value) = value else { return };
        Self::on_main(move |cx| {
            RisePackCollaborationPresentation::shared(cx).update(cx, |presentation, cx| presentation.publish(value, cx));
        })
        .await;
    }

    fn forget(&self, pack_id: &str) {
        self.state.with(|s| s.forget(pack_id));
        self.schedule_publish();
    }

    fn upsert_link(&self, link: &RisePackInviteLink, pack_id: &str, front: bool) {
        if self.state.with(|s| s.upsert_link(link, pack_id, front)) {
            self.schedule_publish();
        }
    }

    fn check(error: &str) -> anyhow::Result<()> {
        if error.is_empty() {
            return Ok(());
        }
        Err(anyhow::Error::new(RisePackCollaborationFailure { code: error.to_owned() }))
    }

    fn merge<Item: Clone>(held: &[Item], fresh: Vec<Item>, id: impl Fn(&Item) -> String) -> Vec<Item> {
        let mut seen: HashSet<String> = held.iter().map(&id).collect();
        let mut merged = held.to_vec();
        merged.extend(fresh.into_iter().filter(|item| seen.insert(id(item))));
        merged
    }
}
