import { jsonInit, request } from './core';
import { link } from '../nav';
import { inTauri } from '../mac-bridge';

export interface DocumentSummary {
  id: string;
  filename: string;
  media_type: string;
  byte_len: number;
  imported_at: string;
  status: 'inbox' | 'filed' | 'archived';
  category: string | null;
  project: string | null;
  extracted_title: string | null;
  extractor: string | null;
  extracted_chars: number;
  suggested_category: string | null;
  suggestion_reason: string | null;
  data_class: 'c2';
}

export interface DocumentDetail {
  document: DocumentSummary;
  text: string | null;
}

interface DocumentPage {
  documents: DocumentSummary[];
  total: number;
  offset: number;
  limit: number;
}

const path = (id: string) => `/documents/api/documents/${encodeURIComponent(id)}`;

export const documents = {
  /** Metadata only; page through the capability without fetching document text. */
  async list(): Promise<DocumentSummary[]> {
    const rows: DocumentSummary[] = [];
    let total = 1;
    while (rows.length < total) {
      const page = await request<DocumentPage>(`/documents/api/documents?limit=500&offset=${rows.length}`);
      rows.push(...page.documents);
      total = page.total;
      if (page.documents.length === 0) break;
    }
    return rows;
  },
  detail: (id: string) => request<DocumentDetail>(path(id)),
  review: (id: string, category: string, project: string | null) =>
    request<DocumentSummary>(path(id), jsonInit('PATCH', { category, project })),
  archive: (id: string) => request<DocumentSummary>(path(id), { method: 'DELETE' }),
  originalUrl: (id: string, download = false) => link(`${path(id)}/original${download ? '?download=true' : ''}`),
  async upload(file: File): Promise<{ document: DocumentSummary; created: boolean }> {
    if (inTauri()) throw new Error('Open Sjel in your browser to upload a document.');
    return request(`/documents/api/documents?filename=${encodeURIComponent(file.name)}`, {
      method: 'POST', body: file, headers: { 'Content-Type': 'application/octet-stream' },
    });
  },
};
