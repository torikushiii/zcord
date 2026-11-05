use serde::Deserialize;

#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub settings: SettingsProto,
    #[serde(default)]
    pub partial: bool,
}

#[derive(Deserialize)]
pub struct SettingsProto {
    #[serde(rename = "type")]
    pub kind: u8,
    pub proto: String,
}
