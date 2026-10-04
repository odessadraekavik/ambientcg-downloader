use crate::model::*;
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::{
    fs,
    io::{self, BufWriter},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// ---------- paths & persistence ----------

pub fn root_of(s: &Settings) -> Option<PathBuf> {
    let p = s.library_path.trim();
    if p.is_empty() {
        return None;
    }
    let base = PathBuf::from(p);
    let is_named = base
        .file_name()
        .map(|n| n.to_string_lossy().eq_ignore_ascii_case("AmbientCG"))
        .unwrap_or(false);
    Some(if is_named { base } else { base.join("AmbientCG") })
}

pub fn thumbs_dir(root: &Path) -> PathBuf {
    root.join(".thumbnails")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_catalogue(root: &Path) -> Vec<Asset> {
    fs::read(root.join(".catalogue.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_catalogue(root: &Path, assets: &[Asset]) -> Result<()> {
    write_atomic(&root.join(".catalogue.json"), &serde_json::to_vec(assets)?)
}

pub fn load_manifest(root: &Path) -> Manifest {
    fs::read(root.join(".manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_manifest(root: &Path, m: &Manifest) -> Result<()> {
    write_atomic(&root.join(".manifest.json"), &serde_json::to_vec(m)?)
}

/// Everything in the catalogue that matches the settings and isn't on disk yet.
pub fn plan(root: &Path, s: &Settings) -> Vec<Item> {
    let manifest = load_manifest(root);
    let mut items: Vec<Item> = load_catalogue(root)
        .iter()
        .filter(|a| s.types.iter().any(|t| t == &a.kind))
        .flat_map(|a| items_for(a, s))
        .filter(|i| {
            !(manifest.done.contains_key(&i.file) && root.join(&i.rel_dir).is_dir())
        })
        .collect();
    // Files that failed before go to the very end (the more failures, the later).
    let fails = |i: &Item| manifest.failed.get(&i.file).copied().unwrap_or(0);
    items.sort_by(|a, b| {
        fails(a)
            .cmp(&fails(b))
            .then_with(|| res_rank(&a.attr).cmp(&res_rank(&b.attr)))
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });
    items
}

pub fn summarize(items: &[Item]) -> PlanSummary {
    let mut ids: Vec<&str> = items.iter().map(|i| i.asset_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    PlanSummary {
        files: items.len() as u64,
        bytes: items.iter().map(|i| i.size).sum(),
        assets: ids.len() as u64,
    }
}

// ---------- shared progress state ----------

#[derive(Default)]
pub struct Stats {
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub failed: AtomicU64,
    pub active: Mutex<std::collections::HashMap<String, (String, u64, u64)>>,
    pub last_error: Mutex<String>,
    pub completed: Mutex<Vec<Completed>>,
}

impl Stats {
    pub fn snapshot(&self, phase: &str, message: &str, speed: u64) -> Progress {
        Progress {
            phase: phase.into(),
            message: message.into(),
            files_done: self.files_done.load(Relaxed),
            files_total: self.files_total.load(Relaxed),
            bytes_done: self.bytes_done.load(Relaxed),
            bytes_total: self.bytes_total.load(Relaxed),
            speed,
            failed: self.failed.load(Relaxed),
            completed: std::mem::take(&mut *self.completed.lock().unwrap()),
            current: {
                let a = self.active.lock().unwrap();
                a.keys().cloned().collect()
            },
            assets: {
                let mut m: std::collections::HashMap<String, (u64, u64)> = Default::default();
                for (asset, done, total) in self.active.lock().unwrap().values() {
                    let e = m.entry(asset.clone()).or_default();
                    e.0 += done;
                    e.1 += total;
                }
                m.into_iter().map(|(id, (done, total))| AssetProgress { id, done, total }).collect()
            },
        }
    }
}

// ---------- downloading ----------

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("AmbientCG-Downloader/0.1 (personal library sync)")
        .connect_timeout(Duration::from_secs(8))
        .read_timeout(Duration::from_secs(12))
        .build()?)
}

/// Resolves once the cancel flag is set, so network waits can be interrupted immediately.
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn fetch_to(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    stats: Option<&Stats>,
    key: &str,
    attempts: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..attempts {
        if cancel.load(Relaxed) {
            bail!("cancelled");
        }
        let mut got = 0u64;
        let res: Result<()> = async {
            let resp = tokio::select! {
                _ = cancelled(cancel) => bail!("cancelled"),
                r = client.get(url).send() => r?,
            };
            let resp = resp.error_for_status()?;
            let mut file = tokio::io::BufWriter::with_capacity(1 << 20, tokio::fs::File::create(dest).await?);
            let mut stream = resp.bytes_stream();
            loop {
                let next = tokio::select! {
                    _ = cancelled(cancel) => bail!("cancelled"),
                    c = stream.next() => c,
                };
                let Some(chunk) = next else { break };
                let c = chunk?;
                file.write_all(&c).await?;
                got += c.len() as u64;
                if let Some(s) = stats {
                    s.bytes_done.fetch_add(c.len() as u64, Relaxed);
                    if let Some(a) = s.active.lock().unwrap().get_mut(key) {
                        a.1 += c.len() as u64;
                    }
                }
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        match res {
            Ok(()) => return Ok(()),
            Err(e) => {
                if let Some(s) = stats {
                    s.bytes_done.fetch_sub(got, Relaxed);
                    if let Some(a) = s.active.lock().unwrap().get_mut(key) {
                        a.1 = 0;
                    }
                }
                let _ = tokio::fs::remove_file(dest).await;
                if cancel.load(Relaxed) {
                    bail!("cancelled");
                }
                last = Some(e);
                if attempt + 1 < attempts {
                    tokio::select! {
                        _ = cancelled(cancel) => bail!("cancelled"),
                        _ = tokio::time::sleep(Duration::from_millis(1000 * (attempt + 1))) => {}
                    }
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    fs::create_dir_all(dest)?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // Zips are flat; keep just the (safe) file name.
        let Some(name) = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_owned()))
        else {
            continue;
        };
        let mut out = BufWriter::new(fs::File::create(dest.join(name))?);
        io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

/// Download missing thumbnails into `.thumbnails/`.
pub async fn fetch_thumbnails(
    client: reqwest::Client,
    root: &Path,
    assets: &[Asset],
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(usize, usize),
) -> Result<()> {
    let dir = thumbs_dir(root);
    fs::create_dir_all(&dir)?;
    let jobs: Vec<(String, String)> = assets
        .iter()
        .filter(|a| a.thumb_url.is_some() && !dir.join(format!("{}.webp", a.id)).exists())
        .map(|a| (a.id.clone(), a.thumb_url.clone().unwrap()))
        .collect();
    let total = jobs.len();
    let mut done = 0usize;
    let mut stream = futures::stream::iter(jobs)
        .map(|(id, url)| {
            let (dir, client, cancel) = (dir.clone(), client.clone(), cancel.clone());
            async move {
                let part = dir.join(format!("{id}.part"));
                if fetch_to(&client, &url, &part, None, "", 3, &cancel).await.is_ok() {
                    let _ = fs::rename(&part, dir.join(format!("{id}.webp")));
                }
            }
        })
        .buffer_unordered(8);
    while stream.next().await.is_some() {
        if cancel.load(Relaxed) {
            bail!("cancelled");
        }
        done += 1;
        if done % 10 == 0 || done == total {
            progress(done, total);
        }
    }
    Ok(())
}

/// Download, extract and register every item. Resumable: finished items are skipped next time.
/// How often a file is attempted within one run before it is given up until the next run.
const RUN_TRIES: u32 = 2;

pub async fn download_all(
    client: reqwest::Client,
    root: PathBuf,
    items: Vec<Item>,
    concurrency: usize,
    stats: Arc<Stats>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let n = concurrency.clamp(1, 8);
    let tmp_dir = root.join(".tmp");
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)?;

    stats.files_total.store(items.len() as u64, Relaxed);
    stats.bytes_total.store(items.iter().map(|i| i.size).sum(), Relaxed);

    let manifest = Arc::new(Mutex::new(load_manifest(&root)));
    let since_save = Arc::new(AtomicU64::new(0));
    // Separate limits: network slots are freed as soon as the zip is on disk,
    // so slow extraction (antivirus!) never starves the downloads.
    let dl_sem = Arc::new(Semaphore::new(n));
    let ex_sem = Arc::new(Semaphore::new(n));
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(items)));
    let tries: Arc<Mutex<std::collections::HashMap<String, u32>>> = Default::default();
    let mut set = tokio::task::JoinSet::new();

    loop {
        if cancel.load(Relaxed) {
            break;
        }
        let next = queue.lock().unwrap().pop_front();
        let Some(item) = next else {
            // Queue is empty: wait for running tasks (a failing one may re-queue itself).
            if set.join_next().await.is_none() {
                break;
            }
            continue;
        };
        let permit = tokio::select! {
            _ = cancelled(&cancel) => break,
            p = dl_sem.clone().acquire_owned() => match p { Ok(p) => p, Err(_) => break },
        };
        let (root, tmp_dir, client, ex_sem) = (root.clone(), tmp_dir.clone(), client.clone(), ex_sem.clone());
        let (stats, cancel) = (stats.clone(), cancel.clone());
        let (manifest, since_save) = (manifest.clone(), since_save.clone());
        let (queue, tries) = (queue.clone(), tries.clone());
        // Every item is its own task, so downloads really run in parallel on the runtime's threads.
        set.spawn(async move {
            stats
                .active
                .lock()
                .unwrap()
                .insert(item.file.clone(), (item.asset_id.clone(), 0, item.size));
            let result = run_item(&client, &root, &tmp_dir, &item, &stats, &cancel, permit, &ex_sem).await;
            stats.active.lock().unwrap().remove(&item.file);
            match result {
                Ok(()) => {
                    stats.files_done.fetch_add(1, Relaxed);
                    stats.completed.lock().unwrap().push(Completed {
                        id: item.asset_id.clone(),
                        attr: item.attr.clone(),
                        rel: item.rel_dir.clone(),
                    });
                    let mut m = manifest.lock().unwrap();
                    m.failed.remove(&item.file);
                    m.done.insert(
                        item.file.clone(),
                        Done {
                            asset: item.asset_id.clone(),
                            attr: item.attr.clone(),
                            dir: item.rel_dir.clone(),
                            size: item.size,
                        },
                    );
                    if since_save.fetch_add(1, Relaxed) % 10 == 9 {
                        let _ = save_manifest(&root, &m);
                    }
                }
                Err(_) if cancel.load(Relaxed) => {}
                Err(e) => {
                    let msg = format!("{}: {e:#}", item.file);
                    log_error(&root, &msg);
                    let attempt = {
                        let mut t = tries.lock().unwrap();
                        let c = t.entry(item.file.clone()).or_default();
                        *c += 1;
                        *c
                    };
                    if attempt < RUN_TRIES {
                        // Do not hammer a failing file: try it again after everything else.
                        queue.lock().unwrap().push_back(item);
                    } else {
                        stats.failed.fetch_add(1, Relaxed);
                        *stats.last_error.lock().unwrap() = msg;
                        // Remember it, so it is also queued last after a restart.
                        let mut m = manifest.lock().unwrap();
                        *m.failed.entry(item.file.clone()).or_default() += 1;
                        let _ = save_manifest(&root, &m);
                    }
                }
            }
        });
        while set.try_join_next().is_some() {}
    }
    while set.join_next().await.is_some() {}

    save_manifest(&root, &manifest.lock().unwrap())?;
    let _ = fs::remove_dir_all(&tmp_dir);
    Ok(())
}

fn log_error(root: &Path, msg: &str) {
    crate::log::append(&root.join(".errors.log"), msg);
    crate::log::line(&format!("download failed: {msg}"));
}

#[allow(clippy::too_many_arguments)]
async fn run_item(
    client: &reqwest::Client,
    root: &Path,
    tmp_dir: &Path,
    item: &Item,
    stats: &Stats,
    cancel: &AtomicBool,
    dl_permit: OwnedSemaphorePermit,
    ex_sem: &Arc<Semaphore>,
) -> Result<()> {
    let part = tmp_dir.join(format!("{}.part", item.file));
    fetch_to(client, &item.url, &part, Some(stats), &item.file, 1, cancel).await?;

    // Hand the network slot to the next download; wait for an extraction slot first
    // so finished zips can't pile up on disk faster than they are unpacked.
    let _ex = ex_sem.clone().acquire_owned().await?;
    drop(dl_permit);
    if cancel.load(Relaxed) {
        let _ = fs::remove_file(&part);
        bail!("cancelled");
    }

    let dest = root.join(&item.rel_dir);
    let is_zip = item.file.to_ascii_lowercase().ends_with(".zip");
    let file = item.file.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if is_zip {
            extract_zip(&part, &dest).with_context(|| format!("extracting {file}"))?;
            fs::remove_file(&part)?;
        } else {
            fs::create_dir_all(&dest)?;
            fs::rename(&part, dest.join(&file))?;
        }
        Ok(())
    })
    .await??;
    Ok(())
}
