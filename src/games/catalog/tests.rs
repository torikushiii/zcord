use super::*;
use serde_json::json;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn host_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

fn catalog(entries: Value) -> Catalog {
    decode(entries.to_string().as_bytes()).expect("synthetic catalog")
}

#[test]
fn keeps_this_platforms_executables_and_skips_bad_entries() {
    let wrong_os = if host_os() == "darwin" {
        "linux"
    } else {
        "darwin"
    };
    let catalog = catalog(json!([
        {"id": "1", "name": "Kept", "executables": [
            {"name": "Kept.EXE", "os": host_os()},
            {"name": "launcher.exe", "os": host_os(), "is_launcher": true},
            {"name": "other.exe", "os": "unknown"},
            {"name": "wrong.exe", "os": wrong_os},
            {"name": "no-os.exe"}
        ]},
        {"id": "2", "name": "No executables"},
        {"id": "bad", "name": "Bad id", "executables": [{"name": "bad.exe", "os": host_os()}]},
        {"name": "No id", "executables": [{"name": "none.exe", "os": host_os()}]},
        {"id": "5", "name": "Bad arguments", "executables": [
            {"name": "badargs.exe", "os": host_os(), "arguments": 42}
        ]}
    ]));
    assert!(catalog.matches("/tmp/kept.exe", String::new).is_some());
    assert!(catalog.matches("/tmp/launcher.exe", String::new).is_none());
    assert!(catalog.matches("/tmp/other.exe", String::new).is_none());
    assert!(catalog.matches("/tmp/bad.exe", String::new).is_none());
    assert!(catalog.matches("/tmp/none.exe", String::new).is_none());
    assert!(catalog.matches("/tmp/badargs.exe", String::new).is_none());
    assert_eq!(catalog.name(1).as_deref(), Some("Kept"));
    assert_eq!(catalog.name(2), None);
}

#[cfg(target_os = "linux")]
#[test]
fn wine_executables_are_kept_on_linux() {
    let catalog = catalog(json!([
        {"id": "1", "name": "Windows game", "executables": [
            {"name": "game.exe", "os": "win32"}
        ]}
    ]));
    assert!(catalog.matches("/steam/game.exe", String::new).is_some());
}

#[test]
fn names_are_normalized_and_exact_matches_keep_their_flag() {
    assert_eq!(
        split_rule("Game\\Bin\\Game.EXE"),
        Some(("game/bin/game.exe".into(), false))
    );
    assert_eq!(split_rule(">Game.EXE"), Some(("game.exe".into(), true)));
    assert_eq!(split_rule(">>Game.EXE"), Some((">game.exe".into(), true)));
    assert_eq!(split_rule(">"), None);
    assert_eq!(split_rule("dir/"), None);
}

#[test]
fn matches_executables_by_path_suffix() {
    let catalog = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": "Game/Bin/Game.exe", "os": host_os()},
            {"name": "bare.exe", "os": host_os()}
        ]}
    ]));
    assert!(
        catalog
            .matches("/opt/Game/Bin/Game.exe", String::new)
            .is_some()
    );
    assert!(catalog.matches("Game/Bin/Game.exe", String::new).is_some());
    assert!(
        catalog
            .matches("/opt/NotGame/Bin/Game.exe", String::new)
            .is_none()
    );
    assert!(
        catalog
            .matches("C:\\Opt\\Game\\Bin\\Game.exe", String::new)
            .is_some()
    );
    assert!(catalog.matches("/opt/bare.exe", String::new).is_some());
    assert!(catalog.matches("/opt/notbare.exe", String::new).is_none());
}

#[test]
fn exact_names_compare_against_the_file_name_only() {
    let exact = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": ">dir/game.exe", "os": host_os()}
        ]}
    ]));
    assert!(exact.matches("/opt/dir/game.exe", String::new).is_none());
    let suffix = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": "dir/game.exe", "os": host_os()}
        ]}
    ]));
    assert!(suffix.matches("/opt/dir/game.exe", String::new).is_some());
    let bare = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": ">game.exe", "os": host_os()}
        ]}
    ]));
    assert!(bare.matches("/opt/game.exe", String::new).is_some());
}

#[test]
fn arguments_are_required_and_case_insensitive() {
    let catalog = catalog(json!([
        {"id": "1", "name": "Garry's Mod", "executables": [
            {"name": ">hl2.exe", "os": host_os(), "arguments": "-game garrysmod"}
        ]},
        {"id": "2", "name": "Half-Life 2", "executables": [
            {"name": ">hl2.exe", "os": host_os(), "arguments": "-game hl2"}
        ]},
        {"id": "3", "name": "Bare", "executables": [
            {"name": "bare.exe", "os": host_os()}
        ]}
    ]));
    let game = catalog
        .matches("/games/hl2.exe", || {
            "HL2.EXE -Game GARRYSMOD -windowed".into()
        })
        .expect("arguments match");
    assert_eq!(game.id, 1);
    assert!(
        catalog
            .matches("/games/hl2.exe", || "-game".into())
            .is_none()
    );
    assert!(
        catalog
            .matches("/games/bare.exe", || panic!("command line was read"))
            .is_some()
    );
}

#[test]
fn cached_catalogs_restore_their_rules() {
    let catalog = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": ">game.exe", "os": host_os(), "arguments": "-play"},
            {"name": ">dir/exact.exe", "os": host_os()}
        ]}
    ]));
    let bytes = serde_json::to_vec(&catalog.cached()).expect("cache serializes");
    let restored = decode_cached(&bytes).expect("cache decodes");
    assert!(
        restored
            .matches("/opt/game.exe", || "-play".into())
            .is_some()
    );
    assert!(restored.matches("/opt/game.exe", String::new).is_none());
    assert!(restored.matches("/opt/exact.exe", String::new).is_none());
    assert!(
        restored
            .matches("/opt/dir/exact.exe", String::new)
            .is_none()
    );
}

#[test]
fn cache_files_round_trip() {
    let directory = std::env::temp_dir().join(format!(
        "zcord-catalog-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let path = directory.join("detectable-games.json");
    let catalog = catalog(json!([
        {"id": "1", "name": "Game", "executables": [
            {"name": "game.exe", "os": host_os()}
        ]}
    ]));
    write_cache(&path, &catalog).expect("cache writes");
    let (restored, age) = read_cache(&path).expect("cache reads");
    assert!(age < MAX_AGE);
    assert!(restored.matches("/opt/game.exe", String::new).is_some());
    let _ = fs::remove_dir_all(&directory);
}
