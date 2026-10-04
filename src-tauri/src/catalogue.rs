use crate::model::{Asset, Dl};
use anyhow::{bail, Result};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

const PAGE: usize = 100;

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn parse_asset(v: &Value) -> Option<Asset> {
    let id = s(v, "assetId");
    if id.is_empty() {
        return None;
    }
    let mut downloads = Vec::new();
    if let Some(folders) = v.get("downloadFolders").and_then(Value::as_object) {
        for folder in folders.values() {
            let Some(cats) = folder
                .get("downloadFiletypeCategories")
                .and_then(Value::as_object)
            else {
                continue;
            };
            for cat in cats.values() {
                for d in cat
                    .get("downloads")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let file = s(d, "fileName");
                    let url = {
                        let u = s(d, "downloadLink");
                        if u.is_empty() { s(d, "fullDownloadPath") } else { u }
                    };
                    if file.is_empty() || url.is_empty() {
                        continue;
                    }
                    downloads.push(Dl {
                        file,
                        url,
                        attr: s(d, "attribute"),
                        size: d.get("size").and_then(Value::as_u64).unwrap_or(0),
                    });
                }
            }
        }
    }
    let kind = s(v, "dataType");
    let mut category = s(v, "displayCategory");
    if category.is_empty() {
        category = id.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
    }
    let thumb_url = v
        .get("previewImage")
        .and_then(|p| p.get("256-WEBP"))
        .and_then(Value::as_str)
        .map(String::from);
    Some(Asset {
        name: {
            let n = s(v, "displayName");
            if n.is_empty() { id.clone() } else { n }
        },
        id,
        kind,
        category,
        tags: v
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        release: s(v, "releaseDate"),
        link: s(v, "shortLink"),
        thumb_url,
        downloads,
    })
}

/// Fetch every asset of one type, page by page.
pub async fn fetch_type(
    client: &reqwest::Client,
    kind: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Vec<Asset>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let url = format!(
            "https://ambientcg.com/api/v2/full_json?type={kind}&limit={PAGE}&offset={offset}&sort=Latest&include=downloadData,previewData,tagData,displayData"
        );
        let mut last = None;
        let mut page: Option<Value> = None;
        for attempt in 0..4u64 {
            match client.get(&url).send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => match r.json::<Value>().await {
                    Ok(j) => {
                        page = Some(j);
                        break;
                    }
                    Err(e) => last = Some(e),
                },
                Err(e) => last = Some(e),
            }
            tokio::time::sleep(std::time::Duration::from_millis(1500 * (attempt + 1))).await;
        }
        let Some(page) = page else {
            bail!("AmbientCG API request failed: {}", last.map(|e| e.to_string()).unwrap_or_default());
        };
        let total = page.get("numberOfResults").and_then(Value::as_u64).unwrap_or(0) as usize;
        let found = page
            .get("foundAssets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let n = found.len();
        out.extend(found.iter().filter_map(parse_asset));
        offset += n;
        progress(offset.min(total), total);
        if n == 0 || offset >= total {
            break;
        }
    }
    Ok(out)
}
