use rise_canon::{rise_proto, rise_view};

pub type UnreadMentionMarksWireDTO = rise_proto::message::ProtoUnreadMentionMarks;

pub type UnreadReactionMarkWireDTO = rise_proto::message::ProtoUnreadReactionMark;

pub type UnreadReactionMarksWireDTO = rise_proto::message::ProtoUnreadReactionMarks;
pub type GetUnreadMarksGatewayDTO = rise_view::message::UnreadMarks;

pub type MarkMessageAsMentionedReadGatewayDTO = rise_proto::message::ProtoMarkMessageAsMentionedReadResponse;

pub type MarkReactionsAsReadGatewayDTO = rise_proto::message::ProtoMarkReactionsAsReadResponse;
pub type ClearUnreadMarksGatewayDTO = rise_proto::message::ProtoClearUnreadMarksResponse;
