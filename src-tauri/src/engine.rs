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
    items.sort_by(|a, b| {
        res_rank(&a.attr)
            .cmp(&res_rank(&b.attr))
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
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()?)
}

async fn fetch_to(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    stats: Option<&Stats>,
    key: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..4u64 {
        if cancel.load(Relaxed) {
            bail!("cancelled");
        }
        let mut got = 0u64;
        let res: Result<()> = async {
            let resp = client.get(url).send().await?.error_for_status()?;
            let mut file = tokio::fs::File::create(dest).await?;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if cancel.load(Relaxed) {
                    bail!("cancelled");
                }
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
                tokio::time::sleep(Duration::from_millis(1500 * (attempt + 1))).await;
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
                if fetch_to(&client, &url, &part, None, "", &cancel).await.is_ok() {
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
pub async fn download_all(
    client: reqwest::Client,
    root: PathBuf,
    items: Vec<Item>,
    concurrency: usize,
    stats: Arc<Stats>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let tmp_dir = root.join(".tmp");
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)?;

    stats.files_total.store(items.len() as u64, Relaxed);
    stats.bytes_total.store(items.iter().map(|i| i.size).sum(), Relaxed);

    let manifest = Arc::new(Mutex::new(load_manifest(&root)));
    let since_save = Arc::new(AtomicU64::new(0));

    futures::stream::iter(items)
        .for_each_concurrent(concurrency.clamp(1, 8), |item| {
            let (root, tmp_dir, client) = (root.clone(), tmp_dir.clone(), client.clone());
            let (stats, cancel) = (stats.clone(), cancel.clone());
            let (manifest, since_save) = (manifest.clone(), since_save.clone());
            async move {
                if cancel.load(Relaxed) {
                    return;
                }
                stats.active.lock().unwrap().insert(item.file.clone(), (item.asset_id.clone(), 0, item.size));
                let result = process(&client, &root, &tmp_dir, &item, &stats, &cancel).await;
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
                    Err(e) => {
                        if !cancel.load(Relaxed) {
                            stats.failed.fetch_add(1, Relaxed);
                            *stats.last_error.lock().unwrap() = format!("{}: {e:#}", item.file);
                        }
                    }
                }
            }
        })
        .await;

    save_manifest(&root, &manifest.lock().unwrap())?;
    let _ = fs::remove_dir_all(&tmp_dir);
    Ok(())
}

async fn process(
    client: &reqwest::Client,
    root: &Path,
    tmp_dir: &Path,
    item: &Item,
    stats: &Stats,
    cancel: &AtomicBool,
) -> Result<()> {
    let part = tmp_dir.join(format!("{}.part", item.file));
    fetch_to(client, &item.url, &part, Some(stats), &item.file, cancel).await?;
    let dest = root.join(&item.rel_dir);
    let is_zip = item.file.to_ascii_lowercase().ends_with(".zip");
    let part2 = part.clone();
    let file = item.file.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if is_zip {
            extract_zip(&part2, &dest).with_context(|| format!("extracting {file}"))?;
            fs::remove_file(&part2)?;
        } else {
            fs::create_dir_all(&dest)?;
            fs::rename(&part2, dest.join(&file))?;
        }
        Ok(())
    })
    .await??;
    Ok(())
}
