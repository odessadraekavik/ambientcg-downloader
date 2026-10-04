export function fmtBytes(n) {
  if (!n) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return `${v >= 100 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function fmtEta(seconds) {
  if (!isFinite(seconds) || seconds <= 0) return '—';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${Math.floor(seconds % 60)}s`;
  return `${Math.ceil(seconds)}s`;
}

export const TYPE_LABELS = {
  Material: 'Materials',
  HDRI: 'HDRIs',
  Decal: 'Decals',
  Atlas: 'Atlases',
  Terrain: 'Terrains',
  Brush: 'Brushes',
  '3DModel': '3D Models',
  Substance: 'Substances',
};

export const ALL_TYPES = Object.keys(TYPE_LABELS);
export const ALL_RESOLUTIONS = ['1K', '2K', '4K', '8K', '12K', '16K'];
export const ALL_FORMATS = ['JPG', 'PNG'];
