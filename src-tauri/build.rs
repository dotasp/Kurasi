//! Build script: stages the web interface for bundling, then runs tauri-build.
//!
//! `ui/dev/` holds screenshots and development files (about 17 MB) that must stay out of the
//! executable and the installer. Tauri embeds `frontendDist` whole, so this script copies
//! `../ui` without `dev/` into `gen/ui/` (gitignored) and `tauri.conf.json` points there.

use std::fs;
use std::path::{Path, PathBuf};

fn copy_tree(src: &Path, dst: &Path, skip: &[&str]) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if skip.iter().any(|s| *s == name_str) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to, &[])?;
        } else {
            let same = fs::metadata(&to)
                .and_then(|m| Ok((m.len(), m.modified()?)))
                .ok()
                .zip(fs::metadata(&from).and_then(|m| Ok((m.len(), m.modified()?))).ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false);
            if !same {
                fs::copy(&from, &to)?;
            }
        }
    }
    Ok(())
}

fn remove_stale(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dst)? {
        let entry = entry?;
        let counterpart = src.join(entry.file_name());
        if !counterpart.exists() {
            if entry.path().is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        } else if entry.path().is_dir() {
            remove_stale(&counterpart, &entry.path())?;
        }
    }
    Ok(())
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ui_src = manifest.join("..").join("ui");
    let ui_dst = manifest.join("gen").join("ui");
    println!("cargo:rerun-if-changed={}", ui_src.display());
    if ui_src.is_dir() {
        copy_tree(&ui_src, &ui_dst, &["dev"]).expect("stage ui/ into gen/ui/");
        remove_stale(&ui_src, &ui_dst).expect("remove stale files from gen/ui/");
        if ui_dst.join("dev").exists() {
            fs::remove_dir_all(ui_dst.join("dev")).expect("remove gen/ui/dev");
        }
    }
    tauri_build::build()
}
