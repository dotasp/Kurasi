mod common;

use common::*;
use kurasi_core::config::{choose_last, AppPaths, ProjectInfo};

fn entry(pid: &str, folder: &str, last: &str, demo: bool) -> ProjectInfo {
    ProjectInfo {
        id: pid.into(),
        name: pid.to_uppercase(),
        folder: folder.into(),
        created: Some("2026-01-01T00:00:00".into()),
        last_opened: Some(last.into()),
        demo,
        read_only: None,
    }
}

#[test]
fn existing_folder_wins_over_newer_missing_one() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("old");
    std::fs::create_dir_all(&old).unwrap();
    let entries = vec![
        entry("p-new", &tmp.path().join("gone").to_string_lossy(), "2026-10-17T09:00:00", false),
        entry("p-old", &old.to_string_lossy(), "2026-10-10T09:00:00", false),
        entry("p-older", &old.to_string_lossy(), "2026-09-01T09:00:00", false),
    ];
    let (chosen, missing) = choose_last(&entries, |f| std::path::Path::new(f).is_dir());
    assert_eq!(chosen.unwrap().id, "p-old");
    assert!(missing.is_none());
}

#[test]
fn missing_folder_is_flagged_when_nothing_opens() {
    let tmp = tempfile::tempdir().unwrap();
    let entries = vec![
        entry("p-a", &tmp.path().join("gone-a").to_string_lossy(), "2026-10-17T09:00:00", false),
        entry("p-b", &tmp.path().join("gone-b").to_string_lossy(), "2026-10-01T09:00:00", false),
    ];
    let (chosen, missing) = choose_last(&entries, |f| std::path::Path::new(f).is_dir());
    assert!(chosen.is_none());
    assert_eq!(missing.unwrap().id, "p-a");
    let (c, m) = choose_last(&[], |_| true);
    assert!(c.is_none() && m.is_none());
}

#[test]
fn demo_is_never_reopened_at_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let entries = vec![entry("demo", &tmp.path().to_string_lossy(), "2026-10-17T09:00:00", true)];
    let (c, m) = choose_last(&entries, |_| true);
    assert!(c.is_none() && m.is_none());
}

#[test]
fn register_is_idempotent_by_folder_and_last_opened_updates() {
    let h = home();
    let folder = h.dir.path().join("proj");
    std::fs::create_dir_all(&folder).unwrap();
    let a = h.paths.register(Some("A"), &folder, None, false).unwrap();
    let with_sep = format!("{}\\", folder.to_string_lossy());
    let b = h.paths.register(Some("B"), std::path::Path::new(&with_sep), None, false).unwrap();
    assert_eq!(a.id, b.id);
    let all = h.paths.projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "B");
    assert!(a.id.starts_with("p-") && a.id.len() == 14);
    h.paths.set_last_opened(&a.id).unwrap();
    assert_eq!(h.paths.last_opened().as_deref(), Some(a.id.as_str()));
    assert_eq!(h.paths.resolve(&folder.to_string_lossy()).unwrap().id, a.id);
    assert!(h.paths.resolve("p-nothing").is_none());
    let gone = h.paths.register(Some("Gone"), &h.dir.path().join("gone-later"), None, false).unwrap();
    h.paths.set_last_opened(&gone.id).unwrap();
    // The newest entry has no folder, so the one before it is chosen.
    assert_eq!(h.paths.last_opened().as_deref(), Some(a.id.as_str()));
    h.paths.unregister(&gone.id, false).unwrap();
    assert_eq!(h.paths.projects().unwrap().len(), 1);
}

#[test]
fn detect_honours_kurasi_home() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("home-from-env");
    std::env::set_var("KURASI_HOME", &target);
    let paths = AppPaths::detect();
    std::env::remove_var("KURASI_HOME");
    assert_eq!(paths.root, target);
    assert!(target.is_dir());
    assert!(paths.demo_dir().ends_with("demo"));
}

#[test]
fn reopen_while_another_thread_holds_a_clone() {
    let h = home();
    let folder = folder(h.dir.path());
    let first = kurasi_core::Project::create(&h.paths, "Shared", &folder).unwrap();
    let id = first.id().to_string();
    let held = first.clone();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        // The server's old handle keeps working after the main thread swapped projects.
        rx.recv().unwrap();
        held.set_setting("theme", serde_json::json!("dark")).unwrap();
        held.name()
    });
    drop(first);
    let second = kurasi_core::Project::open(&h.paths, &id).unwrap();
    second.set_setting("name", serde_json::json!("Renamed")).unwrap();
    tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), "Renamed");
    assert_eq!(second.setting("theme"), serde_json::json!("dark"));
    assert_eq!(second.change_seq(), 2);
}

#[test]
fn project_open_creates_and_reopens() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = kurasi_core::Project::create(&h.paths, "Test", &folder).unwrap();
    let info = p.info();
    assert_eq!(info.name, "Test");
    assert_eq!(info.read_only, Some(false));
    assert!(h.paths.project_dir(&info.id).join("kurasi.sqlite").exists());
    let again = kurasi_core::Project::open(&h.paths, &info.id).unwrap();
    assert_eq!(again.name(), "Test");
    let by_folder = kurasi_core::Project::open(&h.paths, &folder.to_string_lossy()).unwrap();
    assert_eq!(by_folder.id(), info.id);
    assert!(kurasi_core::Project::open(&h.paths, "p-missing").is_err());
    assert!(kurasi_core::Project::create(&h.paths, "Demo", &h.paths.demo_dir()).is_err());
}
