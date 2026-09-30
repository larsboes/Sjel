/**
 * Sjel Live Decision Engine & System-1 Model Manager
 *
 * Dual-mechanism architecture:
 *   Mechanism 1 (Current / Baseline): Zero-memory, instant keyword routing and 5-factor heuristic scoring.
 *   Mechanism 2 (Contrastive Decision): Contrastive-LM / Qwen3-8B System-1 action scoring via local Ollama.
 *
 * Designed with Apple-like simplicity: auto-detects host hardware, selects optimal quantization,
 * provides one-tap model pulling with real-time stream progress, and gracefully degrades to Mechanism 1.
 */

export interface ModelDetail {
  name: string;
  size: number;
  quantization?: string;
  family?: string;
}

export interface HardwareInfo {
  chip: string;
  totalMemoryGb: number;
  os: string;
}

export type DecisionMechanism = 'mechanism1' | 'mechanism2';

const OLLAMA_BASE = 'http://127.0.0.1:11434';
const STORAGE_KEY = 'sjel-decision-mechanism';

// In non-compiled test runners (e.g. Bun test), $state rune is not defined globally.
//
// The probe is not `typeof` alone: a browser DOES define `globalThis.$state`, as a getter
// Svelte installs to reject the rune outside a compiled file, and reading it throws
// rune_outside_svelte — which took the whole dashboard down in dev, `/` and `/systems` both
// rendering SvelteKit's 500 page. So a read that throws means a real rune is present, and
// only a silent `undefined` means the stand-in is needed.
try {
  if (typeof (globalThis as any).$state === 'undefined') {
    (globalThis as any).$state = <T>(val: T): T => val;
  }
} catch {
  // A browser: the rune answered by refusing to be read.
}

class DecisionEngineStore {
  ollamaRunning = $state(false);
  installedModels = $state<ModelDetail[]>([]);
  busy = $state(false);
  isPulling = $state(false);
  pullProgress = $state(0);
  pullStatusText = $state('');
  error = $state<string | null>(null);

  // Selected mechanism: defaults to Mechanism 1 until user enables Mechanism 2
  mechanism = $state<DecisionMechanism>('mechanism1');

  // Node hardware profile (Apple M4 Pro 24GB on this node)
  hardware: HardwareInfo = {
    chip: 'Apple M4 Pro',
    totalMemoryGb: 24,
    os: 'macOS',
  };

  constructor() {
    if (typeof localStorage !== 'undefined') {
      const saved = localStorage.getItem(STORAGE_KEY) as DecisionMechanism | null;
      if (saved === 'mechanism1' || saved === 'mechanism2') {
        this.mechanism = saved;
      }
    }
  }

  /** Optimal quantization tag recommended for this machine's memory profile */
  get recommendedTag(): string {
    const gb = this.hardware.totalMemoryGb;
    if (gb <= 16) return 'qwen3:8b-q4_k_m';
    if (gb <= 32) return 'qwen3:8b-q5_k_m';
    return 'qwen3:8b-q8_0';
  }

  /** Estimated memory footprint for the recommended model in GB */
  get recommendedVramGb(): number {
    const gb = this.hardware.totalMemoryGb;
    if (gb <= 16) return 4.9;
    if (gb <= 32) return 5.8;
    return 8.5;
  }

  /** Check if the recommended model (or any qwen3:8b variant) is installed */
  get hasDecisionModel(): boolean {
    return this.installedModels.some(
      (m) => m.name.startsWith('qwen3:8b') || m.name === this.recommendedTag
    );
  }

  /** Mechanism 2 is truly active only if chosen, Ollama is up, and model exists */
  get isMechanism2Active(): boolean {
    return this.mechanism === 'mechanism2' && this.ollamaRunning && this.hasDecisionModel;
  }

  setMechanism(m: DecisionMechanism): void {
    this.mechanism = m;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(STORAGE_KEY, m);
    }
  }

  /** Poll Ollama loopback status and installed models */
  async refresh(): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      const res = await fetch(`${OLLAMA_BASE}/api/tags`, {
        signal: AbortSignal.timeout(3000),
      });
      if (!res.ok) {
        this.ollamaRunning = false;
        this.installedModels = [];
        return;
      }
      const data = await res.json();
      this.ollamaRunning = true;
      this.installedModels = Array.isArray(data?.models)
        ? data.models.map((m: Record<string, unknown>) => ({
            name: String(m.name || ''),
            size: Number(m.size || 0),
            quantization: String((m.details as Record<string, unknown>)?.quantization_level || ''),
            family: String((m.details as Record<string, unknown>)?.family || ''),
          }))
        : [];
    } catch {
      this.ollamaRunning = false;
      this.installedModels = [];
    } finally {
      this.busy = false;
    }
  }

  /**
   * One-tap installation: stream-pulls the recommended model directly into local Ollama.
   * Displays real-time progress in the Apple-style sheet.
   */
  async installRecommendedModel(): Promise<void> {
    const model = this.recommendedTag;
    this.isPulling = true;
    this.pullProgress = 0;
    this.pullStatusText = 'Connecting to Ollama...';
    this.error = null;

    try {
      const res = await fetch(`${OLLAMA_BASE}/api/pull`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: model, stream: true }),
      });

      if (!res.ok || !res.body) {
        throw new Error(`Ollama refused pull request (HTTP ${res.status})`);
      }

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() ?? '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed) continue;
          try {
            const chunk = JSON.parse(trimmed);
            if (chunk.status) {
              this.pullStatusText = chunk.status;
            }
            if (chunk.total && chunk.completed) {
              const pct = Math.round((chunk.completed / chunk.total) * 100);
              this.pullProgress = Math.min(Math.max(pct, 0), 100);
              const mbDone = Math.round(chunk.completed / (1024 * 1024));
              const mbTotal = Math.round(chunk.total / (1024 * 1024));
              this.pullStatusText = `Downloading: ${this.pullProgress}% (${mbDone} MB / ${mbTotal} MB)`;
            }
          } catch {
            // Ignore partial JSON chunks
          }
        }
      }

      this.pullProgress = 100;
      this.pullStatusText = 'Model ready!';
      await this.refresh();
      // Auto-activate Mechanism 2 upon successful download
      this.setMechanism('mechanism2');
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
    } finally {
      this.isPulling = false;
    }
  }

  /**
   * Fast candidate scoring for travel options or intent classification (Mechanism 2).
   * Automatically returns null if offline so callers immediately fall back to Mechanism 1.
   */
  async scoreCandidate(context: string, candidate: string): Promise<number | null> {
    if (!this.isMechanism2Active) return null;

    try {
      const res = await fetch(`${OLLAMA_BASE}/api/embed`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model: this.recommendedTag,
          input: [context, candidate],
        }),
        signal: AbortSignal.timeout(1500),
      });

      if (!res.ok) return null;
      const data = await res.json();
      const v1 = data?.embeddings?.[0];
      const v2 = data?.embeddings?.[1];
      if (!Array.isArray(v1) || !Array.isArray(v2) || v1.length !== v2.length) return null;

      // Scaled dot product
      let dot = 0;
      for (let i = 0; i < v1.length; i++) {
        dot += v1[i] * v2[i];
      }
      return dot;
    } catch {
      return null;
    }
  }
}

export const decisionEngine = new DecisionEngineStore();
