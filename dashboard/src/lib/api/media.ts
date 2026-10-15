import { request } from './core';

/** One mounted volume the ledger knows: how many of its paths are indexed and how many bytes. */
export interface MediaVolume {
  uuid: string;
  label: string;
  locations: number;
  bytes: number;
  /** When a verb last saw this volume, in the store's canonical stamp. */
  last_seen: string;
}

export interface MediaStatus {
  /** Distinct digests the ledger holds. */
  files: number;
  /** Indexed paths — one digest can sit at several. */
  locations: number;
  volumes: MediaVolume[];
}

/** What an ingest run did with one path. `refused` is the one a count cannot explain. */
export type MediaDisposition = 'duplicate' | 'imported' | 'refused' | 'failed' | 'resumed';

export interface MediaIngestRun {
  id: number;
  source: string;
  staging: string;
  started: string;
  /** Null while the run is still going. */
  finished: string | null;
  considered: number;
  imported: number;
  duplicates: number;
  refused: number;
  failed: number;
  resumed: number;
  verified: number;
}

export interface MediaIngestItem {
  relpath: string;
  digest: string | null;
  size: number | null;
  disposition: MediaDisposition;
  reason: string;
  imported_relpath: string | null;
  verified: boolean;
}

/** One run and the disposition of every path it considered. */
export interface MediaIngestDetail {
  run: MediaIngestRun;
  items: MediaIngestItem[];
}

export const media = {
  status: () => request<MediaStatus>('/media/api/status'),
  ingests: () => request<{ runs: MediaIngestRun[] }>('/media/api/ingests'),
  ingest: (id: number) => request<MediaIngestDetail>(`/media/api/ingests/${id}`),
};
