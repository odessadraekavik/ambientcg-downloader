mod catalogue;
mod engine;
mod model;

use model::*;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    settings: Mutex<Settings>,
    settings_file: PathBuf,
    /// `Some` while a prepare/download job is running.
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    pending: Mutex<Vec<Item>>,
    client: reqwest::Client,
}

impl AppState {
    fn begin(&self) -> Result<Arc<AtomicBool>, String> {
        let mut g = self.cancel.lock().unwrap();
        if g.is_some() {
            return Err("A job is already running".into());
        }
        let flag = Arc::new(AtomicBool::new(false));
        *g = Some(flag.clone());
        Ok(flag)
    }
    fn end(&self) {
        *self.cancel.lock().unwrap() = None;
    }
    fn root(&self) -> Result<PathBuf, String> {
        engine::root_of(&self.settings.lock().unwrap()).ok_or_else(|| "Choose a library folder first".to_string())
    }
}

fn e2s(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn allow_thumbs(app: &AppHandle, s: &Settings) {
    if let Some(root) = engine::root_of(s) {
        let dir = engine::thumbs_dir(&root);
        let _ = fs::create_dir_all(&dir);
        let _ = app.asset_protocol_scope().allow_directory(dir, false);
    }
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
    let mut settings = settings;
    settings.concurrency = settings.concurrency.clamp(1, 8);
    fs::write(&state.settings_file, serde_json::to_vec_pretty(&settings).map_err(e2s)?).map_err(e2s)?;
    allow_thumbs(&app, &settings);
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
fn load_library(state: State<AppState>) -> Result<LibraryView, String> {
    let root = state.root()?;
    let catalogue = engine::load_catalogue(&root);
    let manifest = engine::load_manifest(&root);
    let thumbs = engine::thumbs_dir(&root);

    let mut by_asset: std::collections::HashMap<&str, Vec<Folder>> = Default::default();
    for d in manifest.done.values() {
        let list = by_asset.entry(d.asset.as_str()).or_default();
        if !list.iter().any(|f| f.rel == d.dir) {
            list.push(Folder { attr: d.attr.clone(), rel: d.dir.clone() });
        }
    }
    for list in by_asset.values_mut() {
        list.sort_by(|a, b| res_rank(&a.attr).cmp(&res_rank(&b.attr)).then(a.attr.cmp(&b.attr)));
    }

    let entries = catalogue
        .into_iter()
        .map(|a| Entry {
            has_thumb: thumbs.join(format!("{}.webp", a.id)).exists(),
            folders: by_asset.get(a.id.as_str()).cloned().unwrap_or_default(),
            id: a.id,
            name: a.name,
            kind: a.kind,
            category: a.category,
            tags: a.tags,
            release: a.release,
            link: a.link,
        })
        .collect();
    Ok(LibraryView {
        root: root.to_string_lossy().into_owned(),
        thumbs_dir: thumbs.to_string_lossy().into_owned(),
        entries,
    })
}

/// What is still missing for the current settings, based on the cached catalogue.
#[tauri::command]
fn estimate(state: State<AppState>) -> Result<PlanSummary, String> {
    let root = state.root()?;
    let s = state.settings.lock().unwrap().clone();
    Ok(engine::summarize(&engine::plan(&root, &s)))
}

fn emit_prepare(app: &AppHandle, message: String, done: usize, total: usize) {
    let _ = app.emit(
        "sync-progress",
        Progress {
            phase: "prepare".into(),
            message,
            files_done: done as u64,
            files_total: total as u64,
            ..Default::default()
        },
    );
}

async fn prepare(
    app: AppHandle,
    client: reqwest::Client,
    s: Settings,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<Item>, String> {
    fs::create_dir_all(&root).map_err(e2s)?;
    allow_thumbs(&app, &s);

    let mut catalogue = engine::load_catalogue(&root);
    for kind in &s.types {
        let msg = format!("Reading {kind} catalogue");
        let fresh = catalogue::fetch_type(&client, kind, &cancel, |d, t| {
            emit_prepare(&app, msg.clone(), d, t)
        })
        .await
        .map_err(e2s)?;
        catalogue.retain(|a| &a.kind != kind);
        catalogue.extend(fresh);
    }
    engine::save_catalogue(&root, &catalogue).map_err(e2s)?;

    let wanted: Vec<Asset> = catalogue.iter().filter(|a| s.types.contains(&a.kind)).cloned().collect();
    engine::fetch_thumbnails(client.clone(), &root, &wanted, cancel.clone(), |d, t| {
        emit_prepare(&app, "Caching thumbnails".into(), d, t)
    })
    .await
    .map_err(e2s)?;

    Ok(engine::plan(&root, &s))
}

/// Refresh the catalogue + thumbnails, then work out what needs downloading.
#[tauri::command]
async fn prepare_sync(app: AppHandle, state: State<'_, AppState>) -> Result<PlanSummary, String> {
    let s = state.settings.lock().unwrap().clone();
    let root = state.root()?;
    let cancel = state.begin()?;
    let res = prepare(app, state.client.clone(), s, root, cancel).await;
    state.end();
    let items = res?;
    let summary = engine::summarize(&items);
    *state.pending.lock().unwrap() = items;
    Ok(summary)
}

#[tauri::command]
fn start_download(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    let root = state.root()?;
    let items = std::mem::take(&mut *state.pending.lock().unwrap());
    if items.is_empty() {
        return Err("Nothing to download".into());
    }
    let cancel = state.begin()?;
    let client = state.client.clone();

    tauri::async_runtime::spawn(async move {
        let stats = Arc::new(engine::Stats::default());
        stats.files_total.store(items.len() as u64, Relaxed);
        stats.bytes_total.store(items.iter().map(|i| i.size).sum(), Relaxed);

        let finished = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (app, stats, finished) = (app.clone(), stats.clone(), finished.clone());
            tauri::async_runtime::spawn(async move {
                let (mut last_bytes, mut last_t, mut speed) = (0u64, Instant::now(), 0f64);
                while !finished.load(Relaxed) {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let now = stats.bytes_done.load(Relaxed);
                    let dt = last_t.elapsed().as_secs_f64().max(0.001);
                    let inst = now.saturating_sub(last_bytes) as f64 / dt;
                    speed = if speed == 0.0 { inst } else { speed * 0.8 + inst * 0.2 };
                    last_bytes = now;
                    last_t = Instant::now();
                    let _ = app.emit("sync-progress", stats.snapshot("download", "Downloading", speed as u64));
                }
            })
        };

        let result =
            engine::download_all(client, root, items, s.concurrency, stats.clone(), cancel.clone()).await;
        finished.store(true, Relaxed);
        let _ = ticker.await;

        let cancelled = cancel.load(Relaxed);
        let (phase, msg) = match result {
            Err(e) => ("error", e2s(e)),
            Ok(()) if cancelled => ("cancelled", "Download cancelled".to_string()),
            Ok(()) => {
                let failed = stats.failed.load(Relaxed);
                if failed > 0 {
                    ("done", format!("Finished with {failed} failed file(s) — run Download again to retry. {}", stats.last_error.lock().unwrap()))
                } else {
                    ("done", "Library is up to date".to_string())
                }
            }
        };
        let _ = app.emit("sync-progress", stats.snapshot(phase, &msg, 0));
        app.state::<AppState>().end();
    });
    Ok(())
}

#[tauri::command]
fn cancel_sync(state: State<AppState>) {
    if let Some(flag) = state.cancel.lock().unwrap().as_ref() {
        flag.store(true, Relaxed);
    }
}

fn open_in_explorer(path: PathBuf) {
    let mut cmd = std::process::Command::new("explorer");
    cmd.arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd.spawn();
}

#[tauri::command]
fn exe_dir() -> Option<String> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_library(state: State<AppState>) -> Result<(), String> {
    let root = state.root()?;
    fs::create_dir_all(&root).map_err(e2s)?;
    open_in_explorer(root);
    Ok(())
}

#[tauri::command]
fn open_folder(state: State<AppState>, rel: String) -> Result<(), String> {
    let root = state.root()?;
    let mut p = root.clone();
    for part in rel.split('/') {
        if part.is_empty() || part == ".." || part.contains(':') || part.contains('\\') {
            return Err("Invalid path".into());
        }
        p.push(part);
    }
    if !p.is_dir() {
        return Err("Folder not found".into());
    }
    open_in_explorer(p);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            fs::create_dir_all(&dir)?;
            let settings_file = dir.join("settings.json");
            let settings: Settings = fs::read(&settings_file)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            allow_thumbs(app.handle(), &settings);
            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_file,
                cancel: Mutex::new(None),
                pending: Mutex::new(Vec::new()),
                client: engine::http_client()?,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            load_library,
            estimate,
            prepare_sync,
            start_download,
            cancel_sync,
            exe_dir,
            open_library,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
