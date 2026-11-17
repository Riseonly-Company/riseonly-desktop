use std::collections::HashSet;

use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::{BotInlineCommandsModel, BotInlineIdentityModel};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotCommandModel;

/// one command of one bot seated in a group, as the slash panel lists it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatGroupBotCommandModel {
    pub bot: BotInlineIdentityModel,
    pub command: BotCommandModel,
}

impl ChatGroupBotCommandModel {
    pub fn id(&self) -> String {
        format!("{}/{}", self.bot.id, self.command.command)
    }

    /// text inserted into the composer, eg /cmd@tag when the group has several bots else /cmd
    pub fn insertion(&self, bot_count: usize) -> String {
        if bot_count > 1 {
            format!("/{}@{} ", self.command.command, self.bot.tag)
        } else {
            format!("/{} ", self.command.command)
        }
    }
}

pub enum ChatGroupBotCommands {}

impl ChatGroupBotCommands {
    /// flattens per bot command lists keeping the server order of bots and commands
    pub fn flatten(groups: &[BotInlineCommandsModel]) -> Vec<ChatGroupBotCommandModel> {
        groups
            .iter()
            .flat_map(|group| -> Vec<ChatGroupBotCommandModel> {
                let Some(bot) = &group.bot else {
                    return Vec::new();
                };
                group
                    .commands
                    .iter()
                    .map(|command| ChatGroupBotCommandModel { bot: bot.clone(), command: command.clone() })
                    .collect()
            })
            .collect()
    }

    pub fn bot_count(items: &[ChatGroupBotCommandModel]) -> usize {
        items.iter().map(|item| item.bot.id.as_str()).collect::<HashSet<_>>().len()
    }

    /// filters by the typed slash query, command prefix and an at tag suffix narrows to one bot
    pub fn filter(items: &[ChatGroupBotCommandModel], query: &str) -> Vec<ChatGroupBotCommandModel> {
        let mut parts = query.splitn(2, '@');
        let command_prefix = parts.next().unwrap_or_default().to_lowercase();
        let tag_prefix = parts.next().map(str::to_lowercase);
        items
            .iter()
            .filter(|item| {
                if !command_prefix.is_empty() && !item.command.command.to_lowercase().starts_with(&command_prefix) {
                    return false;
                }
                let Some(tag_prefix) = &tag_prefix else {
                    return true;
                };
                tag_prefix.is_empty() || item.bot.tag.to_lowercase().starts_with(tag_prefix.as_str())
            })
            .cloned()
            .collect()
    }
}
