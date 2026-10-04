use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// Start logging to `<dir>/app.log` and route panics there (with a backtrace).
pub fn init(dir: &Path) {
    let path = dir.join("app.log");
    // Keep the log small: start over once it passes 1 MB.
    if fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = fs::remove_file(&path);
    }
    let _ = FILE.set(path);
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        line(&format!("PANIC: {info}\n{bt}"));
    }));
}

pub fn path() -> Option<&'static Path> {
    FILE.get().map(|p| p.as_path())
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
