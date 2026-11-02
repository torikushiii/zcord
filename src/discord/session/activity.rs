use super::super::*;
use serde_json::{Map, Value};
use std::{collections::HashMap, future::Future, time::Duration};

const IMAGE_RETRY_DELAY: Duration = Duration::from_secs(2);

impl Session {
    pub fn set_local_activity(&self, activity: Option<Value>) {
        let changed = self
            .inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_local_activity(activity);
        if changed {
            if let Err(error) = self.refresh_presence() {
                if !matches!(error, Error::Closed) {
                    log::warn!("Could not refresh local game activity: {error}");
                }
            }
        }
    }

    pub fn shows_current_game(&self) -> bool {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shows_current_game()
    }

    pub async fn resolve_activity_images(&self, mut activity: Value) -> Value {
        let Some(assets) = activity.get("assets").and_then(Value::as_object) else {
            return activity;
        };
        let large = assets
            .get("large_image")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let small = assets
            .get("small_image")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if large.is_none() && small.is_none() {
            return activity;
        }
        let application_id = match activity.get("application_id") {
            Some(Value::String(id)) => id.parse::<u64>().ok(),
            Some(Value::Number(id)) => id.as_u64(),
            _ => None,
        };
        let urls: Vec<String> = [&large, &small]
            .into_iter()
            .flatten()
            .filter(|value| web_url(value))
            .cloned()
            .collect();
        let external = match application_id.filter(|_| !urls.is_empty()) {
            Some(id) => {
                retry(|| self.rest(async |rest| rest.external_assets(id, &urls).await)).await
            }
            None => None,
        };
        let names: Vec<String> = [&large, &small]
            .into_iter()
            .flatten()
            .filter(|value| !already_resolved(value) && !web_url(value))
            .cloned()
            .collect();
        let applications = match application_id.filter(|_| !names.is_empty()) {
            Some(id) => retry(|| self.rest(async |rest| rest.application_assets(id).await)).await,
            None => None,
        };
        let external: HashMap<String, String> = match external.as_ref().and_then(Value::as_array) {
            Some(entries) => entries
                .iter()
                .filter_map(|entry| {
                    let url = entry.get("url")?.as_str()?;
                    let path = entry.get("external_asset_path")?.as_str()?;
                    Some((url.to_owned(), format!("mp:{path}")))
                })
                .collect(),
            None => HashMap::new(),
        };
        let applications: HashMap<String, String> =
            match applications.as_ref().and_then(Value::as_array) {
                Some(entries) => entries
                    .iter()
                    .filter_map(|entry| {
                        let name = entry.get("name")?.as_str()?;
                        let id = match entry.get("id")? {
                            Value::String(id) => id.clone(),
                            Value::Number(id) => id.as_u64()?.to_string(),
                            _ => return None,
                        };
                        Some((name.to_owned(), id))
                    })
                    .collect(),
                None => HashMap::new(),
            };
        let large_present = large.is_some();
        let small_present = small.is_some();
        let large = resolve_image(large, &external, &applications);
        let small = resolve_image(small, &external, &applications);
        if let Some(assets) = activity.get_mut("assets").and_then(Value::as_object_mut) {
            if large_present {
                update_image(assets, "large_image", "large_text", large);
            }
            if small_present {
                update_image(assets, "small_image", "small_text", small);
            }
        }
        activity
    }
}

async fn retry<T, F, Fut>(request: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    retry_after(IMAGE_RETRY_DELAY, request).await
}

async fn retry_after<T, F, Fut>(delay: Duration, mut request: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    for attempt in 0..3 {
        match request().await {
            Ok(value) => return Some(value),
            Err(_) if attempt < 2 => tokio::time::sleep(delay).await,
            Err(_) => return None,
        }
    }
    None
}

fn already_resolved(value: &str) -> bool {
    value.starts_with("mp:")
        || (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn web_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn resolve_image(
    value: Option<String>,
    external: &HashMap<String, String>,
    applications: &HashMap<String, String>,
) -> Option<String> {
    let value = value?;
    if already_resolved(&value) {
        return Some(value);
    }
    if web_url(&value) {
        return external.get(&value).cloned();
    }
    applications.get(&value).cloned()
}

fn update_image(assets: &mut Map<String, Value>, image: &str, text: &str, value: Option<String>) {
    if let Some(value) = value {
        assets.insert(image.into(), Value::String(value));
    } else {
        assets.remove(image);
        assets.remove(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps() -> (HashMap<String, String>, HashMap<String, String>) {
        (
            HashMap::from([(
                "https://example.com/a.png".to_owned(),
                "mp:external/a".to_owned(),
            )]),
            HashMap::from([("cover".to_owned(), "42".to_owned())]),
        )
    }

    #[test]
    fn images_keep_resolved_values_and_resolve_urls_and_names() {
        let (external, applications) = maps();
        assert_eq!(
            resolve_image(Some("mp:external/a".into()), &external, &applications),
            Some("mp:external/a".into())
        );
        assert_eq!(
            resolve_image(Some("12345".into()), &external, &applications),
            Some("12345".into())
        );
        assert_eq!(
            resolve_image(
                Some("https://example.com/a.png".into()),
                &external,
                &applications
            ),
            Some("mp:external/a".into())
        );
        assert_eq!(
            resolve_image(Some("cover".into()), &external, &applications),
            Some("42".into())
        );
        assert_eq!(
            resolve_image(Some("missing".into()), &external, &applications),
            None
        );
    }

    #[test]
    fn unresolved_images_remove_their_matching_text() {
        let mut assets = Map::new();
        assets.insert("large_image".into(), Value::String("missing".into()));
        assets.insert("large_text".into(), Value::String("Large".into()));
        assets.insert("small_image".into(), Value::String("keep".into()));
        update_image(&mut assets, "large_image", "large_text", None);
        assert!(assets.get("large_image").is_none());
        assert!(assets.get("large_text").is_none());
        assert!(assets.get("small_image").is_some());
    }

    #[tokio::test]
    async fn image_lookups_retry_request_errors_but_not_empty_results() {
        let mut failures = 0;
        let failed = retry_after(Duration::ZERO, || {
            failures += 1;
            async { Err::<(), Error>(Error::Network) }
        })
        .await;
        assert!(failed.is_none());
        assert_eq!(failures, 3);

        let mut successes = 0;
        let empty = retry_after(Duration::ZERO, || {
            successes += 1;
            async { Ok::<_, Error>(Value::Array(Vec::new())) }
        })
        .await;
        assert_eq!(empty, Some(Value::Array(Vec::new())));
        assert_eq!(successes, 1);
    }
}
