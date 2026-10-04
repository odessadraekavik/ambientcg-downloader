use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub library_path: String,
    pub resolutions: Vec<String>,
    pub formats: Vec<String>,
    pub types: Vec<String>,
    pub concurrency: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library_path: String::new(),
            resolutions: vec!["1K".into(), "2K".into()],
            formats: vec!["JPG".into()],
            types: vec!["Material".into()],
            concurrency: 4,
        }
    }
}

/// One downloadable file of an asset (e.g. `Wood052_1K-JPG.zip`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Dl {
    pub file: String,
    pub url: String,
    pub attr: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category: String,
    pub tags: Vec<String>,
    pub release: String,
    pub link: String,
    pub thumb_url: Option<String>,
    pub downloads: Vec<Dl>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Done {
    pub asset: String,
    pub attr: String,
    /// Directory relative to the library root, `/`-separated.
    pub dir: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Manifest {
    /// Keyed by file name.
    pub done: HashMap<String, Done>,
    /// Files that failed in earlier runs (file name -> times failed). Retried last.
    #[serde(default)]
    pub failed: HashMap<String, u32>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub asset_id: String,
    pub file: String,
    pub url: String,
    pub attr: String,
    pub size: u64,
    pub rel_dir: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub files: u64,
    pub bytes: u64,
    pub assets: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Folder {
    pub attr: String,
    pub rel: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category: String,
    pub tags: Vec<String>,
    pub release: String,
    pub link: String,
    pub has_thumb: bool,
    pub folders: Vec<Folder>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub root: String,
    pub thumbs_dir: String,
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub message: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed: u64,
    pub failed: u64,
    pub current: Vec<String>,
    pub assets: Vec<AssetProgress>,
    pub completed: Vec<Completed>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Completed {
    pub id: String,
    pub attr: String,
    pub rel: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AssetProgress {
    pub id: String,
    pub done: u64,
    pub total: u64,
}

pub fn type_folder(kind: &str) -> &'static str {
    match kind {
        "Material" => "Materials",
        "HDRI" => "HDRIs",
        "Decal" => "Decals",
        "Atlas" => "Atlases",
        "Terrain" => "Terrains",
        "Brush" => "Brushes",
        "3DModel" => "Models",
        "Substance" => "Substances",
        _ => "Other",
    }
}

/// Splits `4K-JPG` into (`Some("4K")`, `"JPG"`); attributes without a resolution
/// prefix (e.g. `COMPILED`) give `(None, "")`.
pub fn split_attr(attr: &str) -> (Option<&str>, &str) {
    let (head, rest) = match attr.split_once('-') {
        Some((h, r)) => (h, r),
        None => (attr, ""),
    };
    let is_res = head.len() >= 2
        && head.ends_with('K')
        && head[..head.len() - 1].chars().all(|c| c.is_ascii_digit());
    if is_res {
        (Some(head), rest)
    } else {
        (None, "")
    }
}

pub fn res_rank(attr: &str) -> u32 {
    match split_attr(attr).0 {
        Some(r) => r[..r.len() - 1].parse().unwrap_or(999),
        None => 0,
    }
}

pub fn wanted(s: &Settings, attr: &str) -> bool {
    match split_attr(attr) {
        (None, _) => true,
        (Some(res), variant) => {
            s.resolutions.iter().any(|r| r.eq_ignore_ascii_case(res))
                && (variant.is_empty()
                    || s.formats.iter().any(|f| {
                        f.eq_ignore_ascii_case(variant.split('-').next().unwrap_or(""))
                    }))
        }
    }
}

pub fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() {
        "Uncategorized".into()
    } else {
        t
    }
}

pub fn items_for(a: &Asset, s: &Settings) -> Vec<Item> {
    a.downloads
        .iter()
        .filter(|d| wanted(s, &d.attr))
        .map(|d| Item {
            asset_id: a.id.clone(),
            file: d.file.clone(),
            url: d.url.clone(),
            attr: d.attr.clone(),
            size: d.size,
            rel_dir: format!(
                "{}/{}/{}/{}",
                type_folder(&a.kind),
                sanitize(&d.attr),
                sanitize(&a.category),
                sanitize(&a.id)
            ),
        })
        .collect()
}
