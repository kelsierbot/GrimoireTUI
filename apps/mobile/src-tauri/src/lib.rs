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

mod storage;

use grimoire_app::shelf::Folder;
use grimoire_app::{Book, Outline, Saved, Scene, Stamp};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Errors cross to the front end as plain sentences a writer could read.
type Reply<T> = Result<T, String>;

fn plainly<T>(r: anyhow::Result<T>) -> Reply<T> {
    r.map_err(|e| e.to_string())
}

/// The app's own folder, where books live until the writer says otherwise.
fn private_shelf(app: &tauri::AppHandle) -> Reply<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no app folder: {e}"))?
        .join("Books");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Where settings live — never inside the books themselves, which move.
fn config(app: &tauri::AppHandle) -> Reply<PathBuf> {
    app.path().app_config_dir().map_err(|e| format!("no config folder: {e}"))
}

/// The shelf in use: what the writer chose, or the app's own folder when they
/// never chose, or when what they chose has since gone.
fn shelf(app: &tauri::AppHandle) -> Reply<PathBuf> {
    let fallback = private_shelf(app)?;
    let dir = grimoire_app::shelf::root(&config(app)?, &fallback);
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

/// Everything the "where your books live" screen needs in one answer: the
/// shelf now, whether it is the app's own folder, whether folders outside the
/// sandbox can be reached at all, and somewhere to start browsing.
#[derive(serde::Serialize)]
struct Where {
    root: PathBuf,
    private: bool,
    books: usize,
    shared_storage: bool,
    places: Vec<Folder>,
}

#[tauri::command]
fn shelf_now(app: tauri::AppHandle) -> Reply<Where> {
    let root = shelf(&app)?;
    let private = root == private_shelf(&app)?;
    Ok(Where {
        books: grimoire_app::books(&root).len(),
        private,
        shared_storage: storage::shared_storage_ready(),
        places: grimoire_app::shelf::places(&storage::candidate_roots()),
        root,
    })
}

/// Browse: the folders inside one folder, with what each already holds.
#[tauri::command]
fn folders(dir: String) -> Reply<Vec<Folder>> {
    plainly(grimoire_app::shelf::folders(Path::new(&dir)))
}

/// Put the shelf somewhere else. `bring_books` moves what is already written —
/// a writer who changes this setting and finds their book gone would be right
/// to never trust the app again.
#[tauri::command]
fn choose_shelf(app: tauri::AppHandle, dir: String, bring_books: bool) -> Reply<usize> {
    let from = shelf(&app)?;
    let to = Path::new(&dir);
    let moved = if bring_books { plainly(grimoire_app::shelf::move_books(&from, to))? } else { 0 };
    plainly(grimoire_app::shelf::choose(&config(&app)?, to))?;
    Ok(moved)
}

/// Open the system screen that grants access to folders outside the sandbox.
#[tauri::command]
fn ask_for_storage() -> Reply<()> {
    storage::ask_for_shared_storage()
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
            mark_place,
            shelf_now,
            folders,
            choose_shelf,
            ask_for_storage
        ])
        .run(tauri::generate_context!())
        .expect("Grimoire could not start");
}
