use rise_canon::rise_proto;

pub type BotMentionCandidateWireDTO = rise_proto::bot::MentionCandidate;

pub type BotMentionCandidatesGatewayDTO = rise_proto::bot::GetMentionCandidatesResponse;
pub type BotInlineIdentityWireDTO = rise_proto::bot::InlineBotInfo;

pub type BotInlineCommandsGatewayDTO = rise_proto::bot::GetInlineBotCommandsResponse;

pub type BotChatCommandsGatewayDTO = rise_proto::bot::GetChatBotCommandsResponse;

pub type BotInlinePreviewMediaWireDTO = rise_proto::bot::InlineResultMedia;

pub type BotInlinePreviewMessageWireDTO = rise_proto::bot::InlineSendMessage;
pub type BotInlineResultWireDTO = rise_proto::bot::InlineResult;

pub type BotInlineResultsGatewayDTO = rise_proto::bot::QueryInlineResultsResponse;
