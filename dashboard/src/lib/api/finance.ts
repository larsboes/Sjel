import { jsonInit, request } from './core';

export type BillingCycle = 'weekly' | 'monthly' | 'quarterly' | 'yearly' | 'one_off';

export type SubscriptionState = 'considering' | 'trial' | 'active' | 'covered' | 'paused' | 'cancelled';

/** Append-only. A price change adds one of these; it never edits the one before. */
export interface PricePoint {
  valid_from: string;
  amount_cents: number;
  currency: string;
  cycle: BillingCycle;
  /** Which tier: "Pro", "Max", "2TB". Absent for a subscription with only one. */
  plan?: string | null;
  reason: string;
}

/** Append-only, same reasoning. `paused → active` is two rows, not an edit. */
export interface StateChange {
  effective: string;
  state: SubscriptionState;
  note: string;
}

export interface Subscription {
  id: string;
  name: string;
  source_path: string;
  category: string | null;
  value_rating: number | null;
  prices: PricePoint[];
  states: StateChange[];
}

/** Computed from the series at a date, never read from a stored total. */
export interface Burn {
  at: string;
  currencies: Array<{
    currency: string;
    monthly_cents: number;
    annual_cents: number;
  }>;
  billing_count: number;
  covered_count: number;
  unknown_price_count: number;
  total_count: number;
}

export interface WritebackResult {
  ok: boolean;
  written: number;
  unchanged: number;
  conflicts: string[];
  not_imported: string[];
  /** Subscriptions with no note in the vault, written as whole generated files
   *  under Resources/Sjel/ instead (PRD Q31). Since the 2026-08-23 vault
   *  reorganisation moved every finance note to the overlay, this is where the
   *  price and state series actually land. */
  projected: {
    created: number;
    updated: number;
    unchanged: number;
    refused: string[];
    removed: string[];
  };
}

export type FinanceTransactionKind = 'income' | 'expense' | 'transfer';

export type SpendingPurpose = 'day_to_day' | 'trip' | 'work' | 'housing' | 'other';

export type CandidateState = 'pending' | 'confirmed' | 'rejected' | 'duplicate';

export type CsvDateFormat =
  | 'iso_year_month_day'
  | 'day_month_year_dots'
  | 'day_month_year_slashes';

export interface TransactionCandidate {
  id: string;
  fingerprint: string;
  booked_at: string;
  description: string;
  amount_cents: number;
  currency: string;
  source_account: string;
  source_reference: string | null;
  proposed_account: string;
  confidence_basis_points: number;
  state: CandidateState;
  transfer_match_ids: string[];
}

export interface CsvMapping {
  delimiter: string;
  decimal_separator: string;
  date_column: string;
  amount_column: string;
  description_column: string;
  categorization_columns: string[];
  reference_column?: string | null;
  currency_column?: string | null;
  default_currency: string;
  source_account: string;
  default_outflow_account: string;
  default_inflow_account: string;
  categorization_rules: CsvCategorizationRule[];
  row_filter?: CsvRowFilter | null;
  amount_sign: 'as_provided' | 'invert';
  amount_rounding?: 'reject' | 'half_away_from_zero';
  date_formats: CsvDateFormat[];
  row_policy: 'strict' | 'required_fields';
}

export interface CsvCategorizationRule {
  description_contains_any: string[];
  description_starts_with_any: string[];
  field_equals_any?: CsvFieldEquals[];
  direction: 'any' | 'outflow' | 'inflow';
  account: string;
  confidence_basis_points: number;
}

export interface CsvFieldEquals {
  column: string;
  values: string[];
}

export interface CsvRowFilter {
  column: string;
  include_values: string[];
}

export interface CsvImportPreview {
  preview_id: string;
  candidate_count: number;
  duplicate_rows: number;
  preserved_repetitions: number;
  ignored_non_transaction_rows: number;
  outflow_count: number;
  inflow_count: number;
}

export interface CsvMappingProfile {
  label: string;
  mapping: CsvMapping;
}

export interface InvestmentCsvMapping {
  delimiter: string;
  decimal_separator: string;
  date_column: string;
  instrument_column: string;
  quantity_column: string;
  activity_type_column?: string | null;
  position_activity_values: string[];
  non_position_activity_values: string[];
  reference_column?: string | null;
  price_column?: string | null;
  currency_column?: string | null;
  default_currency: string;
  instrument_aliases: Record<string, string>;
}

export interface InvestmentCsvMappingProfile {
  source_key: string;
  label: string;
  coverage: HoldingsCoverage;
  mapping: InvestmentCsvMapping;
}

export type HoldingsCoverage = 'complete' | 'partial';

export interface InvestmentHolding {
  instrument: string;
  quantity: { mantissa: string; scale: number };
  latest_unit_price: { mantissa: string; scale: number } | null;
  currency: string;
}

export interface InvestmentPreview {
  snapshot_id: string;
  activity_count: number;
  duplicate_rows: number;
  ignored_non_position_rows: number;
  closed_positions: number;
  holdings: InvestmentHolding[];
}

export interface ReviewedHoldingsSnapshot {
  schema_version: number;
  snapshot_id: string;
  reviewed_at: string;
  coverage: HoldingsCoverage;
  holdings: InvestmentHolding[];
  sources: Array<{
    source_key: string;
    snapshot_id: string;
    reviewed_at: string;
    coverage: HoldingsCoverage;
  }>;
}

export interface PortfolioValuation {
  currency: string;
  value: { mantissa: string; scale: number };
  priced_holdings: number;
  unpriced_holdings: number;
}

export interface FinanceTransaction {
  id: string;
  date: string;
  description: string;
  kind: FinanceTransactionKind;
  account: string;
  category: string;
  amount_cents: number;
  currency: string;
  source_id: string | null;
  purpose: SpendingPurpose | null;
  trip_id: string | null;
  cash_amount_cents: number;
  shared_cents: number;
  reimbursement_for: string | null;
}

export interface SharedExpenseSummary {
  source_id: string;
  candidate_id: string;
  date: string;
  description: string;
  account: string;
  category: string;
  purpose: SpendingPurpose | null;
  trip_id: string | null;
  gross_cents: number;
  personal_cents: number;
  shared_cents: number;
  reimbursed_cents: number;
  outstanding_cents: number;
  currency: string;
}

/** Per-trip cost roll-up; `trip_id` is the trips plan id (tag `sjel-trip-id`). */
export interface TripSpendingSummary {
  trip_id: string;
  personal_spending_cents: number;
  gross_cash_outflow_cents: number;
  reimbursed_cents: number;
  outstanding_cents: number;
  expense_posting_count: number;
}

export interface FinanceDashboard {
  summary: {
    income_cents: number;
    personal_spending_cents: number;
    gross_cash_outflow_cents: number;
    reimbursement_received_cents: number;
    personal_result_cents: number;
    external_cash_inflow_cents: number;
    external_cash_movement_cents: number;
    savings_rate_percent: number | null;
    currency: string;
  };
  quality: {
    expense_posting_count: number;
    categorized_expense_posting_count: number;
    personal_spending_cents: number;
    categorized_personal_spending_cents: number;
    categorization_count_percent: number | null;
    categorization_value_percent: number | null;
    first_transaction_date: string | null;
    latest_transaction_date: string | null;
    observed_months: number;
    expected_months: number;
  };
  trend: Array<{
    month: string;
    income_cents: number;
    personal_spending_cents: number;
    gross_cash_outflow_cents: number;
    reimbursement_received_cents: number;
    personal_result_cents: number;
    external_cash_inflow_cents: number;
    external_cash_movement_cents: number;
  }>;
  category_trend: Array<{
    month: string;
    category: string;
    amount_cents: number;
  }>;
  transactions: FinanceTransaction[];
  sankey: Array<{
    month: string;
    source: string;
    target: string;
    amount_cents: number;
    account: string;
    category: string;
  }>;
  accounts: string[];
  categories: string[];
  investment: ReviewedHoldingsSnapshot | null;
  portfolio_values: PortfolioValuation[];
  shared_expenses: SharedExpenseSummary[];
  purpose_spending: Array<{
    purpose: SpendingPurpose | null;
    personal_spending_cents: number;
    expense_posting_count: number;
  }>;
  trip_spending: TripSpendingSummary[];
  balance_snapshot: ManualBalanceSnapshot | null;
  tracked_net_worth: TrackedNetWorth | null;
  source_freshness: Array<{
    source: string;
    label: string;
    as_of: string | null;
    age_days: number | null;
    freshness: 'current' | 'stale' | 'missing';
    coverage: 'complete' | 'partial' | 'missing';
  }>;
  commitment_as_of: string;
  current_commitment_monthly_cents: number;
  commitments: RecurringCommitment[];
  planning: FinancePlanningReport;
}

export interface FinancePlanningReport {
  as_of: string;
  currency: string;
  baseline: {
    months: string[];
    monthly_income_cents: number;
    monthly_spending_cents: number;
    forecast_base_cents: number;
    monthly_result_cents: number;
    savings_rate_percent: number | null;
    behavior: Array<{ behavior: 'fixed' | 'variable' | 'discretionary' | 'exceptional' | 'unclassified'; monthly_cents: number }>;
    classified_value_percent: number | null;
  };
  forecasts: Array<{
    as_of: string;
    historical_base_cents: number;
    commitments_cents: number;
    subscriptions_cents: number;
    adjustments_cents: number;
    projected_spending_cents: number;
    projected_result_cents: number;
    savings_rate_percent: number | null;
  }>;
  liquidity: {
    currency: string;
    liquid_assets_cents: number;
    liabilities_cents: number;
    invested_cents: number | null;
    net_worth_cents: number | null;
    cash_share_percent: number | null;
    largest_priced_holding_percent: number | null;
    runway_months: number | null;
    target_cash_cents: number;
    cash_buffer_cents: number;
    complete: boolean;
  } | null;
  subscriptions: {
    monthly_cents: number;
    annual_cents: number;
    billing_count: number;
    covered_count: number;
    unknown_price_count: number;
    anomalies: Array<{
      subscription_id: string;
      subscription_name: string;
      kind: string;
      detail: string;
    }>;
  };
  card_decision: {
    annual_eligible_spend_cents: number;
    annual_fx_spend_cents: number;
    usage_reviewed: boolean;
    spend_source: 'manual' | 'reviewed_transactions';
    spend_period_start: string | null;
    spend_period_end: string | null;
    provisional: boolean;
    options: Array<{
      id: string;
      label: string;
      currency: string;
      annual_fee_cents: number;
      annual_face_value_cents: number;
      annual_benefit_value_cents: number;
      annual_unvalued_face_value_cents: number;
      annual_reward_value_cents: number;
      annual_fx_cost_cents: number;
      annual_net_value_cents: number;
      break_even_eligible_spend_cents: number | null;
      points_per_currency_unit_milli: number;
      point_value_milli_cents: number;
      point_value_assumption: string;
      terms_checked_on: string;
      source_urls: string[];
      benefits: Array<{
        id: string;
        label: string;
        annual_face_value_cents: number;
        annual_personal_value_cents: number;
      }>;
    }>;
  } | null;
  loyalty: Array<{
    id: string;
    label: string;
    points: number;
    point_value_milli_cents: number;
    estimated_value_cents: number;
    transferable: boolean;
    assumption: string;
    as_of: string | null;
    expires_on: string | null;
    transfer_path: string | null;
    source_urls: string[];
  }>;
  caveats: string[];
}

export interface RecurringCommitment {
  id: string;
  label: string;
  account: string;
  monthly_cents: number;
  currency: string;
  valid_from: string;
  valid_until: string | null;
}

export type BalanceCoverage = 'complete' | 'partial';

export type BalanceKind = 'asset' | 'liability';

export interface ManualBalance {
  id: string;
  label: string;
  kind: BalanceKind;
  amount_cents: number;
}

export interface ManualBalanceSnapshot {
  schema_version: number;
  as_of: string;
  updated_at: string;
  currency: string;
  coverage: BalanceCoverage;
  balances: ManualBalance[];
}

export interface ManualBalanceUpdate {
  as_of: string;
  currency: string;
  coverage: BalanceCoverage;
  balances: ManualBalance[];
}

export interface TrackedNetWorth {
  currency: string;
  value: { mantissa: string; scale: number };
  manual_balance_cents: number;
  portfolio_included: boolean;
  complete: boolean;
}

export const finance = {
  subscriptions: (signal?: AbortSignal) =>
    request<Subscription[]>('/finance/api/subscriptions', signal ? { signal } : undefined),
  /** `at` is the whole point: the price series makes "what will this cost in
   *  October" a different answer from "what does it cost today". */
  burn: (at?: string, signal?: AbortSignal) =>
    request<Burn>(
      `/finance/api/subscriptions/burn${at ? `?at=${encodeURIComponent(at)}` : ''}`,
      signal ? { signal } : undefined,
    ),
  appendPrice: (id: string, price: PricePoint) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/price`,
      jsonInit('POST', price),
    ),
  appendState: (id: string, change: StateChange) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/state`,
      jsonInit('POST', change),
    ),
  importVault: () =>
    request<{ ok: boolean; created: number; already_present: number }>(
      '/finance/api/import/obsidian',
      { method: 'POST' },
    ),
  /** Conflicts come back named, never resolved — the caller shows them. */
  writeback: () => request<WritebackResult>('/finance/api/writeback', { method: 'POST' }),
  dashboard: (filters: {
    start?: string;
    end?: string;
    account?: string;
    category?: string;
    currency?: string;
  } = {}, signal?: AbortSignal) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) if (value) query.set(key, value);
    return request<FinanceDashboard>(
      `/finance/api/dashboard${query.size ? `?${query}` : ''}`,
      signal ? { signal } : undefined,
    );
  },
  /** The travel page wants only the per-trip slice, never the whole projection. */
  tripSpending: (signal?: AbortSignal) =>
    request<FinanceDashboard>(
      '/finance/api/dashboard',
      signal ? { signal } : undefined,
    ).then((dashboard) => dashboard.trip_spending),
  rebuildLedger: () =>
    request<{ ok: boolean; rows: number }>('/finance/api/ledger/rebuild', { method: 'POST' }),
  csvMappings: () =>
    request<CsvMappingProfile[]>('/finance/api/import/csv/mappings'),
  investmentMappings: () =>
    request<InvestmentCsvMappingProfile[]>('/finance/api/import/investments/mappings'),
  previewInvestments: (content: string, mapping: InvestmentCsvMapping) =>
    request<InvestmentPreview>(
      '/finance/api/import/investments/preview',
      jsonInit('POST', { content, mapping }),
    ),
  confirmInvestments: (
    content: string,
    mapping: InvestmentCsvMapping,
    sourceKey: string,
    expectedSnapshotId: string,
    coverage: HoldingsCoverage,
  ) =>
    request<{ ok: boolean; created: boolean; snapshot: ReviewedHoldingsSnapshot }>(
      '/finance/api/import/investments/confirm',
      jsonInit('POST', {
        content,
        mapping,
        source_key: sourceKey,
        expected_snapshot_id: expectedSnapshotId,
        coverage,
      }),
    ),
  candidates: () => request<TransactionCandidate[]>('/finance/api/import/candidates'),
  previewCsv: (content: string, mapping: CsvMapping) =>
    request<CsvImportPreview>(
      '/finance/api/import/csv/preview',
      jsonInit('POST', { content, mapping }),
    ),
  importCsv: (content: string, mapping: CsvMapping, expectedPreviewId: string) =>
    request<{
      ok: boolean;
      created: number;
      already_present: number;
      duplicate_rows: number;
      preserved_repetitions: number;
      ignored_non_transaction_rows: number;
    }>(
      '/finance/api/import/csv',
      jsonInit('POST', {
        content,
        mapping,
        expected_preview_id: expectedPreviewId,
      }),
    ),
  reviewCandidate: (
    id: string,
    decision: 'confirm' | 'reject',
    account?: string,
    sourceAccount?: string,
  ) =>
    request<{ ok: boolean; id: string; state: CandidateState; journal_written: boolean }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/review`,
      jsonInit('POST', { decision, account, source_account: sourceAccount }),
    ),
  confirmCandidates: (items: { id: string; account: string }[]) =>
    request<{ ok: boolean; confirmed: number; journal_writes: number }>(
      '/finance/api/import/candidates/confirm-batch',
      jsonInit('POST', { items }),
    ),
  reclassifyCandidates: (items: { id: string; account: string }[]) =>
    request<{ ok: boolean; reviewed: number; reclassified: number }>(
      '/finance/api/import/candidates/reclassify-batch',
      jsonInit('POST', { items }),
    ),
  reconcileTransfer: (id: string, counterpartId: string) =>
    request<{
      ok: boolean;
      canonical_id: string;
      duplicate_id: string;
      journal_written: boolean;
    }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/reconcile-transfer`,
      jsonInit('POST', { counterpart_id: counterpartId }),
    ),
  allocateExpense: (
    id: string,
    personalCents: number,
    purpose: SpendingPurpose,
    tripId: string | null,
  ) =>
    request<{ ok: boolean; id: string; journal_written: boolean }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/allocation`,
      jsonInit('POST', {
        personal_cents: personalCents,
        purpose,
        trip_id: tripId,
      }),
    ),
  linkReimbursement: (id: string, expenseCandidateId: string) =>
    request<{
      ok: boolean;
      id: string;
      expense_candidate_id: string;
      journal_written: boolean;
    }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/reimbursement`,
      jsonInit('POST', { expense_candidate_id: expenseCandidateId }),
    ),
  updateBalanceSnapshot: (update: ManualBalanceUpdate) =>
    request<{ ok: boolean; snapshot: ManualBalanceSnapshot }>(
      '/finance/api/balance-snapshot',
      jsonInit('POST', update),
    ),
};

// The wire contract of capabilities/places (README.md there, "HTTP surface").
// Layer endpoints answer GeoJSON FeatureCollections so the map consumes them
// without translation; cents are integers, EUR implied, and every coordinate
// pair is [longitude, latitude].
