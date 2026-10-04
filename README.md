<div align="center">

# ⬇ AmbientCG Downloader

**Mirror the entire [ambientCG](https://ambientcg.com) library to your disk — organised, thumbnailed and searchable.**

![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![Svelte](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-backend-DEA584?logo=rust&logoColor=white)
![Platform](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?logo=windows&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-green)

</div>

> **Unofficial.** This project is not affiliated with or endorsed by ambientCG. All assets are published by ambientCG under [CC0](https://creativecommons.org/publicdomain/zero/1.0/). If you use their library, please consider [supporting them](https://ambientcg.com/).

---

## ✨ Features

- **One-click sync** — a single **Download** button fetches everything you selected. Press it again later and it doubles as *check for updates*: only new assets are downloaded.
- **Pick exactly what you want** — tick the resolutions (1K → 16K), formats (JPG / PNG) and asset types (Materials, HDRIs, Decals, Atlases, Terrains, Brushes, 3D Models, Substances).
- **Know the cost first** — before anything downloads you see the file count and total size.
- **Neatly organised** — files land in `Type / Resolution / Category / Asset`, ready to point Blender, Unreal or Unity at.
- **Cached thumbnails** — previews are stored in the library, so browsing works offline.
- **Instant local search** — search by name, tag or category as you type (`Ctrl + K`), filter by type, category or downloaded / not downloaded.
- **Live progress** — a global progress bar (speed, ETA, current files) plus a small bar on every thumbnail being downloaded, and the green mark appears the moment an asset finishes.
- **Cancel & resume** — cancel any time; finished assets are kept, partial files are discarded and re-downloaded on the next run.
- **Small & fast** — ~2.5 MB installer, native Rust downloader.

## 📁 Library layout

```text
<your folder>/AmbientCG/
├─ Materials/
│  ├─ 1K-JPG/
│  │  └─ Wood/
│  │     └─ Wood052/
│  │        ├─ Wood052_1K-JPG_Color.jpg
│  │        ├─ Wood052_1K-JPG_NormalGL.jpg
│  │        └─ …
│  └─ 2K-JPG/ …
├─ HDRIs/ …
├─ .thumbnails/Wood052.webp
├─ .catalogue.json
└─ .manifest.json
```

File names inside each asset are kept exactly as ambientCG ships them, because the bundled `.usdc` / `.mtlx` files reference them.

## 🚀 Getting started

### Download

Grab the latest installer or exe from the [Releases](../../releases) page, or build it yourself (below).

### Use it

1. Open **Settings** (⚙) and choose your **library folder**.
2. Tick the **resolutions**, **formats** and **asset types** you want.
3. Hit **Download**, review the size summary and confirm.

> ⚠️ Higher resolutions are huge (an 8K-PNG material is ~800 MB). The defaults are Materials · 1K + 2K · JPG.

## 🛠 Build from source

Requirements: [Rust](https://rustup.rs), [Node.js](https://nodejs.org) LTS, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for Windows (MSVC build tools + WebView2).

```bash
npm install
npm run tauri dev       # run in development
npm run tauri build     # release build → src-tauri/target/release
```

The release exe is at `src-tauri/target/release/ambientcg-downloader.exe` and the installer in `src-tauri/target/release/bundle/nsis/`.

## 🧱 How it works

| Layer | What it does |
| --- | --- |
| **Rust** (`src-tauri/src`) | Reads the public ambientCG API, caches the catalogue, downloads in parallel with retries, extracts zips safely, and tracks finished files in a manifest. |
| **Svelte** (`src`) | The UI: library grid, search, settings drawer, live progress. |

Be kind to ambientCG's servers: the app identifies itself with its own user-agent and limits parallel downloads to 8 (default 4).

## 📄 License

[MIT](LICENSE) for this app's code. The downloaded assets are © ambientCG, released under CC0.
