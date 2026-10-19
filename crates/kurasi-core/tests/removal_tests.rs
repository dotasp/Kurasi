//! Removing a project: the safety gate refuses every folder Kurasi did not create, user files
//! stay where they are, and a removal can be undone with its data.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use kurasi_core::config::ProjectInfo;
use kurasi_core::removal::{check_target, TargetKind};
use kurasi_core::{sidecar, AppPaths};

/// A scanned project with its sidecar, an exported AI-use log and a file of the user inside
/// `.kurasi`. The project is closed again, so its database can move.
fn prepared(h: &Home) -> (ProjectInfo, PathBuf) {
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    sidecar::write(&p).unwrap();
    p.export_ai_log().unwrap();
    std::fs::write(folder.join(".kurasi").join("my-notes.txt"), "written by the user").unwrap();
    let entry = h.paths.resolve(p.id()).unwrap();
    drop(p);
    (entry, folder)
}

/// Every file under `dir` with its bytes, relative paths sorted.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn user_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    snapshot(dir)
        .into_iter()
        .filter(|(rel, _)| !rel.starts_with(".kurasi\\metadata.json") && !rel.starts_with(".kurasi\\AI_USE_LOG.md"))
        .collect()
}

fn refused(paths: &AppPaths, entry: &ProjectInfo, target: &Path) {
    let err = check_target(paths, entry, target).expect_err(&format!("{} must be refused", target.display()));
    assert_eq!(err.status(), 409, "{}", err.message());
    assert!(err.message().starts_with("Kurasi refuses to delete"), "{}", err.message());
}

#[test]
fn safety_gate_refuses_everything_kurasi_did_not_create() {
    let h = home();
    let (entry, folder) = prepared(&h);
    let app = h.paths.root.join("projects").join(&entry.id);
    let side = folder.join(".kurasi");
    assert_eq!(check_target(&h.paths, &entry, &app).unwrap(), TargetKind::AppData);
    assert_eq!(check_target(&h.paths, &entry, &side).unwrap(), TargetKind::Sidecar);

    // The project root, a parent folder and a drive root.
    refused(&h.paths, &entry, &folder);
    refused(&h.paths, &entry, folder.parent().unwrap());
    refused(&h.paths, &entry, folder.ancestors().last().unwrap());
    refused(&h.paths, &entry, Path::new("C:\\"));
    refused(&h.paths, &entry, Path::new("C:\\Users"));
    // Paths outside Kurasi's data for this project.
    let outside = h.dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::copy(app.join("kurasi.sqlite"), outside.join("kurasi.sqlite")).unwrap();
    refused(&h.paths, &entry, &outside);
    refused(&h.paths, &entry, &folder.join("lib"));
    refused(&h.paths, &entry, &folder.join("lib").join(".kurasi"));
    refused(&h.paths, &entry, &h.paths.root);
    refused(&h.paths, &entry, &h.paths.root.join("projects"));
    refused(&h.paths, &entry, &h.paths.root.join("thumbs"));
    refused(&h.paths, &entry, &app.join("..").join(&entry.id).join(".."));
    // Another project's data folder does not pass for this entry.
    let mut other = entry.clone();
    other.id = "p-0123456789ab".into();
    refused(&h.paths, &other, &app);
    // The sample project is never a target.
    let mut demo = entry.clone();
    demo.demo = true;
    refused(&h.paths, &demo, &app);

    // Markers: an app folder with a file Kurasi did not create, a database without the Kurasi
    // tables, a .kurasi folder without a valid metadata.json.
    std::fs::write(app.join("thesis.docx"), "user file").unwrap();
    refused(&h.paths, &entry, &app);
    std::fs::remove_file(app.join("thesis.docx")).unwrap();
    std::fs::rename(app.join("kurasi.sqlite"), h.dir.path().join("kept.sqlite")).unwrap();
    std::fs::write(app.join("kurasi.sqlite"), "SQLite format 3\0 but not really").unwrap();
    refused(&h.paths, &entry, &app);
    std::fs::remove_file(app.join("kurasi.sqlite")).unwrap();
    std::fs::rename(h.dir.path().join("kept.sqlite"), app.join("kurasi.sqlite")).unwrap();
    assert_eq!(check_target(&h.paths, &entry, &app).unwrap(), TargetKind::AppData);
    std::fs::rename(side.join("metadata.json"), h.dir.path().join("kept.json")).unwrap();
    refused(&h.paths, &entry, &side);
    std::fs::write(side.join("metadata.json"), "{\"references\": []}").unwrap();
    refused(&h.paths, &entry, &side);
}

#[test]
fn remove_with_data_keeps_user_files_and_undo_restores_everything() {
    let h = home();
    let (entry, folder) = prepared(&h);
    let app = h.paths.root.join("projects").join(&entry.id);
    let targets = h.paths.data_targets(&entry);
    let before_user = user_files(&folder);
    let before_app = snapshot(&app);
    let before_side = snapshot(&folder.join(".kurasi"));
    assert!(before_user.iter().any(|(rel, _)| rel == ".kurasi\\my-notes.txt"));

    assert_eq!(targets.app.as_ref().unwrap().path, app.to_string_lossy());
    let side = targets.sidecar.as_ref().unwrap();
    assert_eq!(side.files, vec!["AI_USE_LOG.md".to_string(), "metadata.json".to_string()]);
    assert!(!side.whole, "my-notes.txt keeps .kurasi in place");

    let removal = h.paths.remove_project(&entry.id, true).unwrap();
    assert!(h.paths.resolve(&entry.id).is_none());
    assert!(!app.exists());
    assert!(!folder.join(".kurasi").join("metadata.json").exists());
    assert!(!folder.join(".kurasi").join("AI_USE_LOG.md").exists());
    assert_eq!(user_files(&folder), before_user, "the user's files are untouched");
    assert_eq!(removal.moved.len(), 3);
    assert!(h.paths.removed_dir().join(&removal.removal_id).join("removed.json").is_file());

    let back = h.paths.restore_project(&removal.removal_id).unwrap();
    assert_eq!(back, entry);
    assert_eq!(h.paths.resolve(&entry.id).unwrap(), entry);
    assert_eq!(snapshot(&app), before_app);
    assert_eq!(snapshot(&folder.join(".kurasi")), before_side);
    assert_eq!(user_files(&folder), before_user);
    assert!(!h.paths.removed_dir().join(&removal.removal_id).exists());
    // A second undo of the same removal finds nothing.
    assert_eq!(h.paths.restore_project(&removal.removal_id).unwrap_err().status(), 404);
    assert_eq!(h.paths.restore_project("..\\..\\projects").unwrap_err().status(), 404);
}

#[test]
fn an_empty_kurasi_folder_goes_and_comes_back() {
    let h = home();
    let (entry, folder) = prepared(&h);
    std::fs::remove_file(folder.join(".kurasi").join("my-notes.txt")).unwrap();
    assert!(h.paths.data_targets(&entry).sidecar.unwrap().whole);
    let before_user = user_files(&folder);
    let removal = h.paths.remove_project(&entry.id, true).unwrap();
    assert!(!folder.join(".kurasi").exists());
    assert_eq!(user_files(&folder), before_user);
    h.paths.restore_project(&removal.removal_id).unwrap();
    assert!(folder.join(".kurasi").join("metadata.json").is_file());
}

#[test]
fn remove_without_data_leaves_every_file() {
    let h = home();
    let (entry, folder) = prepared(&h);
    let app = h.paths.root.join("projects").join(&entry.id);
    let _ = h.paths.data_targets(&entry);
    let before = snapshot(&folder);
    let before_app = snapshot(&app);
    let removal = h.paths.remove_project(&entry.id, false).unwrap();
    assert!(removal.moved.is_empty());
    assert!(h.paths.resolve(&entry.id).is_none());
    assert_eq!(snapshot(&folder), before);
    assert_eq!(snapshot(&app), before_app);
    h.paths.restore_project(&removal.removal_id).unwrap();
    assert_eq!(h.paths.resolve(&entry.id).unwrap(), entry);
}

#[test]
fn read_only_project_keeps_its_sidecar() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    sidecar::write(&p).unwrap();
    p.set_read_only(true).unwrap();
    let entry = h.paths.resolve(p.id()).unwrap();
    drop(p);
    let targets = h.paths.data_targets(&entry);
    assert!(targets.app.is_some());
    assert!(targets.sidecar.is_none());
    let before = snapshot(&folder);
    h.paths.remove_project(&entry.id, true).unwrap();
    assert_eq!(snapshot(&folder), before);
}

#[test]
fn held_database_refuses_and_changes_nothing() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    sidecar::write(&p).unwrap();
    let entry = h.paths.resolve(p.id()).unwrap();
    // `p` keeps the database open, as a running MCP server would.
    let err = h.paths.remove_project(&entry.id, true).unwrap_err();
    assert_eq!(err.status(), 409, "{}", err.message());
    assert!(err.message().contains("in use by another program"), "{}", err.message());
    assert_eq!(h.paths.resolve(&entry.id).unwrap(), entry);
    assert!(h.paths.root.join("projects").join(&entry.id).join("kurasi.sqlite").is_file());
    assert!(folder.join(".kurasi").join("metadata.json").is_file());
    assert!(std::fs::read_dir(h.paths.removed_dir()).map(|d| d.count()).unwrap_or(0) == 0);
    drop(p);
}

#[test]
fn sample_project_cannot_be_removed() {
    let h = home();
    let info = kurasi_core::demo::build(&h.paths).unwrap();
    let err = h.paths.remove_project(&info.id, true).unwrap_err();
    assert_eq!(err.status(), 400);
    assert!(h.paths.resolve(&info.id).is_some());
    assert!(h.paths.data_targets(&info).is_empty());
    assert_eq!(h.paths.remove_project("p-nothere00000", false).unwrap_err().status(), 404);
}

#[test]
fn purge_deletes_old_removals_only() {
    let h = home();
    let (entry, _folder) = prepared(&h);
    let removal = h.paths.remove_project(&entry.id, true).unwrap();
    let trash = h.paths.removed_dir().join(&removal.removal_id);
    // A folder in the trash without a manifest is left alone.
    let stray = h.paths.removed_dir().join("rm-000000000000");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("keep.txt"), "x").unwrap();
    assert_eq!(h.paths.purge_removed(Duration::from_secs(3600)).unwrap(), 0);
    assert!(trash.is_dir());
    assert_eq!(h.paths.purge_removed(Duration::ZERO).unwrap(), 1);
    assert!(!trash.exists());
    assert!(stray.join("keep.txt").is_file());
    assert_eq!(h.paths.restore_project(&removal.removal_id).unwrap_err().status(), 404);
}
