//! Saving on the way out when something other than Ctrl-Q ends the session.
//!
//! Closing the terminal window sends SIGHUP; a logout or `kill` sends SIGTERM.
//! Left alone, either ends the process on the spot and takes every unsaved
//! word with it. Here they only raise a flag. The event loop sees it, saves,
//! and exits properly.
//!
//! Windows is harsher: when the console window closes, the process is ended as
//! soon as the handler returns. So the handler waits — up to the few seconds
//! Windows allows — for the event loop to report that it has saved.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static SAVED: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
static SIGNALLED: std::sync::OnceLock<std::sync::Arc<AtomicBool>> = std::sync::OnceLock::new();

/// Has the outside world asked us to stop?
pub fn requested() -> bool {
    #[cfg(unix)]
    if SIGNALLED.get().is_some_and(|f| f.load(Ordering::SeqCst)) {
        return true;
    }
    REQUESTED.load(Ordering::SeqCst)
}

/// Tell a waiting handler that everything that could be saved has been.
pub fn done() {
    SAVED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
pub fn install() {
    let flag = SIGNALLED.get_or_init(|| std::sync::Arc::new(AtomicBool::new(false)));
    for sig in [signal_hook::consts::SIGHUP, signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        let _ = signal_hook::flag::register(sig, flag.clone());
    }
}

#[cfg(windows)]
pub fn install() {
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        SetConsoleCtrlHandler,
    };

    unsafe extern "system" fn handler(kind: u32) -> windows_sys::core::BOOL {
        match kind {
            CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
                REQUESTED.store(true, Ordering::SeqCst);
                if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
                    // Returning lets Windows end the process, so hold on until
                    // the loop has saved, within the grace period it gives us.
                    for _ in 0..90 {
                        if SAVED.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                1
            }
            _ => 0,
        }
    }

    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(any(unix, windows)))]
pub fn install() {}
