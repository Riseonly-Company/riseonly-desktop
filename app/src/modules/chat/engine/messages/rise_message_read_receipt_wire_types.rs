use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::RiseMessageReadReceiptSummaryKind;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageSenderWireDTO;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredReadReceipt {
    pub user: Option<MessageSenderWireDTO>,
    pub read_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseStoredReadReceipts {
    pub message_id: Option<String>,
    pub users: Option<Vec<MessageSenderWireDTO>>,
    pub is_have_more: Option<bool>,
    pub receipts: Option<Vec<RiseStoredReadReceipt>>,
    pub total_count: Option<i64>,
    pub summary_kind: Option<String>,
    pub representative_read_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMessageReadReceiptStoredResource {
    pub response: RiseStoredReadReceipts,
    pub refreshed_at_milliseconds: i64,
    pub is_local_hint: Option<bool>,
}

impl RiseMessageReadReceiptStoredResource {
    pub fn new(response: RiseStoredReadReceipts, refreshed_at_milliseconds: i64, is_local_hint: Option<bool>) -> Self {
        Self { response, refreshed_at_milliseconds, is_local_hint }
    }

    pub fn local_hint(message_id: &str, read_at_milliseconds: i64) -> Self {
        Self::new(
            RiseStoredReadReceipts {
                message_id: Some(message_id.to_owned()),
                users: None,
                is_have_more: None,
                receipts: None,
                total_count: None,
                summary_kind: Some(RiseMessageReadReceiptSummaryKind::Timestamp.raw_value().to_owned()),
                representative_read_at: Some(read_at_milliseconds),
            },
            0,
            Some(true),
        )
    }
}
