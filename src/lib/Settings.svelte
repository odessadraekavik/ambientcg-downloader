<script>
  import { ALL_FORMATS, ALL_RESOLUTIONS, ALL_TYPES, TYPE_LABELS, fmtBytes } from './format.js';

  let { settings, estimate, locked, onchange, onpick, onopen, onlog, onclose } = $props();

  function toggle(list, value) {
    const i = list.indexOf(value);
    if (i >= 0) list.splice(i, 1);
    else list.push(value);
    onchange();
  }

  const heavy = (r) => ['8K', '12K', '16K'].includes(r);
</script>

<div class="scrim" onclick={onclose} role="presentation"></div>
<aside class="drawer">
  <header>
    <h2>Settings</h2>
    <button class="icon" onclick={onclose} aria-label="Close settings">✕</button>
  </header>

  <div class="body">
    <section>
      <h3>Library folder</h3>
      <div class="path" title={settings.libraryPath}>{settings.libraryPath || 'Not chosen yet'}</div>
      <div class="row">
        <button class="btn" onclick={onpick} disabled={locked}>Change…</button>
        <button class="btn ghost" onclick={onopen} disabled={!settings.libraryPath}>Open in Explorer</button>
      </div>
      <div class="row" style="margin-top:8px">
        <button class="btn ghost" onclick={onlog}>Show log file</button>
      </div>
      <p class="hint">Files go into an <b>AmbientCG</b> folder inside it, sorted as <code>Type / Resolution / Category / Asset</code>.</p>
    </section>

    <section>
      <h3>Resolutions</h3>
      <div class="chips">
        {#each ALL_RESOLUTIONS as r}
          <label class="chip" class:on={settings.resolutions.includes(r)}>
            <input type="checkbox" checked={settings.resolutions.includes(r)} disabled={locked}
              onchange={() => toggle(settings.resolutions, r)} />
            {r}{#if heavy(r)}<span class="warn" title="Very large downloads">•</span>{/if}
          </label>
        {/each}
      </div>
    </section>

    <section>
      <h3>Formats</h3>
      <div class="chips">
        {#each ALL_FORMATS as f}
          <label class="chip" class:on={settings.formats.includes(f)}>
            <input type="checkbox" checked={settings.formats.includes(f)} disabled={locked}
              onchange={() => toggle(settings.formats, f)} />
            {f}
          </label>
        {/each}
      </div>
      <p class="hint">JPG is much smaller. PNG is lossless.</p>
    </section>

    <section>
      <h3>Asset types</h3>
      <div class="chips">
        {#each ALL_TYPES as t}
          <label class="chip" class:on={settings.types.includes(t)}>
            <input type="checkbox" checked={settings.types.includes(t)} disabled={locked}
              onchange={() => toggle(settings.types, t)} />
            {TYPE_LABELS[t]}
          </label>
        {/each}
      </div>
    </section>

    <section>
      <h3>Parallel downloads <span class="val">{settings.concurrency}</span></h3>
      <input type="range" min="1" max="8" bind:value={settings.concurrency} onchange={onchange} disabled={locked} />
      <p class="hint">Be kind to AmbientCG's servers — 3 to 4 is plenty.</p>
    </section>

    <section class="estimate">
      <h3>Still to download</h3>
      {#if estimate}
        <div class="big">{fmtBytes(estimate.bytes)}</div>
        <div class="hint">{estimate.files.toLocaleString()} files · {estimate.assets.toLocaleString()} assets</div>
      {:else}
        <div class="hint">Press <b>Download</b> once to read the catalogue and see the total.</div>
      {/if}
    </section>
  </div>
</aside>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 20; }
  .drawer {
    position: fixed; top: 0; right: 0; bottom: 0; width: 380px; z-index: 21;
    background: var(--surface); border-left: 1px solid var(--border);
    display: flex; flex-direction: column; animation: slide 0.18s ease-out;
  }
  @keyframes slide { from { transform: translateX(40px); opacity: 0; } }
  header { display: flex; align-items: center; justify-content: space-between; padding: 16px 20px; border-bottom: 1px solid var(--border); }
  h2 { margin: 0; font-size: 16px; }
  .body { padding: 8px 20px 24px; overflow-y: auto; }
  section { padding: 14px 0; border-bottom: 1px solid var(--border); }
  section:last-child { border-bottom: 0; }
  h3 { margin: 0 0 10px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); display: flex; justify-content: space-between; }
  .val { color: var(--text); }
  .path { background: var(--bg); border: 1px solid var(--border); border-radius: 8px; padding: 8px 10px; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; margin-bottom: 10px; }
  .row { display: flex; gap: 8px; }
  .hint { color: var(--muted); font-size: 12px; margin: 8px 0 0; }
  code { background: var(--bg); padding: 1px 5px; border-radius: 4px; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { position: relative; padding: 6px 12px; border-radius: 999px; border: 1px solid var(--border); background: var(--surface-2); cursor: pointer; transition: 0.12s; }
  .chip input { position: absolute; opacity: 0; pointer-events: none; }
  .chip.on { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); font-weight: 600; }
  .chip:has(input:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .chip:has(input:disabled) { opacity: 0.55; cursor: default; }
  .warn { margin-left: 3px; color: #ffd27a; }
  .chip.on .warn { color: #7a2f00; }
  input[type='range'] { width: 100%; accent-color: var(--accent); }
  .estimate .big { font-size: 28px; font-weight: 700; }
  .btn { background: var(--surface-2); border: 1px solid var(--border); padding: 7px 14px; border-radius: 8px; }
  .btn:hover:not(:disabled) { border-color: var(--muted); }
  .btn.ghost { background: transparent; }
  .icon { background: transparent; border: 0; color: var(--muted); font-size: 16px; padding: 4px 8px; border-radius: 6px; }
  .icon:hover { color: var(--text); background: var(--surface-2); }
</style>
