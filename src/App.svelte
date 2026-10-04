<script>
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';
  import Settings from './lib/Settings.svelte';
  import { fmtBytes, fmtEta, TYPE_LABELS } from './lib/format.js';

  let settings = $state(null);
  let lib = $state({ root: '', thumbsDir: '', entries: [] });
  let estimate = $state(null);
  let progress = $state(null);
  let busy = $state(false);
  let plan = $state(null);
  let showSettings = $state(false);
  let selected = $state(null);
  let toast = $state('');
  let toastTimer;

  let query = $state('');
  let typeFilter = $state('All');
  let status = $state('all');
  let category = $state('All');
  let sort = $state('name');
  let limit = $state(200);
  let searchEl = $state();
  let sentinel = $state();

  const say = (msg) => {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ''), 6000);
  };

  async function loadLibrary() {
    if (!settings?.libraryPath) return;
    try {
      const v = await invoke('load_library');
      v.entries.forEach((e) => (e.hay = `${e.name} ${e.id} ${e.category} ${e.kind} ${e.tags.join(' ')}`.toLowerCase()));
      lib = v;
      estimate = await invoke('estimate');
    } catch (e) {
      say(String(e));
    }
  }

  async function saveSettings() {
    await invoke('save_settings', { settings: $state.snapshot(settings) });
    try { estimate = await invoke('estimate'); } catch {}
  }

  async function pickFolder() {
    const dir = await open({
      directory: true,
      title: 'Choose your texture library folder',
      defaultPath: await invoke('exe_dir'),
    });
    if (!dir) return;
    settings.libraryPath = dir;
    await saveSettings();
    await loadLibrary();
  }

  async function download() {
    if (!settings.libraryPath) return pickFolder();
    if (!settings.resolutions.length || !settings.formats.length || !settings.types.length)
      return say('Pick at least one resolution, format and asset type in Settings.');
    busy = true;
    progress = { phase: 'prepare', message: 'Contacting AmbientCG…', filesDone: 0, filesTotal: 0 };
    try {
      const p = await invoke('prepare_sync');
      await loadLibrary();
      progress = null;
      if (p.files === 0) { busy = false; say('Everything is up to date ✓'); }
      else plan = p;
    } catch (e) {
      busy = false; progress = null;
      if (e !== 'cancelled') say(String(e));
    }
  }

  async function startPlan() {
    plan = null;
    try { await invoke('start_download'); }
    catch (e) { busy = false; say(String(e)); }
  }

  function cancelPlan() { plan = null; busy = false; }
  const cancel = () => invoke('cancel_sync');

  onMount(() => {
    let unlisten;
    (async () => {
      settings = await invoke('get_settings');
      await loadLibrary();
      unlisten = await listen('sync-progress', async (e) => {
        const p = e.payload;
        if (['done', 'cancelled', 'error'].includes(p.phase)) {
          progress = null; busy = false;
          say(p.message);
          await loadLibrary();
        } else {
          for (const c of p.completed ?? []) {
            const entry = lib.entries.find((x) => x.id === c.id);
            if (entry && !entry.folders.some((f) => f.rel === c.rel)) entry.folders.push({ attr: c.attr, rel: c.rel });
          }
          progress = p;
        }
      });
    })();
    const key = (ev) => {
      if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 'k') { ev.preventDefault(); searchEl?.focus(); searchEl?.select(); }
      if (ev.key === 'Escape') { if (selected) selected = null; else if (query) query = ''; }
    };
    window.addEventListener('keydown', key);
    return () => { unlisten?.(); window.removeEventListener('keydown', key); };
  });

  const kinds = $derived.by(() => {
    const m = {};
    for (const e of lib.entries) m[e.kind] = (m[e.kind] || 0) + 1;
    return m;
  });
  const categories = $derived.by(() => {
    const s = new Set();
    for (const e of lib.entries) if (typeFilter === 'All' || e.kind === typeFilter) s.add(e.category);
    return [...s].sort();
  });

  const filtered = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const list = lib.entries.filter((e) =>
      (typeFilter === 'All' || e.kind === typeFilter) &&
      (category === 'All' || e.category === category) &&
      (status === 'all' || (status === 'have') === e.folders.length > 0) &&
      words.every((w) => e.hay.includes(w)));
    if (sort === 'name') list.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }));
    else list.sort((a, b) => b.release.localeCompare(a.release));
    return list;
  });

  $effect(() => { query; typeFilter; status; category; sort; limit = 200; });
  $effect(() => { typeFilter; category = 'All'; });

  $effect(() => {
    if (!sentinel) return;
    const io = new IntersectionObserver((en) => { if (en[0].isIntersecting) limit += 200; }, { rootMargin: '600px' });
    io.observe(sentinel);
    return () => io.disconnect();
  });

  const thumb = (e) => convertFileSrc(`${lib.thumbsDir}\\${e.id}.webp`);
  const pct = $derived(progress?.bytesTotal ? (progress.bytesDone / progress.bytesTotal) * 100
      : progress?.filesTotal ? (progress.filesDone / progress.filesTotal) * 100 : 0);
  const eta = $derived(progress?.speed ? (progress.bytesTotal - progress.bytesDone) / progress.speed : 0);
  const active = $derived(new Map((progress?.assets ?? []).map((p) => [p.id, p])));
  const have = $derived(lib.entries.filter((e) => e.folders.length).length);

  function copy(text) { navigator.clipboard?.writeText(text); say('Copied'); }
</script>

{#if settings}
<div class="app">
  <header class="top">
    <div class="brand"><span class="logo">⬇</span> AmbientCG</div>
    <div class="search">
      <svg viewBox="0 0 20 20" width="16" height="16"><circle cx="8.5" cy="8.5" r="5.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M13 13l4 4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
      <input bind:this={searchEl} type="search" bind:value={query} placeholder="Search name, tag, category…" spellcheck="false" />
      <kbd>Ctrl K</kbd>
    </div>
    <button class="icon" onclick={() => (showSettings = true)} aria-label="Settings" title="Settings">⚙</button>
    {#if busy}
      <button class="primary" onclick={cancel}>Cancel</button>
    {:else}
      <button class="primary" onclick={download}>
        Download{#if estimate?.files} · {fmtBytes(estimate.bytes)}{/if}
      </button>
    {/if}
  </header>

  {#if !settings.libraryPath}
    <div class="empty">
      <h1>Welcome</h1>
      <p>Pick a folder for your texture library. An <b>AmbientCG</b> folder will be created inside it.</p>
      <button class="primary big" onclick={pickFolder}>Choose library folder…</button>
    </div>
  {:else if lib.entries.length === 0}
    <div class="empty">
      <h1>Your library is empty</h1>
      <p>Press <b>Download</b> to read the AmbientCG catalogue and fetch the resolutions you ticked in Settings.</p>
      <button class="primary big" onclick={download} disabled={busy}>Download</button>
    </div>
  {:else}
    <div class="filters">
      <div class="seg">
        <button class:on={typeFilter === 'All'} onclick={() => (typeFilter = 'All')}>All</button>
        {#each Object.keys(kinds) as k}
          <button class:on={typeFilter === k} onclick={() => (typeFilter = k)}>{TYPE_LABELS[k] ?? k} <small>{kinds[k]}</small></button>
        {/each}
      </div>
      <div class="spacer"></div>
      <select bind:value={category}>
        <option value="All">All categories</option>
        {#each categories as c}<option value={c}>{c}</option>{/each}
      </select>
      <select bind:value={status}>
        <option value="all">All</option>
        <option value="have">Downloaded ({have})</option>
        <option value="missing">Not downloaded</option>
      </select>
      <select bind:value={sort}>
        <option value="name">Name</option>
        <option value="new">Newest</option>
      </select>
      <span class="count">{filtered.length.toLocaleString()} results</span>
    </div>

    <main class="grid-wrap">
      {#if filtered.length === 0}
        <div class="none">No matches for “{query}”.</div>
      {:else}
        <div class="grid">
          {#each filtered.slice(0, limit) as e (e.id)}
            <button class="card" onclick={() => (selected = e)}>
              <div class="img">
                {#if e.hasThumb}<img src={thumb(e)} alt="" loading="lazy" draggable="false" />{/if}
                {#if e.folders.length}<span class="dot" title="Downloaded"></span>{/if}
                {#if active.has(e.id)}
                  {@const p = active.get(e.id)}
                  <div class="dl"><div class="dlfill" style="width:{p.total ? (p.done / p.total) * 100 : 0}%"></div></div>
                {/if}
              </div>
              <div class="meta">
                <div class="name">{e.name}</div>
                <div class="sub">{e.folders.length ? [...new Set(e.folders.map((f) => f.attr))].join(' · ') : e.category}</div>
              </div>
            </button>
          {/each}
        </div>
        <div bind:this={sentinel} style="height:1px"></div>
      {/if}
    </main>
  {/if}

  {#if progress}
    <footer class="progress">
      <div class="line">
        <b>{progress.phase === 'download' ? 'Downloading' : progress.message}</b>
        {#if progress.phase === 'download'}
          <span>{progress.filesDone.toLocaleString()} / {progress.filesTotal.toLocaleString()} files · {fmtBytes(progress.bytesDone)} / {fmtBytes(progress.bytesTotal)}</span>
          <span class="right">{fmtBytes(progress.speed)}/s · ETA {fmtEta(eta)}{#if progress.failed} · <em>{progress.failed} failed</em>{/if}</span>
        {:else if progress.filesTotal}
          <span>{progress.filesDone.toLocaleString()} / {progress.filesTotal.toLocaleString()}</span>
        {/if}
      </div>
      <div class="bar"><div class="fill" class:indeterminate={!pct && progress.phase !== 'download'} style="width:{pct || (progress.filesTotal ? (progress.filesDone / progress.filesTotal) * 100 : 0)}%"></div></div>
      {#if progress.current?.length}<div class="cur">{progress.current.join('   ')}</div>{/if}
    </footer>
  {/if}

  {#if toast}<div class="toast" role="status">{toast}</div>{/if}

  {#if showSettings}
    <Settings {settings} {estimate} locked={busy} onchange={saveSettings} onpick={pickFolder}
      onopen={() => invoke('open_library').catch((e) => say(String(e)))} onclose={() => (showSettings = false)} />
  {/if}

  {#if plan}
    <div class="scrim" role="presentation"></div>
    <div class="modal">
      <h2>Ready to download</h2>
      <div class="stats">
        <div><b>{plan.files.toLocaleString()}</b><span>files</span></div>
        <div><b>{plan.assets.toLocaleString()}</b><span>assets</span></div>
        <div><b>{fmtBytes(plan.bytes)}</b><span>total</span></div>
      </div>
      <p class="hint">Into <code>{lib.root}</code>. You can cancel any time and resume later — finished files are kept.</p>
      <div class="actions">
        <button class="ghost" onclick={cancelPlan}>Not now</button>
        <button class="primary" onclick={startPlan}>Start download</button>
      </div>
    </div>
  {/if}

  {#if selected}
    <div class="scrim" onclick={() => (selected = null)} role="presentation"></div>
    <div class="modal detail">
      <button class="icon x" onclick={() => (selected = null)} aria-label="Close">✕</button>
      <div class="dimg">{#if selected.hasThumb}<img src={thumb(selected)} alt="" />{/if}</div>
      <div class="dinfo">
        <h2>{selected.name}</h2>
        <div class="hint">{TYPE_LABELS[selected.kind] ?? selected.kind} · {selected.category} · {selected.release.slice(0, 10)}</div>
        <div class="tags">{#each selected.tags as t}<button class="tag" onclick={() => { query = t; selected = null; }}>{t}</button>{/each}</div>
        <h3>On disk</h3>
        {#if selected.folders.length}
          <div class="folders">
            {#each selected.folders as f}
              <button class="ghost" onclick={() => invoke('open_folder', { rel: f.rel }).catch((e) => say(String(e)))}>📂 {f.attr}</button>
            {/each}
          </div>
        {:else}
          <div class="hint">Not downloaded yet.</div>
        {/if}
        <div class="actions left">
          <button class="ghost" onclick={() => copy(selected.link)}>Copy link</button>
        </div>
      </div>
    </div>
  {/if}
</div>
{/if}

<style>
  .app { height: 100%; display: flex; flex-direction: column; position: relative; }
  .top { display: flex; align-items: center; gap: 12px; padding: 12px 20px; border-bottom: 1px solid var(--border); background: var(--surface); }
  .brand { font-weight: 700; font-size: 16px; display: flex; align-items: center; gap: 8px; min-width: 130px; }
  .logo { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 8px; background: var(--accent); color: var(--accent-ink); }
  .search { flex: 1; display: flex; align-items: center; gap: 10px; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; padding: 0 12px; color: var(--muted); }
  .search:focus-within { border-color: var(--accent); color: var(--text); }
  .search input { flex: 1; background: none; border: 0; outline: 0; padding: 10px 0; }
  kbd { font-size: 11px; border: 1px solid var(--border); border-radius: 5px; padding: 1px 6px; color: var(--muted); }
  .primary { background: var(--accent); color: var(--accent-ink); border: 0; font-weight: 700; padding: 9px 18px; border-radius: 10px; white-space: nowrap; }
  .primary:hover:not(:disabled) { filter: brightness(1.08); }
  .primary.big { padding: 12px 26px; font-size: 15px; }
  .ghost { background: transparent; border: 1px solid var(--border); padding: 8px 14px; border-radius: 8px; }
  .ghost:hover { border-color: var(--muted); }
  .icon { background: transparent; border: 0; color: var(--muted); font-size: 18px; padding: 6px 10px; border-radius: 8px; }
  .icon:hover { color: var(--text); background: var(--surface-2); }

  .filters { display: flex; align-items: center; gap: 10px; padding: 12px 20px 8px; }
  .spacer { flex: 1; }
  .seg { display: flex; gap: 4px; flex-wrap: wrap; }
  .seg button { background: transparent; border: 1px solid transparent; padding: 5px 12px; border-radius: 999px; color: var(--muted); }
  .seg button small { opacity: 0.7; margin-left: 3px; }
  .seg button:hover { color: var(--text); }
  .seg button.on { background: var(--surface-2); border-color: var(--border); color: var(--text); }
  select { background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 6px 8px; max-width: 170px; }
  .count { color: var(--muted); font-size: 12px; min-width: 80px; text-align: right; }

  .grid-wrap { flex: 1; overflow-y: auto; padding: 8px 20px 24px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(168px, 1fr)); gap: 14px; }
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: 0; text-align: left; overflow: hidden; transition: transform 0.12s, border-color 0.12s; content-visibility: auto; contain-intrinsic-size: 168px 230px; }
  .card:hover { transform: translateY(-2px); border-color: var(--accent); }
  .img { aspect-ratio: 1; background: var(--surface-2); position: relative; }
  .img img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .dot { position: absolute; top: 8px; right: 8px; width: 10px; height: 10px; border-radius: 50%; background: var(--good); box-shadow: 0 0 0 3px rgba(0, 0, 0, 0.45); }
  .dl { position: absolute; left: 8px; right: 8px; bottom: 8px; height: 6px; border-radius: 99px; background: rgba(0, 0, 0, 0.6); overflow: hidden; }
  .dlfill { height: 100%; background: var(--accent); transition: width 0.3s; }
  .meta { padding: 9px 11px 11px; }
  .name { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--muted); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .none { color: var(--muted); text-align: center; padding: 80px 0; }

  .empty { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; gap: 6px; padding: 40px; }
  .empty h1 { margin: 0; font-size: 26px; }
  .empty p { color: var(--muted); max-width: 440px; margin: 0 0 18px; }

  .progress { border-top: 1px solid var(--border); background: var(--surface); padding: 10px 20px 12px; }
  .line { display: flex; gap: 16px; align-items: baseline; margin-bottom: 8px; }
  .line span { color: var(--muted); }
  .line .right { margin-left: auto; }
  .line em { color: var(--bad); font-style: normal; }
  .bar { height: 6px; background: var(--surface-2); border-radius: 99px; overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 99px; transition: width 0.3s; }
  .fill.indeterminate { width: 30% !important; animation: slide 1.2s infinite ease-in-out; }
  @keyframes slide { from { margin-left: -30%; } to { margin-left: 100%; } }
  .cur { margin-top: 6px; color: var(--muted); font-size: 11px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .toast { position: absolute; left: 50%; bottom: 24px; transform: translateX(-50%); background: var(--surface-2); border: 1px solid var(--border); padding: 10px 18px; border-radius: 10px; box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5); max-width: 80%; z-index: 40; }

  .scrim { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55); z-index: 30; }
  .modal { position: fixed; z-index: 31; left: 50%; top: 50%; transform: translate(-50%, -50%); width: min(460px, 92vw); background: var(--surface); border: 1px solid var(--border); border-radius: 16px; padding: 24px; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6); }
  .modal h2 { margin: 0 0 14px; }
  .stats { display: flex; gap: 12px; }
  .stats div { flex: 1; background: var(--bg); border-radius: 10px; padding: 12px; display: flex; flex-direction: column; }
  .stats b { font-size: 20px; }
  .stats span, .hint { color: var(--muted); font-size: 12px; }
  .hint { margin: 12px 0 0; }
  .hint code { background: var(--bg); padding: 1px 5px; border-radius: 4px; word-break: break-all; }
  .actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 20px; }
  .actions.left { justify-content: flex-start; }
  .detail { width: min(680px, 94vw); display: flex; gap: 22px; }
  .dimg { width: 240px; aspect-ratio: 1; flex: none; background: var(--surface-2); border-radius: 12px; overflow: hidden; }
  .dimg img { width: 100%; height: 100%; object-fit: cover; }
  .dinfo { flex: 1; min-width: 0; }
  .dinfo h2 { margin-bottom: 0; }
  .dinfo h3 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin: 18px 0 8px; }
  .tags, .folders { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
  .folders { margin-top: 0; }
  .tag { background: var(--surface-2); border: 1px solid var(--border); border-radius: 999px; padding: 2px 10px; font-size: 12px; color: var(--muted); }
  .tag:hover { color: var(--text); border-color: var(--accent); }
  .x { position: absolute; top: 10px; right: 10px; }
</style>
