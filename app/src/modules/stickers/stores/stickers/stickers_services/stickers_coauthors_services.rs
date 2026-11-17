use crate::modules::stickers::engine::rise_pack_collaboration_presentation::RisePackCollaborationSnapshot;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::{
    RisePackCoauthor, RisePackInviteLink, RisePackInviteStats, RisePackJoinRequest,
};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

impl StickersServicesStore {
    fn collaboration(&self) -> &RisePackCollaborationSnapshot {
        &self.collaboration
    }

    pub fn pack_invite_links(&self, pack_id: &str) -> Option<Vec<RisePackInviteLink>> {
        self.collaboration().links.get(pack_id).map(|page| page.links.items.clone())
    }

    pub fn pack_invite_stats(&self, pack_id: &str) -> Option<RisePackInviteStats> {
        self.collaboration().links.get(pack_id)?.stats.clone()
    }

    pub fn pack_invite_links_has_more(&self, pack_id: &str) -> bool {
        self.collaboration().links.get(pack_id).is_some_and(|page| page.links.next_relative_id.is_some())
    }

    pub fn pack_invite_link(&self, pack_id: &str, link_id: &str) -> Option<RisePackInviteLink> {
        self.collaboration().links.get(pack_id)?.links.items.iter().find(|link| link.id == link_id).cloned()
    }

    pub fn pack_link_joiners(&self, link_id: &str) -> Option<Vec<RisePackCoauthor>> {
        self.collaboration().joiners.get(link_id).map(|page| page.items.clone())
    }

    pub fn pack_link_joiners_has_more(&self, link_id: &str) -> bool {
        self.collaboration().joiners.get(link_id).is_some_and(|page| page.next_relative_id.is_some())
    }

    pub fn pack_join_requests(&self, pack_id: &str) -> Option<Vec<RisePackJoinRequest>> {
        self.collaboration().requests.get(pack_id).map(|page| page.items.clone())
    }

    pub fn pack_join_requests_count(&self, pack_id: &str) -> i64 {
        let Some(page) = self.collaboration().requests.get(pack_id) else { return 0 };
        page.total_count.unwrap_or(page.items.len() as i64)
    }

    pub fn pack_join_requests_has_more(&self, pack_id: &str) -> bool {
        self.collaboration().requests.get(pack_id).is_some_and(|page| page.next_relative_id.is_some())
    }

    pub fn pack_coauthors(&self, pack_id: &str) -> Option<Vec<RisePackCoauthor>> {
        self.collaboration().coauthors.get(pack_id).map(|page| page.items.clone())
    }

    pub fn pack_coauthors_count(&self, pack_id: &str) -> i64 {
        let Some(page) = self.collaboration().coauthors.get(pack_id) else { return 0 };
        page.total_count.unwrap_or(page.items.len() as i64)
    }

    pub fn pack_coauthors_has_more(&self, pack_id: &str) -> bool {
        self.collaboration().coauthors.get(pack_id).is_some_and(|page| page.next_relative_id.is_some())
    }

    pub fn pack_coauthor(&self, pack_id: &str, user_id: &str) -> Option<RisePackCoauthor> {
        self.collaboration().coauthors.get(pack_id)?.items.iter().find(|coauthor| coauthor.user_id == user_id).cloned()
    }
}
