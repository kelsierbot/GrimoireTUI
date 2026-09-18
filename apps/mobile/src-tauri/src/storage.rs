//! Reaching a folder the writer already syncs.
//!
//! Android hands an app one private folder and, by default, nothing else. That
//! is a fine place for books to start — no permission, nothing to explain —
//! but a manuscript locked inside an app is a manuscript with no way home. To
//! put the books in an Obsidian vault or a cloud folder, the app has to ask for
//! the system's permission to see files it did not create.
//!
//! Everything here is deliberately shallow: ask whether shared storage can be
//! written, and open the screen where the writer says yes. All of it returns
//! plain answers on a desktop so the same commands exist everywhere.

use std::path::{Path, PathBuf};

/// Where a phone keeps the folders a person would recognise, each with the
/// name they would use for it. Android's shared storage is literally a folder
/// called "0", which is nobody's idea of a place.
pub fn candidate_roots() -> Vec<(PathBuf, String)> {
    let named = |p: PathBuf, label: &str| (p, label.to_string());
    if cfg!(target_os = "android") {
        let shared = Path::new("/storage/emulated/0");
        vec![
            named(shared.join("Documents"), "Documents"),
            named(shared.join("Download"), "Downloads"),
            named(shared.join("Obsidian"), "Obsidian"),
            named(shared.to_path_buf(), "Phone storage"),
        ]
    } else {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        vec![
            named(home.join("Documents"), "Documents"),
            named(home.join("Grimoire"), "Grimoire"),
            named(home.clone(), "Home"),
        ]
    }
}

/// Can the app actually write outside its own sandbox right now? Asked by
/// writing, not by reading a permission flag: on Android a path can look
/// perfectly ordinary and still be refused, and the writer should learn that
/// here rather than with a paragraph in hand.
pub fn shared_storage_ready() -> bool {
    if !cfg!(target_os = "android") {
        return true;
    }
    let probe = Path::new("/storage/emulated/0/Documents").join(".grimoire-access-test");
    if let Some(parent) = probe.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&probe, b"grimoire") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Open the system screen where all-files access is granted, deep-linked to
/// this app so the writer lands on one switch rather than a list of hundreds.
#[cfg(target_os = "android")]
pub fn ask_for_shared_storage() -> Result<(), String> {
    use jni::objects::{JObject, JString, JValue};

    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| format!("no Java VM: {e}"))?;
    let mut env = vm.attach_current_thread().map_err(|e| format!("cannot reach the VM: {e}"))?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

    let package: JString = env
        .call_method(&activity, "getPackageName", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .map_err(|e| format!("asking the app its own name: {e}"))?
        .into();
    let package = env.get_string(&package).map_err(|e| format!("{e}"))?;
    let target = env
        .new_string(format!("package:{}", package.to_string_lossy()))
        .map_err(|e| format!("{e}"))?;

    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&target)],
        )
        .and_then(|v| v.l())
        .map_err(|e| format!("building the settings link: {e}"))?;

    let action = env
        .new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")
        .map_err(|e| format!("{e}"))?;
    let intent = env
        .new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )
        .map_err(|e| format!("building the request: {e}"))?;

    env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])
        .map_err(|e| format!("opening settings: {e}"))?;
    Ok(())
}

/// On a desktop there is nothing to ask for.
#[cfg(not(target_os = "android"))]
pub fn ask_for_shared_storage() -> Result<(), String> {
    Ok(())
}
