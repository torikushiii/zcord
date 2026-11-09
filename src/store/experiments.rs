use crate::discord::model::{Experiments, Guild};

const SERVER_THEME_USER: &str = "2026-05-server-theme-user";
const SERVER_THEME: &str = "2026-04_server_theme";
const SERVER_THEME_APEX: &str = "2026-06-server-theme-apex-shadow";
const GAME_SERVERS: &str = "2025-08_portkey_enabled";

pub(super) fn server_theme(experiments: &Experiments, guild: &Guild) -> bool {
    experiments.user_variant(SERVER_THEME_USER) == Some(1)
        && match experiments.guild_treatment(SERVER_THEME, guild) {
            Some(0) => false,
            Some(1 | 2) => true,
            _ => matches!(
                experiments.guild_variant(SERVER_THEME_APEX, guild),
                Some(3 | 4)
            ),
        }
}

pub(super) fn game_servers(experiments: &Experiments, guild: &Guild) -> bool {
    experiments.guild_treatment(GAME_SERVERS, guild) == Some(1)
}
