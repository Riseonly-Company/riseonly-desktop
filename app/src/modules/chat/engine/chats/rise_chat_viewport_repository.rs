use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use rise_engine::rise_store::rise_store_entities::{
    RiseEntityViewItem, RiseEntityViewPosition, RiseEntityWindow, RiseEntityWindowQuery,
};

use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::core::rise_conversation_codec::RiseConversationCodec;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationEntityStamp,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

#[derive(Clone)]
pub struct RiseChatViewportSnapshotRow {
    pub local_id: String,
    pub position: RiseEntityViewPosition,
    pub stamp: RiseConversationEntityStamp,
    pub chat: ChatInfo,
    pub content_version: i64,
}

/// page local payload window
/// membership and order stay in sqlite so an evicted folder needs one anchored read
#[derive(Clone)]
pub struct RiseChatViewportSnapshot {
    pub rows: Vec<RiseChatViewportSnapshotRow>,
    pub has_before: bool,
    pub has_after: bool,
    pub has_remote_after: bool,
    pub cursor: Option<String>,
}

fn chat_hash_value(chat: &ChatInfo) -> i64 {
    let mut hasher = DefaultHasher::new();
    format!("{chat:?}").hash(&mut hasher);
    hasher.finish() as i64
}

impl RiseChatRepository {
    pub async fn first_viewport_anchor(
        &self,
        kind: RiseChatListKind,
        ids: &[String],
    ) -> anyhow::Result<Option<String>> {
        for id in ids {
            let Some(local_id) = self.local_id(id).await? else {
                continue;
            };
            let position = self
                .engine
                .entity_view_position(
                    RiseConversationStoreLayout::chats(),
                    kind.store_view().raw_value().to_owned(),
                    local_id,
                )
                .await?;
            if position.is_some() {
                return Ok(Some(id.clone()));
            }
        }
        Ok(None)
    }

    pub async fn viewport(
        &self,
        kind: RiseChatListKind,
        folder: Option<&ChatFolder>,
        contacts: &HashSet<String>,
        anchor_chat_id: Option<&str>,
        before_count: i64,
        count: i64,
    ) -> anyhow::Result<RiseChatViewportSnapshot> {
        let limit = self.presentation_limit.min(count.max(1));
        let anchor_local_id = match anchor_chat_id {
            Some(anchor_chat_id) => self.local_id(anchor_chat_id).await?,
            None => None,
        };
        let anchor = match anchor_local_id {
            Some(local_id) => {
                self.engine
                    .entity_view_position(
                        RiseConversationStoreLayout::chats(),
                        kind.store_view().raw_value().to_owned(),
                        local_id,
                    )
                    .await?
            }
            None => None,
        };
        let before = before_count.max(0).min(limit - 1);
        let mut rows: Vec<RiseChatViewportSnapshotRow> = Vec::new();
        let mut has_before = false;
        if let Some(anchor) = anchor.as_ref() {
            if before > 0 {
                let (older, has_more) =
                    self.viewport_rows(kind, folder, contacts, Some(anchor), true, before).await?;
                rows = older;
                has_before = has_more;
            } else {
                let (older, _) = self.viewport_rows(kind, folder, contacts, Some(anchor), true, 1).await?;
                has_before = !older.is_empty();
            }
            let exact = self
                .engine
                .entity_window(Self::viewport_query(
                    kind,
                    RiseEntityWindow::Around { anchor: anchor.clone(), before_limit: 0, after_limit: 0 },
                ))
                .await?;
            rows.extend(self.viewport_models(&exact.items, folder, contacts).await?);
        }
        let (after_rows, after_has_more) = self
            .viewport_rows(kind, folder, contacts, anchor.as_ref(), false, limit - rows.len() as i64)
            .await?;
        rows.extend(after_rows);
        if let Some(anchor) = anchor.as_ref() {
            if (rows.len() as i64) < limit && has_before {
                let (preceding, has_more) = self
                    .viewport_rows(kind, folder, contacts, Some(anchor), true, before + limit - rows.len() as i64)
                    .await?;
                let dropped = usize::try_from(before).unwrap_or(0).min(rows.len());
                rows.drain(..dropped);
                rows.splice(0..0, preceding);
                has_before = has_more;
            }
        }
        let state = self
            .engine
            .entity_view_state(RiseConversationStoreLayout::chats(), kind.store_view().raw_value().to_owned())
            .await?;
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.chat.id.clone()));
        if let Some(folder) = folder {
            if kind == RiseChatListKind::Inbox {
                let lower_edge = if has_before { rows.first().map(|row| row.position.clone()) } else { None };
                let upper_edge = if after_has_more { rows.last().map(|row| row.position.clone()) } else { None };
                rows = self
                    .merging_archived_folder_rows(rows, folder, contacts, lower_edge.as_ref(), upper_edge.as_ref())
                    .await?;
            }
        }
        Ok(RiseChatViewportSnapshot {
            rows,
            has_before,
            has_after: after_has_more,
            has_remote_after: state.as_ref().is_some_and(|state| state.has_more_after),
            cursor: RiseConversationCodec::decode_cursor(state.as_ref().and_then(|state| state.after_cursor.as_deref())),
        })
    }

    const ARCHIVED_FOLDER_SCAN_LIMIT: usize = 1_000;

    fn precedes(lhs: &RiseEntityViewPosition, rhs: &RiseEntityViewPosition) -> bool {
        if lhs.position_key != rhs.position_key {
            lhs.position_key < rhs.position_key
        } else {
            lhs.local_id < rhs.local_id
        }
    }

    async fn merging_archived_folder_rows(
        &self,
        rows: Vec<RiseChatViewportSnapshotRow>,
        folder: &ChatFolder,
        contacts: &HashSet<String>,
        lower_edge: Option<&RiseEntityViewPosition>,
        upper_edge: Option<&RiseEntityViewPosition>,
    ) -> anyhow::Result<Vec<RiseChatViewportSnapshotRow>> {
        let archived = self.archived_folder_rows(folder, contacts).await?;
        if archived.is_empty() {
            return Ok(rows);
        }
        let mut present: HashSet<String> = rows.iter().map(|row| row.chat.id.clone()).collect();
        let inside: Vec<RiseChatViewportSnapshotRow> = archived
            .into_iter()
            .filter(|row| {
                lower_edge.is_none_or(|edge| Self::precedes(edge, &row.position))
                    && upper_edge.is_none_or(|edge| Self::precedes(&row.position, edge))
                    && present.insert(row.chat.id.clone())
            })
            .collect();
        if inside.is_empty() {
            return Ok(rows);
        }
        let mut merged = rows;
        merged.extend(inside);
        merged.sort_by(|lhs, rhs| {
            if Self::precedes(&lhs.position, &rhs.position) {
                std::cmp::Ordering::Less
            } else if Self::precedes(&rhs.position, &lhs.position) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        Ok(merged)
    }

    async fn archived_folder_rows(
        &self,
        folder: &ChatFolder,
        contacts: &HashSet<String>,
    ) -> anyhow::Result<Vec<RiseChatViewportSnapshotRow>> {
        let page_limit = usize::try_from(self.page_limit).unwrap_or(0);
        let mut rows: Vec<RiseChatViewportSnapshotRow> = Vec::new();
        let mut cursor: Option<RiseEntityViewPosition> = None;
        let mut scanned = 0usize;
        while scanned < Self::ARCHIVED_FOLDER_SCAN_LIMIT {
            let window = match cursor.take() {
                Some(anchor) => RiseEntityWindow::After { anchor, limit: page_limit },
                None => RiseEntityWindow::First { limit: page_limit },
            };
            let query = RiseEntityWindowQuery::new(
                RiseConversationStoreLayout::chats(),
                RiseConversationStoreLayoutChatView::Archive.raw_value(),
                window,
            );
            let result = self.engine.entity_window(query).await?;
            rows.extend(self.viewport_models(&result.items, Some(folder), contacts).await?);
            scanned += result.items.len();
            let Some(edge) = result.items.last() else {
                break;
            };
            cursor = Some(RiseEntityViewPosition::new(edge.position_key.clone(), edge.entity.local_id.clone()));
            if !result.has_cached_after {
                break;
            }
        }
        Ok(rows)
    }

    fn viewport_query(kind: RiseChatListKind, window: RiseEntityWindow) -> RiseEntityWindowQuery {
        RiseEntityWindowQuery::new(RiseConversationStoreLayout::chats(), kind.store_view().raw_value(), window)
    }

    async fn viewport_rows(
        &self,
        kind: RiseChatListKind,
        folder: Option<&ChatFolder>,
        contacts: &HashSet<String>,
        boundary: Option<&RiseEntityViewPosition>,
        backwards: bool,
        count: i64,
    ) -> anyhow::Result<(Vec<RiseChatViewportSnapshotRow>, bool)> {
        if count <= 0 {
            return Ok((Vec::new(), true));
        }
        let page_limit = usize::try_from(self.page_limit).unwrap_or(0);
        let mut cursor = boundary.cloned();
        let mut rows: Vec<RiseChatViewportSnapshotRow> = Vec::new();
        let mut seen = HashSet::new();
        while rows.len() as i64 <= count {
            let window = match cursor.take() {
                Some(anchor) if backwards => RiseEntityWindow::Before { anchor, limit: page_limit },
                Some(anchor) => RiseEntityWindow::After { anchor, limit: page_limit },
                None => RiseEntityWindow::First { limit: page_limit },
            };
            let result = self.engine.entity_window(Self::viewport_query(kind, window)).await?;
            let mut batch = self.viewport_models(&result.items, folder, contacts).await?;
            batch.retain(|row| seen.insert(row.local_id.clone()));
            if backwards {
                rows.splice(0..0, batch);
            } else {
                rows.extend(batch);
            }
            let edge = if backwards { result.items.first() } else { result.items.last() };
            let Some(edge) = edge else {
                break;
            };
            cursor = Some(RiseEntityViewPosition::new(edge.position_key.clone(), edge.entity.local_id.clone()));
            if !(if backwards { result.has_cached_before } else { result.has_cached_after }) {
                break;
            }
        }
        let has_more = rows.len() as i64 > count;
        let kept = usize::try_from(count).unwrap_or(0).min(rows.len());
        if backwards {
            rows.drain(..rows.len() - kept);
        } else {
            rows.truncate(kept);
        }
        Ok((rows, has_more))
    }

    async fn viewport_models(
        &self,
        items: &[RiseEntityViewItem],
        folder: Option<&ChatFolder>,
        contacts: &HashSet<String>,
    ) -> anyhow::Result<Vec<RiseChatViewportSnapshotRow>> {
        let records = self.decoded_record_cache.records(items).await?;
        let mut rows = Vec::new();
        for (item, record) in items.iter().zip(records.iter()) {
            if record.tombstone.is_some() {
                continue;
            }
            let chat = record.wire.to_chat_info();
            if !folder.is_none_or(|folder| folder.contains(&chat, &|id: &str| contacts.contains(id))) {
                continue;
            }
            let content_version = chat_hash_value(&chat);
            rows.push(RiseChatViewportSnapshotRow {
                local_id: item.entity.local_id.clone(),
                position: RiseEntityViewPosition::new(item.position_key.clone(), item.entity.local_id.clone()),
                stamp: RiseConversationEntityStamp {
                    version: item.entity.version,
                    updated_at_milliseconds: item.entity.updated_at_milliseconds,
                },
                chat,
                content_version,
            });
        }
        Ok(rows)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseChatSelectedModels {
    pub chats: HashMap<String, ChatInfo>,
}

impl RiseChatRepository {
    pub async fn selected_models(&self, ids: &[String]) -> anyhow::Result<RiseChatSelectedModels> {
        let mut result = HashMap::new();
        let unique: HashSet<&String> = ids.iter().collect();
        for id in unique {
            if let Some(record) = self.chat(id).await? {
                if record.tombstone.is_none() {
                    result.insert(id.clone(), record.wire.to_chat_info());
                }
            }
        }
        Ok(RiseChatSelectedModels { chats: result })
    }
}
