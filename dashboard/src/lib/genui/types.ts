/**
 * Declarative Generative UI Component Specifications.
 *
 * Implements Sjel's Product Rule 6 ("Screens and assistant tools come from the typed data")
 * and OpenUI / Intelligent UI declarative widgets. Models and project notes emit structured
 * data specifications, which Sjel renders as interactive Svelte 5 components with live state.
 */

export interface MetricItem {
  label: string;
  value: string | number;
  unit?: string;
  status?: 'nominal' | 'good' | 'warning' | 'critical' | 'idle' | 'active';
  delta?: string;
  hint?: string;
}

export interface MetricGridSpec {
  type: 'metric-grid';
  title?: string;
  subtitle?: string;
  columns?: number;
  items: MetricItem[];
}

export interface DecisionCriterion {
  id: string;
  name: string;
  weight: number; // 1 to 5
  description?: string;
}

export interface DecisionOption {
  id: string;
  name: string;
  description?: string;
  /** Scores per criterion ID (1 to 10) */
  scores: Record<string, number>;
  tag?: string;
}

export interface DecisionMatrixSpec {
  type: 'decision-matrix';
  title?: string;
  description?: string;
  criteria: DecisionCriterion[];
  options: DecisionOption[];
}

export interface SimulatorInput {
  id: string;
  label: string;
  type: 'slider' | 'segmented';
  min?: number;
  max?: number;
  step?: number;
  default: number | string;
  unit?: string;
  options?: Array<{ label: string; value: string | number; multiplier?: number }>;
}

export interface SimulatorOutput {
  id: string;
  label: string;
  unit?: string;
  formula: string; // e.g. "load * temp * 0.015"
  format?: 'number' | 'currency' | 'percent';
  decimals?: number;
}

export interface ParameterSimulatorSpec {
  type: 'simulator';
  title?: string;
  description?: string;
  inputs: SimulatorInput[];
  outputs: SimulatorOutput[];
}

export interface StatusTimelineStep {
  label: string;
  date?: string;
  status: 'done' | 'current' | 'pending' | 'blocked';
  detail?: string;
}

export interface StatusTimelineSpec {
  type: 'status-timeline';
  title?: string;
  steps: StatusTimelineStep[];
}

export interface RawSpec {
  type: 'raw';
  raw: string;
  error?: string;
}

export type GenUISpec =
  | MetricGridSpec
  | DecisionMatrixSpec
  | ParameterSimulatorSpec
  | StatusTimelineSpec
  | RawSpec;
