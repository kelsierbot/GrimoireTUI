//! Grimoire on a phone: a thin shell around `grimoire-app`.
//!
//! Every command here is one screen's worth of question and nothing more, so a
//! phone that gets backgrounded mid-sentence never has to reconstruct a session.
//! The interesting behaviour — what a book is, how a scene is counted, what
//! happens when two devices save the same scene — all lives in the crates that
//! the terminal app uses too. This file only translates.
//!
//! Where the books live: on Android an app owns one directory that survives
//! updates and needs no permissions, so that is the shelf. Nothing here assumes
//! it can reach the rest of the filesystem, which keeps the door open for the
//! sync we choose later without changing any of these commands.

use grimoire_app::{Book, Outline, Saved, Scene, Stamp};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Errors cross to the front end as plain sentences a writer could read.
type Reply<T> = Result<T, String>;

fn plainly<T>(r: anyhow::Result<T>) -> Reply<T> {
    r.map_err(|e| e.to_string())
}

/// The shelf: `<app data>/Books`, made on first run.
fn shelf(app: &tauri::AppHandle) -> Reply<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app folder: {e}"))?
        .join("Books");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

#[tauri::command]
fn base_dir(app: tauri::AppHandle) -> Reply<String> {
    Ok(shelf(&app)?.to_string_lossy().to_string())
}

#[tauri::command]
fn books(app: tauri::AppHandle) -> Reply<Vec<Book>> {
    Ok(grimoire_app::books(&shelf(&app)?))
}

#[tauri::command]
fn create_book(app: tauri::AppHandle, name: String) -> Reply<Book> {
    plainly(grimoire_app::create_book(&shelf(&app)?, &name))
}

#[tauri::command]
fn outline(book: String) -> Reply<Outline> {
    plainly(grimoire_app::outline(Path::new(&book)))
}

#[tauri::command]
fn read_scene(path: String) -> Reply<Scene> {
    plainly(grimoire_app::read_scene(Path::new(&path)))
}

#[tauri::command]
fn save_scene(path: String, text: String, front: Option<String>, seen: Stamp) -> Reply<Saved> {
    plainly(grimoire_app::save_scene(Path::new(&path), &text, front.as_deref(), &seen))
}

/// Remove a parked version once the writer has settled a conflict. The crate
/// refuses anything that is not a conflict copy, so a slip here cannot eat a
/// scene.
#[tauri::command]
fn drop_copy(path: String) -> Reply<()> {
    plainly(grimoire_app::drop_conflict_copy(Path::new(&path)))
}

/// Where the writer stopped, on any device that shares this shelf.
#[tauri::command]
fn resuming(app: tauri::AppHandle) -> Reply<Option<grimoire_app::Resuming>> {
    Ok(grimoire_app::resuming(&shelf(&app)?))
}

/// Remember this scene as the place to pick up. The desktop reads the same
/// file, so a sentence started on the sofa is waiting on the shelf at the desk.
#[tauri::command]
fn mark_place(book: String, scene: String, line: usize) -> Reply<()> {
    plainly(grimoire_app::mark_place(Path::new(&book), Path::new(&scene), line))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            base_dir,
            books,
            create_book,
            outline,
            read_scene,
            save_scene,
            drop_copy,
            resuming,
            mark_place
        ])
        .run(tauri::generate_context!())
        .expect("Grimoire could not start");
}
