import { jsonInit, request } from './api';

export type RuntimeSelection = 'auto' | 'normal' | 'on-the-go';
export type RuntimeCategory = 'other-local-models' | 'remote-models' | 'bulk-indexing' | 'transcription' | 'media-conversion';
export interface RuntimeStatus {
  device: string;
  selection: RuntimeSelection;
  effective: Exclude<RuntimeSelection, 'auto'>;
  power: 'ac' | 'battery' | 'no-battery' | 'unknown';
  power_fresh: boolean;
  allow: RuntimeCategory[];
  revision: number;
  detail: string | null;
  configured: boolean;
}
export interface RuntimeUpdate {
  selection?: RuntimeSelection;
  allow?: RuntimeCategory[];
  expected_revision?: number;
}

export const RUNTIME_MODES: Record<RuntimeSelection, string> = {
  auto: 'Auto', normal: 'Normal', 'on-the-go': 'On the go',
};
export const RUNTIME_CATEGORIES: Record<RuntimeCategory, string> = {
  'other-local-models': 'Other local models',
  'remote-models': 'Remote models',
  'bulk-indexing': 'Bulk indexing',
  transcription: 'Transcription',
  'media-conversion': 'Media conversion',
};
export const POWER_LABELS: Record<RuntimeStatus['power'], string> = {
  ac: 'AC', battery: 'Battery', 'no-battery': 'No battery', unknown: 'Power unknown',
};
const API = '/sjel-status/api/sjel-status/runtime';
export const runtime = {
  status: () => request<RuntimeStatus>(API),
  update: (update: RuntimeUpdate) => request<RuntimeStatus>(API, jsonInit('POST', update)),
};

export function runtimeView(status: RuntimeStatus) {
  const onTheGo = status.effective === 'on-the-go';
  const exceptions = onTheGo ? status.allow.map((category) => RUNTIME_CATEGORIES[category]) : [];
  const afmOnly = onTheGo && !status.allow.includes('other-local-models') && !status.allow.includes('remote-models');
  const source = !status.configured ? 'Not configured' : status.selection === 'auto'
    ? `Auto · ${POWER_LABELS[status.power]}${status.power_fresh ? '' : ' (stale)'}`
    : 'Manual';
  const label = RUNTIME_MODES[status.effective];
  const summary = `${label} · ${source}${afmOnly ? ' · AFM only' : ''} · ${exceptions.length ? `Allowed: ${exceptions.join(', ')}` : 'No active exceptions'}`;
  return { onTheGo, afmOnly, source, label, exceptions, summary };
}
