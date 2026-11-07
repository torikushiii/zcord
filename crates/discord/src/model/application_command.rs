use super::serde_helpers::*;
use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
pub struct ApplicationCommandIndex {
    #[serde(default, deserialize_with = "entries")]
    pub application_commands: Vec<ApplicationCommand>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApplicationCommand {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub application_id: u64,
    #[serde(rename = "type", default = "chat_input")]
    pub kind: u8,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, deserialize_with = "entries")]
    pub options: Vec<CommandOption>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CommandOption {
    #[serde(rename = "type")]
    pub kind: u8,
}

fn chat_input() -> u8 {
    1
}

/// https://docs.discord.food/interactions/application-commands#application-command-option-type
const SUB_COMMAND: u8 = 1;
const SUB_COMMAND_GROUP: u8 = 2;

impl ApplicationCommandIndex {
    pub fn popular(self, application_id: u64, popular: &[u64]) -> Vec<ApplicationCommand> {
        let mut commands: Vec<_> = self
            .application_commands
            .into_iter()
            .filter(|command| {
                command.application_id == application_id
                    && command.kind == 1
                    && popular.contains(&command.id)
                    && !command
                        .options
                        .iter()
                        .any(|option| matches!(option.kind, SUB_COMMAND | SUB_COMMAND_GROUP))
            })
            .collect();
        commands.sort_by_key(|command| popular.iter().position(|id| *id == command.id));
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataAccess, ProfileApplication};
    use serde_json::json;

    #[test]
    fn popular_commands_skip_subcommands_and_follow_popularity() {
        let index: ApplicationCommandIndex = serde_json::from_value(json!({
            "application_commands": [
                {"id": "1", "application_id": "9", "type": 1, "name": "rank", "options": [{"type": 6}]},
                {"id": "2", "application_id": "9", "type": 1, "name": "slots", "options": [{"type": 1}]},
                {"id": "3", "application_id": "9", "type": 1, "name": "pet", "options": [{"type": 2}]},
                {"id": "4", "application_id": "9", "type": 1, "name": "daily"},
                {"id": "5", "application_id": "8", "type": 1, "name": "other"},
                {"id": "6", "application_id": "9", "type": 4, "name": "launch"},
                {"id": "7", "application_id": "9", "type": 1, "name": "help"}
            ]
        }))
        .unwrap();
        let names: Vec<_> = index
            .popular(9, &[4, 2, 1, 3, 5, 6])
            .into_iter()
            .map(|command| command.name)
            .collect();
        assert_eq!(names, ["daily", "rank"]);
    }

    #[test]
    fn data_access_reads_the_full_and_limited_intent_flags() {
        let application = |flags: u64| -> ProfileApplication {
            serde_json::from_value(json!({"id": "1", "flags": flags})).unwrap()
        };
        assert_eq!(
            application(10899456).data_access(),
            [
                DataAccess::Messages,
                DataAccess::Presence,
                DataAccess::Members
            ]
        );
        assert_eq!(
            application(1 << 13 | 1 << 19).data_access(),
            [DataAccess::Messages, DataAccess::Presence]
        );
        assert!(application(0).data_access().is_empty());
    }
}
