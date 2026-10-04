use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        OnceLock,
    },
};
use tauri::{AppHandle, Emitter};

static FILE: OnceLock<PathBuf> = OnceLock::new();
static MARKER: OnceLock<PathBuf> = OnceLock::new();
static HANDLE: OnceLock<AppHandle> = OnceLock::new();
static NATIVE_SHOWN: AtomicBool = AtomicBool::new(false);

const TITLE: &str = "AmbientCG Downloader — crash report";

/// Start logging to `<dir>/app.log`, route panics and hard crashes there, and
/// detect whether the previous run ended without a clean exit.
pub fn init(dir: &Path) {
    let path = dir.join("app.log");
    // Keep the log small: start over once it passes 1 MB.
    if fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = fs::remove_file(&path);
    }
    let _ = FILE.set(path);

    // A marker file exists for as long as the app runs; it is removed on a clean exit.
    let marker = dir.join("running.lock");
    let unclean = marker.exists();
    let _ = fs::write(&marker, std::process::id().to_string());
    let _ = MARKER.set(marker);
    if unclean {
        let tail = tail_lines(20);
        line("previous run did not exit cleanly");
        // Separate thread: don't hold up startup while the dialog is open.
        std::thread::spawn(move || {
            native_box(&format!(
                "AmbientCG Downloader did not close normally last time.\n\nLast log lines:\n\n{tail}\n\nFull log: {}\n\n(Press Ctrl+C to copy this text.)",
                path_str()
            ));
        });
    }

    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let details = format!("{info}\nthread: {}\n{bt}", thread.name().unwrap_or("<unnamed>"));
        line(&format!("PANIC: {details}"));
        report(&details);
    }));

    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(on_exception));
    }
}

/// Give the panic hook a way to reach the UI.
pub fn set_handle(h: AppHandle) {
    let _ = HANDLE.set(h);
}

/// Call on a normal exit so the next start doesn't report a crash.
pub fn clean_exit() {
    line("app exited cleanly");
    if let Some(m) = MARKER.get() {
        let _ = fs::remove_file(m);
    }
}

pub fn path() -> Option<&'static Path> {
    FILE.get().map(|p| p.as_path())
}

fn path_str() -> String {
    path().map(|p| p.display().to_string()).unwrap_or_default()
}

/// Append one timestamped (UTC) line, written in a single call so concurrent lines don't interleave.
pub fn line(msg: &str) {
    let Some(path) = FILE.get() else { return };
    append(path, msg);
}

pub fn append(path: &Path, msg: &str) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let t = secs % 86_400;
    let text = format!("[{:02}:{:02}:{:02}Z] {msg}\n", t / 3600, (t % 3600) / 60, t % 60);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
    }
}

fn tail_lines(n: usize) -> String {
    let Some(p) = path() else { return String::new() };
    let text = fs::read_to_string(p).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    let out = lines[start..].join("\n");
    // Keep the dialog a sensible size.
    out.chars().rev().take(2500).collect::<Vec<_>>().into_iter().rev().collect()
}

/// Show details in the app's crash window; fall back to a native dialog when the UI isn't reachable.
fn report(details: &str) {
    if let Some(h) = HANDLE.get() {
        if h.emit("crash", details.to_string()).is_ok() {
            return;
        }
    }
    native_once(details);
}

fn native_once(details: &str) {
    // Only the first problem gets a dialog, so a cascade of panics can't bury the user.
    if NATIVE_SHOWN.swap(true, Relaxed) {
        return;
    }
    let short: String = details.chars().take(1800).collect();
    native_box(&format!(
        "AmbientCG Downloader hit an unexpected error.\n\n{short}\n\nFull log: {}\n\n(Press Ctrl+C to copy this text.)",
        path_str()
    ));
}

#[cfg(windows)]
fn native_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide(TITLE).as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(not(windows))]
fn native_box(_text: &str) {}

/// Access violations, illegal instructions and similar hard crashes that kill the process.
#[cfg(windows)]
unsafe extern "system" fn on_exception(info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    let (code, addr) = if info.is_null() || (*info).ExceptionRecord.is_null() {
        (0u32, 0usize)
    } else {
        let r = &*(*info).ExceptionRecord;
        (r.ExceptionCode as u32, r.ExceptionAddress as usize)
    };
    let details = format!("Fatal exception {code:#010x} at address {addr:#x} (the app is closing)");
    line(&format!("CRASH: {details}"));
    native_once(&details);
    1 // EXCEPTION_EXECUTE_HANDLER: terminate quietly after the report
}
