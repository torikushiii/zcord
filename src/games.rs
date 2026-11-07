mod catalog;
pub mod programs;
pub mod registry;
mod rpc;
mod scan;

use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinHandle};

const INTERVAL: Duration = Duration::from_secs(15);

/// What to publish as the user's activity, and the game detected running, if any. An app
/// setting activity over RPC isn't a running game, so Discord's account panel leaves it out.
#[derive(Clone, PartialEq)]
pub struct GameActivity {
    pub activity: Value,
    pub running: Option<RunningGame>,
}

#[derive(Clone, PartialEq)]
pub struct RunningGame {
    pub name: String,
    pub application_id: Option<u64>,
}

pub fn start(enabled: watch::Receiver<bool>) -> watch::Receiver<Option<GameActivity>> {
    let (output, receiver) = watch::channel(None);
    tokio::spawn(supervise(enabled, output));
    receiver
}

async fn supervise(
    mut enabled: watch::Receiver<bool>,
    output: watch::Sender<Option<GameActivity>>,
) {
    loop {
        if !*enabled.borrow() {
            output.send_replace(None);
            match enabled.changed().await {
                Ok(()) => continue,
                Err(_) => return,
            }
        }
        if !*enabled.borrow() {
            continue;
        }
        let mut tasks = Tasks::start();
        let mut throttle = Throttle::new();
        let mut stopped = false;
        loop {
            if !*enabled.borrow() {
                break;
            }
            let rpc = tasks.rpc.borrow().clone();
            let detected = tasks.detected.borrow().clone();
            let catalog = tasks.catalog.borrow().clone();
            let desired =
                compose(rpc.as_ref(), detected.as_ref(), catalog.as_deref()).map(|activity| {
                    GameActivity {
                        activity,
                        running: detected.as_ref().map(|game| RunningGame {
                            name: game.name.clone(),
                            application_id: game.application_id,
                        }),
                    }
                });
            if throttle.offer(desired, Instant::now()) {
                output.send_replace(throttle.shown().cloned());
            }
            let delay = throttle.delay(Instant::now());
            tokio::select! {
                result = enabled.changed() => {
                    if result.is_err() {
                        stopped = true;
                        break;
                    }
                    if !*enabled.borrow() {
                        break;
                    }
                }
                _ = tasks.catalog.changed() => {}
                _ = tasks.detected.changed() => {}
                _ = tasks.rpc.changed() => {}
                () = wait(delay) => {
                    if !*enabled.borrow() {
                        break;
                    }
                    if throttle.flush(Instant::now()) {
                        output.send_replace(throttle.shown().cloned());
                    }
                }
            }
        }
        tasks.stop().await;
        registry::flush();
        output.send_replace(None);
        if stopped {
            return;
        }
    }
}

struct Tasks {
    catalog: watch::Receiver<Option<Arc<catalog::Catalog>>>,
    detected: watch::Receiver<Option<scan::Detected>>,
    rpc: watch::Receiver<Option<Value>>,
    catalog_sender: watch::Sender<Option<Arc<catalog::Catalog>>>,
    detected_sender: watch::Sender<Option<scan::Detected>>,
    rpc_sender: watch::Sender<Option<Value>>,
    catalog_task: JoinHandle<()>,
    scan_task: JoinHandle<()>,
    rpc_task: JoinHandle<()>,
}

impl Tasks {
    fn start() -> Self {
        let (catalog_sender, catalog) = watch::channel::<Option<Arc<catalog::Catalog>>>(None);
        let (detected_sender, detected) = watch::channel::<Option<scan::Detected>>(None);
        let (rpc_sender, rpc) = watch::channel::<Option<Value>>(None);
        let catalog_task = tokio::spawn(catalog::run(catalog_sender.clone()));
        let scan_task = tokio::spawn(scan::run(catalog.clone(), detected_sender.clone()));
        let rpc_task = tokio::spawn(rpc::run(rpc_sender.clone()));
        Self {
            catalog,
            detected,
            rpc,
            catalog_sender,
            detected_sender,
            rpc_sender,
            catalog_task,
            scan_task,
            rpc_task,
        }
    }

    async fn stop(self) {
        let Self {
            catalog_sender,
            detected_sender,
            rpc_sender,
            catalog_task,
            scan_task,
            rpc_task,
            ..
        } = self;
        drop((catalog_sender, detected_sender, rpc_sender));
        catalog_task.abort();
        scan_task.abort();
        rpc_task.abort();
        let _ = catalog_task.await;
        let _ = scan_task.await;
        let _ = rpc_task.await;
    }
}

fn compose(
    rpc: Option<&Value>,
    detected: Option<&scan::Detected>,
    catalog: Option<&catalog::Catalog>,
) -> Option<Value> {
    match rpc {
        Some(activity) => Some(with_name(activity.clone(), detected, catalog)),
        None => detected.map(|game| {
            let mut activity = json!({
                "type": 0,
                "name": game.name,
                "timestamps": {"start": game.start_ms},
            });
            if let Some(id) = game.application_id {
                activity["application_id"] = Value::String(id.to_string());
            }
            activity
        }),
    }
}

fn with_name(
    mut activity: Value,
    detected: Option<&scan::Detected>,
    catalog: Option<&catalog::Catalog>,
) -> Value {
    let missing = activity
        .get("name")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    if !missing {
        return activity;
    }
    let application = activity
        .get("application_id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<u64>().ok());
    let name = application.and_then(|id| {
        detected
            .filter(|game| game.application_id == Some(id))
            .map(|game| game.name.as_str())
            .or_else(|| catalog.and_then(|catalog| catalog.name(id)))
    });
    if let Some(activity) = activity.as_object_mut() {
        activity.insert(
            "name".into(),
            Value::String(name.unwrap_or("Game").to_owned()),
        );
    }
    activity
}

struct Throttle<T> {
    last: Option<Instant>,
    shown: Option<T>,
    pending: Option<T>,
}

impl<T: PartialEq> Throttle<T> {
    fn new() -> Self {
        Self {
            last: None,
            shown: None,
            pending: None,
        }
    }

    fn shown(&self) -> Option<&T> {
        self.shown.as_ref()
    }

    fn offer(&mut self, next: Option<T>, now: Instant) -> bool {
        if self.shown == next && self.pending.is_none() {
            return false;
        }
        match next {
            None => {
                self.pending = None;
                if self.shown.is_none() {
                    return false;
                }
                self.shown = None;
                self.last = Some(now);
                true
            }
            Some(value) => {
                if self.shown.as_ref() == Some(&value) {
                    self.pending = None;
                    return false;
                }
                if self
                    .last
                    .is_none_or(|last| now.saturating_duration_since(last) >= INTERVAL)
                {
                    self.shown = Some(value);
                    self.pending = None;
                    self.last = Some(now);
                    true
                } else {
                    self.pending = Some(value);
                    false
                }
            }
        }
    }

    fn flush(&mut self, now: Instant) -> bool {
        let Some(value) = self.pending.take() else {
            return false;
        };
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= INTERVAL)
        {
            self.shown = Some(value);
            self.last = Some(now);
            true
        } else {
            self.pending = Some(value);
            false
        }
    }

    fn delay(&self, now: Instant) -> Option<Duration> {
        self.pending.as_ref()?;
        let last = self.last?;
        Some(INTERVAL.saturating_sub(now.saturating_duration_since(last)))
    }
}

async fn wait(delay: Option<Duration>) {
    match delay {
        Some(delay) => tokio::time::sleep(delay).await,
        None => std::future::pending::<()>().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected() -> scan::Detected {
        scan::Detected {
            application_id: Some(42),
            name: "Detected".into(),
            start_ms: 1_000,
        }
    }

    #[test]
    fn rpc_activity_takes_priority_and_missing_names_are_filled() {
        let detected = detected();
        let named = json!({"type": 0, "name": "RPC", "application_id": "42"});
        assert_eq!(
            compose(Some(&named), Some(&detected), None).unwrap()["name"],
            "RPC"
        );
        let unnamed = json!({"type": 0, "name": "", "application_id": "42"});
        let composed = compose(Some(&unnamed), Some(&detected), None).unwrap();
        assert_eq!(composed["name"], "Detected");
        assert_eq!(composed["application_id"], "42");
        let composed = compose(Some(&unnamed), None, None).unwrap();
        assert_eq!(composed["name"], "Game");
    }

    #[test]
    fn detected_games_use_the_published_shape() {
        let detected = detected();
        assert_eq!(
            compose(None, Some(&detected), None).unwrap(),
            json!({
                "type": 0,
                "name": "Detected",
                "application_id": "42",
                "timestamps": {"start": 1_000}
            })
        );
        assert_eq!(compose(None, None, None), None);
        let program = scan::Detected {
            application_id: None,
            ..detected
        };
        assert_eq!(
            compose(None, Some(&program), None).unwrap(),
            json!({"type": 0, "name": "Detected", "timestamps": {"start": 1_000}})
        );
    }

    #[test]
    fn missing_names_fall_back_to_the_catalog() {
        let os = if cfg!(target_os = "windows") {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        let catalog = catalog::decode(
            format!(
                r#"[{{"id":"42","name":"Catalog","executables":[{{"name":"game.exe","os":"{os}"}}]}}]"#
            )
            .as_bytes(),
        )
        .expect("synthetic catalog");
        let unnamed = json!({"type": 0, "name": "", "application_id": "42"});
        assert_eq!(
            compose(Some(&unnamed), None, Some(&catalog)).unwrap()["name"],
            "Catalog"
        );
        let unknown = json!({"type": 0, "name": "", "application_id": "7"});
        assert_eq!(
            compose(Some(&unknown), None, Some(&catalog)).unwrap()["name"],
            "Game"
        );
    }

    #[tokio::test]
    async fn disabled_supervisors_publish_nothing_and_exit_with_the_sender() {
        let (enabled, receiver) = watch::channel(false);
        let mut output = start(receiver);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(output.borrow().is_none());
        drop(enabled);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if output.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .expect("supervisor exits");
    }

    #[test]
    fn throttle_sends_one_change_per_interval_and_keeps_the_latest() {
        let base = Instant::now();
        let mut throttle = Throttle::new();
        assert!(throttle.offer(Some(json!({"name": "A"})), base));
        assert!(!throttle.offer(Some(json!({"name": "B"})), base + Duration::from_secs(1)));
        assert!(!throttle.offer(Some(json!({"name": "C"})), base + Duration::from_secs(2)));
        assert_eq!(
            throttle.delay(base + Duration::from_secs(2)),
            Some(Duration::from_secs(13))
        );
        assert!(!throttle.flush(base + Duration::from_secs(14)));
        assert!(throttle.flush(base + Duration::from_secs(15)));
        assert_eq!(throttle.shown(), Some(&json!({"name": "C"})));
    }

    #[test]
    fn throttle_clears_immediately_and_skips_unchanged_values() {
        let base = Instant::now();
        let mut throttle = Throttle::new();
        let first = json!({"name": "A"});
        assert!(throttle.offer(Some(first.clone()), base));
        assert!(!throttle.offer(Some(first), base + Duration::from_secs(20)));
        assert!(throttle.offer(None, base + Duration::from_secs(21)));
        assert_eq!(throttle.shown(), None);
        assert!(!throttle.offer(None, base + Duration::from_secs(22)));
        assert!(!throttle.offer(Some(json!({"name": "B"})), base + Duration::from_secs(22)));
        assert!(throttle.flush(base + Duration::from_secs(36)));
        assert_eq!(throttle.shown(), Some(&json!({"name": "B"})));
    }
}
