use super::*;

#[derive(Clone, Debug)]
pub struct WidgetConfig {
    pub application_id: u64,
    pub display_name: Option<String>,
    /// `widget_top.components.title.fields.text`
    pub title: Option<WidgetText>,
    /// `widget_top` subtitles in numeric order (subtitle_2 before subtitle_10).
    pub subtitles: Vec<WidgetSubtitle>,
    /// `widget_top.components.hero_image.fields.image`: an uploaded asset or an identity-data key.
    pub hero: Option<WidgetImage>,
    /// `widget_top.components.contained_image.fields.image`.
    pub contained: Option<WidgetImage>,
    /// `widget_bottom` stats in numeric order (stat_2 before stat_10).
    pub stats: Vec<WidgetStat>,
    /// `widget_bottom` collection items in numeric order (item_2 before item_10).
    pub collection: Vec<WidgetItem>,
    /// `surfaces.mini_profile`, the surface Discord renders in the profile popout.
    pub mini: Option<MiniProfile>,
    /// `surfaces.activity_accessory`, the strip under the current-activity card.
    pub accessory: Option<WidgetAccessory>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetAccessory {
    pub layout: String,
    pub label: Option<WidgetText>,
    pub text: Option<WidgetText>,
    pub icon: Option<WidgetImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniProfile {
    pub layout: String,
    pub text: Option<WidgetText>,
    pub label: Option<WidgetText>,
    pub icon: Option<WidgetImage>,
    pub image: Option<WidgetImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WidgetImage {
    Asset(WidgetAsset),
    /// A key into identity stats; the UI resolves it to an image URL. `fallback` is the config's
    /// stand-in art for when that stat is missing.
    Data {
        key: String,
        fallback: Option<Box<WidgetImage>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WidgetText {
    /// `value_type: "data"`: a key into identity stats, or "username".
    Data(String),
    /// `value_type: "custom_string"`.
    Literal(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSubtitle {
    pub text: WidgetText,
    pub label: Option<WidgetText>,
    pub icon: Option<WidgetAsset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetAsset {
    pub asset_id: u64,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetStat {
    pub value: WidgetText,
    pub label: WidgetText,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetItem {
    pub name: Option<WidgetText>,
    pub description: Option<WidgetText>,
    pub image: Option<WidgetImage>,
}

pub fn parse_featured_widget_configs(value: Value) -> HashMap<u64, WidgetConfig> {
    let mut configs = HashMap::new();
    let Some(by_application) = value.get("configs").and_then(Value::as_object) else {
        return configs;
    };
    for (application_id, value) in by_application {
        let Ok(application_id) = application_id.parse::<u64>() else {
            continue;
        };
        let Some(value) = value.as_array() else {
            continue;
        };
        if let Some(config) = select_widget_config(application_id, value) {
            configs.insert(application_id, config);
        }
    }
    configs
}

pub fn parse_application_widget_configs(value: Value, application_id: u64) -> Option<WidgetConfig> {
    let configs = match &value {
        Value::Array(configs) => configs.as_slice(),
        Value::Object(object) => match object.get("configs") {
            Some(Value::Array(configs)) => configs.as_slice(),
            Some(Value::Object(by_application)) => {
                match by_application.get(&application_id.to_string()) {
                    Some(Value::Array(configs)) => configs.as_slice(),
                    _ => by_application.values().find_map(Value::as_array)?,
                }
            }
            // A few endpoints return the config object directly.
            _ if object.contains_key("surfaces") => {
                return select_widget_config(application_id, std::slice::from_ref(&value));
            }
            _ => return None,
        },
        _ => return None,
    };
    select_widget_config(application_id, configs)
}

fn select_widget_config(application_id: u64, configs: &[Value]) -> Option<WidgetConfig> {
    let configs: Vec<&Value> = configs.iter().filter(|config| config.is_object()).collect();
    let selected = configs
        .iter()
        .find(|config| config.get("status").and_then(Value::as_str) == Some("published"))
        .or_else(|| configs.first())?;
    Some(widget_config_from_value(application_id, selected))
}

fn widget_config_from_value(application_id: u64, config: &Value) -> WidgetConfig {
    let top = config
        .get("surfaces")
        .and_then(|surfaces| surfaces.get("widget_top"));
    let bottom = config
        .get("surfaces")
        .and_then(|surfaces| surfaces.get("widget_bottom"));
    let assets = resolved_assets(config);
    let mini = mini_profile_from_value(config, &assets);
    let accessory = widget_accessory_from_value(config, &assets);
    let title = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("title"))
        .and_then(|title| title.get("fields"))
        .and_then(|fields| fields.get("text"))
        .and_then(widget_text);
    let subtitles = top
        .and_then(|top| top.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut subtitles: Vec<(u32, WidgetSubtitle)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("subtitle_")?.parse().ok()?;
                    let fields = component.get("fields")?;
                    let text = widget_text(fields.get("text")?)?;
                    Some((
                        index,
                        WidgetSubtitle {
                            text,
                            label: fields.get("label").and_then(widget_text),
                            icon: fields
                                .get("icon")
                                .and_then(|icon| widget_asset(icon, &assets)),
                        },
                    ))
                })
                .collect();
            subtitles.sort_by_key(|(index, _)| *index);
            subtitles
                .into_iter()
                .map(|(_, subtitle)| subtitle)
                .collect()
        })
        .unwrap_or_default();
    let hero = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("hero_image"))
        .and_then(|hero| hero.get("fields"))
        .and_then(|fields| fields.get("image"))
        .and_then(|image| widget_image(image, &assets));
    let contained = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("contained_image"))
        .and_then(|contained| contained.get("fields"))
        .and_then(|fields| fields.get("image"))
        .and_then(|image| widget_image(image, &assets));
    let stats = bottom
        .filter(|bottom| {
            bottom.get("layout").and_then(Value::as_str) == Some("widget_bottom_stats")
        })
        .and_then(|bottom| bottom.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut stats: Vec<(u32, WidgetStat)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("stat_")?.parse().ok()?;
                    let fields = component.get("fields")?;
                    Some((
                        index,
                        WidgetStat {
                            value: widget_text(fields.get("value")?)?,
                            label: widget_text(fields.get("label")?)?,
                        },
                    ))
                })
                .collect();
            stats.sort_by_key(|(index, _)| *index);
            stats.into_iter().map(|(_, stat)| stat).collect()
        })
        .unwrap_or_default();
    let collection = bottom
        .filter(|bottom| {
            bottom.get("layout").and_then(Value::as_str) == Some("widget_bottom_collection")
        })
        .and_then(|bottom| bottom.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut items: Vec<(u32, WidgetItem)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("item_")?.parse().ok()?;
                    let fields = component.get("fields");
                    let name = fields
                        .and_then(|fields| fields.get("name"))
                        .and_then(widget_text);
                    let description = fields
                        .and_then(|fields| fields.get("description"))
                        .and_then(widget_text);
                    let image = fields
                        .and_then(|fields| fields.get("image"))
                        .and_then(|image| widget_image(image, &assets));
                    if name.is_none() && description.is_none() && image.is_none() {
                        return None;
                    }
                    Some((
                        index,
                        WidgetItem {
                            name,
                            description,
                            image,
                        },
                    ))
                })
                .collect();
            items.sort_by_key(|(index, _)| *index);
            items.into_iter().map(|(_, item)| item).collect()
        })
        .unwrap_or_default();
    WidgetConfig {
        application_id,
        display_name: config
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        title,
        subtitles,
        hero,
        contained,
        stats,
        collection,
        mini,
        accessory,
    }
}

fn widget_accessory_from_value(
    config: &Value,
    assets: &HashMap<String, WidgetAsset>,
) -> Option<WidgetAccessory> {
    let accessory = config.get("surfaces")?.get("activity_accessory")?;
    let fields = accessory
        .get("components")
        .and_then(|components| components.get("stat"))
        .and_then(|stat| stat.get("fields"));
    Some(WidgetAccessory {
        layout: accessory
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        label: fields
            .and_then(|fields| fields.get("label"))
            .and_then(widget_text),
        text: fields
            .and_then(|fields| fields.get("text"))
            .and_then(widget_text),
        icon: fields
            .and_then(|fields| fields.get("icon"))
            .and_then(|icon| widget_image(icon, assets)),
    })
}

fn mini_profile_from_value(
    config: &Value,
    assets: &HashMap<String, WidgetAsset>,
) -> Option<MiniProfile> {
    let mini = config.get("surfaces")?.get("mini_profile")?;
    let components = mini.get("components");
    let fields = components
        .and_then(|components| components.get("stat"))
        .and_then(|stat| stat.get("fields"));
    let image = |name: &str| {
        components
            .and_then(|components| components.get(name))
            .and_then(|component| component.get("fields"))
            .and_then(|fields| fields.get("image"))
            .and_then(|image| widget_image(image, assets))
    };
    let image = image("contained_image").or_else(|| image("hero_image"));
    Some(MiniProfile {
        layout: mini
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        text: fields
            .and_then(|fields| fields.get("text"))
            .and_then(widget_text),
        label: fields
            .and_then(|fields| fields.get("label"))
            .and_then(widget_text),
        icon: fields
            .and_then(|fields| fields.get("icon"))
            .and_then(|icon| widget_image(icon, assets)),
        image,
    })
}

fn widget_image(field: &Value, assets: &HashMap<String, WidgetAsset>) -> Option<WidgetImage> {
    let value = field.get("value")?.as_str()?;
    match field.get("value_type")?.as_str()? {
        "application_asset" => assets.get(value).cloned().map(WidgetImage::Asset),
        "data" if field.get("presentation_type").and_then(Value::as_str) == Some("image") => {
            // Only data fields carry a fallback; an unresolvable one is simply dropped.
            let fallback = field
                .get("fallback")
                .and_then(|fallback| widget_image(fallback, assets))
                .map(Box::new);
            Some(WidgetImage::Data {
                key: value.to_owned(),
                fallback,
            })
        }
        _ => None,
    }
}

fn widget_text(field: &Value) -> Option<WidgetText> {
    let value = field.get("value")?.as_str()?;
    match field.get("value_type")?.as_str()? {
        "data" => Some(WidgetText::Data(value.to_owned())),
        "custom_string" => Some(WidgetText::Literal(value.to_owned())),
        _ => None,
    }
}

fn widget_asset(field: &Value, assets: &HashMap<String, WidgetAsset>) -> Option<WidgetAsset> {
    if field.get("value_type").and_then(Value::as_str) != Some("application_asset") {
        return None;
    }
    let key = field.get("value").and_then(Value::as_str)?;
    assets.get(key).cloned()
}

fn resolved_assets(config: &Value) -> HashMap<String, WidgetAsset> {
    config
        .get("resolved_assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let key = asset.get("key").and_then(Value::as_str)?.to_owned();
            let asset_id = asset.get("asset_id").and_then(id_value)?;
            let metadata = asset.get("metadata");
            let width = metadata
                .and_then(|metadata| metadata.get("width"))
                .and_then(Value::as_u64)
                .and_then(|width| u32::try_from(width).ok())
                .unwrap_or_default();
            let height = metadata
                .and_then(|metadata| metadata.get("height"))
                .and_then(Value::as_u64)
                .and_then(|height| u32::try_from(height).ok())
                .unwrap_or_default();
            let animated = metadata
                .and_then(|metadata| metadata.get("is_animated"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some((
                key,
                WidgetAsset {
                    asset_id,
                    width,
                    height,
                    animated,
                },
            ))
        })
        .collect()
}
