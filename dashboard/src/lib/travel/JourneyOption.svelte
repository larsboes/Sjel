<script lang="ts">
  import { tip } from "$lib/tip";
  import Icon from "$lib/Icon.svelte";
  import SparpreisSparkline, { type Observation } from "$lib/travel/SparpreisSparkline.svelte";
  import type { Journey, ConnectionLeg } from "$lib/api";

  let {
    journey,
    expanded,
    saved = false,
    priceHistory,
    onToggle,
    onSave,
  }: {
    journey: Journey;
    expanded: boolean;
    saved?: boolean;
    priceHistory?: Observation[];
    onToggle: () => void;
    onSave?: () => void;
  } = $props();

  const time = (date: string) =>
    new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit" }).format(
      new Date(date),
    );

  const duration = (minutes: number) =>
    `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")} h`;

  /// Minutes between the plan and reality, or null when HAFAS offered no real-time
  /// value. Null is "not reported", never "on time" -- rendering the two the same way
  /// is what made a late train look punctual before these fields were carried at all.
  const delayOf = (scheduled?: string | null, realtime?: string | null): number | null => {
    if (!scheduled || !realtime) return null;
    const delta = (new Date(realtime).getTime() - new Date(scheduled).getTime()) / 60000;
    return Number.isFinite(delta) ? Math.round(delta) : null;
  };

  const legDelay = (leg: ConnectionLeg) => ({
    departure: delayOf(leg.scheduled_departure, leg.realtime_departure),
    arrival: delayOf(leg.scheduled_arrival, leg.realtime_arrival),
  });

  const signed = (minutes: number) => (minutes > 0 ? `+${minutes}` : `${minutes}`);

  const punctuality = $derived(journey.arrival_punctuality ?? null);

  /// Punctuality's own sample floor is 30, so anything present cleared it. These bands
  /// only say how far past the floor it got -- the number is shown either way, because
  /// a reader who wants to judge 47 observations for themselves should be able to.
  const confidence = $derived.by(() => {
    if (!punctuality) return null;
    if (punctuality.n >= 1000) return { label: "lots of data", bars: 4 };
    if (punctuality.n >= 300) return { label: "solid sample", bars: 3 };
    if (punctuality.n >= 100) return { label: "narrow sample", bars: 2 };
    return { label: "few journeys", bars: 1 };
  });

  const tone = $derived.by(() => {
    if (!punctuality) return "unknown";
    if (punctuality.share_late_6 >= 0.35) return "bad";
    if (punctuality.share_late_6 >= 0.15) return "mixed";
    return "good";
  });

  const percent = (share: number) => `${Math.round(share * 100)} %`;

  /// The p50 marker's position on a track that ends at p90. Both are minutes late, so
  /// the track is the spread between the ordinary case and the unlucky one.
  const p50Offset = $derived.by(() => {
    if (!punctuality || punctuality.p90 <= 0) return 0;
    return Math.min(100, Math.max(0, (punctuality.p50 / punctuality.p90) * 100));
  });

  const anyCancelled = $derived(journey.legs.some((leg) => leg.cancelled));
  const hasRegional = $derived(journey.legs.some((leg) => leg.is_regional));
  const allRegional = $derived(
    journey.legs.length > 0 && journey.legs.every((leg) => leg.is_regional),
  );
</script>

<li class:expanded>
  <button class="journey-open" type="button" aria-expanded={expanded} onclick={onToggle}>
    <span class="journey-time">
      <strong>{time(journey.legs[0]?.departure_time ?? "")}</strong>
      <span>{duration(journey.total_duration_minutes)}</span>
    </span>
    <span class="journey-route">
      <span class="trains-line">
        <strong>{journey.legs.map((leg) => leg.train_name || leg.train_number).join(" · ")}</strong>
        {#if allRegional}
          <span class="d-ticket-pill full" use:tip={"All regional legs — fully covered by Deutschlandticket"}>D-Ticket</span>
        {:else if hasRegional}
          <span class="d-ticket-pill part" use:tip={"Includes regional legs eligible for Deutschlandticket"}>Part D-Ticket</span>
        {/if}
      </span>
      <span>
        {journey.legs.length - 1 === 0
          ? "direct"
          : `${journey.legs.length - 1} change${journey.legs.length - 1 === 1 ? "" : "s"}`}
        {#if punctuality}
          <span class="dot" aria-hidden="true">·</span>
          <span class="risk {tone}">{percent(punctuality.share_late_6)} ≥ 6 min late</span>
        {:else}
          <span class="dot" aria-hidden="true">·</span>
          <span class="risk unknown">no delay history</span>
        {/if}
      </span>
    </span>
    <Icon name="arrow-right" size={13} />
  </button>

  <div class="journey-action">
    {#if journey.ranking}
      <span
        class="rank"
        use:tip={`ranked by your ${journey.ranking.source === "request" ? "request" : "profile"} weights`}
      >
        #{journey.ranking.rank}
      </span>
    {/if}
    <div class="price-stack">
      {#if allRegional}
        <strong class="d-ticket-free">0.00 €</strong>
        {#if journey.total_price !== null && journey.total_price > 0}
          <span class="price-strikethrough">{journey.total_price.toFixed(2)} €</span>
        {/if}
      {:else}
        <strong>
          {journey.total_price === null ? "price unknown" : `${journey.total_price.toFixed(2)} €`}
        </strong>
        {#if priceHistory && priceHistory.length >= 2}
          <SparpreisSparkline history={priceHistory} width={76} height={18} />
        {/if}
      {/if}
    </div>
    {#if onSave}
      <button class="save" type="button" disabled={saved} onclick={onSave}>
        {#if saved}
          <Icon name="check" size={13} /> Saved
        {:else}
          <Icon name="plus" size={13} /> Add
        {/if}
      </button>
    {/if}
  </div>

  {#if anyCancelled}
    <p class="cancelled-banner">At least one leg is reported cancelled.</p>
  {/if}

  {#if expanded}
    <div class="detail">
      {#if journey.ranking}
        <h4>Why it ranked here</h4>
        <ul class="rank-factors">
          {#each journey.ranking.factors as factor (factor.key)}
            <li>
              <span class="factor-label">{factor.label}</span>
              <span class="factor-bar" aria-hidden="true">
                <span style={`width: ${Math.round(factor.score * 100)}%`}></span>
              </span>
              <span class="factor-why">{factor.rationale}</span>
              <span class="factor-weight">{Math.round(factor.weight * 100)}%</span>
            </li>
          {/each}
        </ul>
        <p class="rank-note">
          {journey.ranking.factors.length === 4
            ? "All four terms were measured."
            : `${4 - journey.ranking.factors.length} term(s) could not be measured and were dropped, not scored zero -- the rest were re-normalised.`}
        </p>
      {/if}
      <h4>Itinerary</h4>
      <ol class="leg-list">
        {#each journey.legs as leg, index (`${journey.id}:${index}`)}
          {@const delay = legDelay(leg)}
          <li class:leg-cancelled={leg.cancelled}>
            <span class="leg-index">{index + 1}</span>
            <div>
              <strong>{leg.train_name || leg.train_number}</strong>
              <span class="leg-stop">
                <span class="clock">{time(leg.departure_time)}</span>
                {leg.origin.name}
                {#if delay.departure !== null && delay.departure !== 0}
                  <span class="delay {delay.departure > 0 ? 'late' : 'early'}">
                    {signed(delay.departure)} min
                  </span>
                {/if}
              </span>
              <span class="leg-stop">
                <span class="clock">{time(leg.arrival_time)}</span>
                {leg.destination.name}
                {#if delay.arrival !== null && delay.arrival !== 0}
                  <span class="delay {delay.arrival > 0 ? 'late' : 'early'}">
                    {signed(delay.arrival)} min
                  </span>
                {/if}
              </span>
              {#if leg.cancelled}
                <span class="badge danger">Cancelled</span>
              {/if}
            </div>
            <small>
              {#if leg.platform}Platform {leg.platform}{:else}Platform TBA{/if}
              {#if leg.is_regional}<br />Deutschland-Ticket{/if}
            </small>
          </li>
        {/each}
      </ol>

      <h4>Punctuality at destination</h4>
      {#if punctuality}
        <p class="cell-key">
          {punctuality.station_name ?? journey.end_station.name} · {punctuality.train_type} ·
          {punctuality.weekend ? "weekend" : "weekday"}, {punctuality.hour}:00
        </p>

        <dl class="stats">
          <div>
            <dt>typically</dt>
            <dd class="mono">{signed(punctuality.p50)} min</dd>
          </div>
          <div>
            <dt>unlucky case</dt>
            <dd class="mono">{signed(punctuality.p90)} min</dd>
          </div>
          <div>
            <dt>≥ 6 min late</dt>
            <dd class="mono {tone}">{percent(punctuality.share_late_6)}</dd>
          </div>
          <div>
            <dt>cancelled</dt>
            <dd class="mono">{percent(punctuality.cancel_rate)}</dd>
          </div>
        </dl>

        <div class="spread" aria-hidden="true">
          <span class="spread-end">+0</span>
          <span class="track">
            <span class="fill {tone}" style="width: {p50Offset}%"></span>
            <span class="marker" style="left: {p50Offset}%"></span>
          </span>
          <span class="spread-end">{signed(punctuality.p90)}</span>
        </div>

        <p class="confidence">
          <span class="bars" aria-hidden="true">
            {#each [1, 2, 3, 4] as bar (bar)}
              <span class:on={confidence !== null && bar <= confidence.bars}></span>
            {/each}
          </span>
          {confidence?.label} · {punctuality.n.toLocaleString("en-GB")} journeys measured
        </p>
        <p class="caveat">
          Measured on comparable trains at this stop. Not a forecast for this journey,
          and not a statement about transfer risk.
        </p>
      {:else}
        <p class="caveat">
          No delay history for this stop, train type and hour. That means unmeasured,
          not punctual.
        </p>
      {/if}
    </div>
  {/if}
</li>

<style>
  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  li:last-child {
    border-bottom: 0;
  }

  li.expanded {
    background: color-mix(in srgb, var(--primary-soft) 38%, var(--card-bg));
  }

  .journey-open {
    display: grid;
    grid-template-columns: 4rem minmax(0, 1fr) auto;
    gap: 0.75rem;
    align-items: center;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .journey-open :global(svg) {
    color: var(--text-tertiary);
    transition: transform var(--motion-fast) ease;
  }

  .journey-open[aria-expanded="true"] :global(svg) {
    transform: rotate(90deg);
  }

  .journey-time strong,
  .journey-time span,
  .journey-route strong,
  .journey-route span,
  .journey-action strong {
    display: block;
  }

  .journey-time strong {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .journey-time span,
  .journey-route span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .journey-route {
    min-width: 0;
  }

  .journey-route strong {
    overflow: hidden;
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .trains-line {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    overflow: hidden;
  }

  .d-ticket-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.05rem 0.35rem;
    border-radius: 999px;
    font-size: var(--text-2xs);
    font-weight: 700;
    letter-spacing: 0.02em;
    flex-shrink: 0;
  }

  .d-ticket-pill.full {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.3);
  }

  .d-ticket-pill.part {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-tertiary);
    border: 1px solid var(--card-border);
  }

  .price-stack {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
  }

  .d-ticket-free {
    color: #10b981;
    font-family: var(--font-mono);
  }

  .price-strikethrough {
    font-size: var(--text-2xs);
    text-decoration: line-through;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
  }

  .leg-dticket {
    color: #10b981;
    font-weight: 600;
  }

  .dot,
  .rank {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
    letter-spacing: 0.02em;
  }

  .rank-factors {
    list-style: none;
    margin: 0 0 0.5rem;
    padding: 0;
    display: grid;
    gap: 0.25rem;
  }

  .rank-factors li {
    display: grid;
    grid-template-columns: 7.5rem 4rem 1fr 2.5rem;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }

  .factor-label {
    color: var(--text-tertiary);
  }

  /* The bar shows the score the factor earned, not its weight: the weight is the
     number on the right, and drawing the two the same way made a heavily weighted
     factor that scored badly look like a good one. */
  .factor-bar {
    display: block;
    height: 0.4rem;
    border-radius: 999px;
    background: var(--card-border);
    overflow: hidden;
  }

  .factor-bar > span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .factor-why {
    color: var(--text-primary);
  }

  .factor-weight {
    text-align: right;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .rank-note {
    margin: 0 0 0.75rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .risk {
    display: inline !important;
    margin-left: 0.25rem;
  }

  .risk.good {
    color: var(--success);
  }

  .risk.mixed {
    color: var(--warning-ink);
  }

  .risk.bad {
    color: var(--danger);
  }

  .risk.unknown {
    color: var(--text-tertiary);
    font-style: italic;
  }

  .journey-action {
    text-align: right;
  }

  .journey-action strong {
    font-size: var(--text-xs);
  }

  .save {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.2rem;
    padding: 0.2rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: 0.625rem;
    font-weight: 600;
    cursor: pointer;
  }

  .save:disabled {
    color: var(--success);
    cursor: default;
  }

  .cancelled-banner {
    grid-column: 1 / -1;
    margin: 0;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: 0.625rem;
    font-weight: 600;
  }

  .detail {
    grid-column: 1 / -1;
    padding-top: 0.5rem;
    border-top: 1px solid var(--card-border);
  }

  h4 {
    margin: 0.6rem 0 0.35rem;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .leg-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .leg-list > li {
    display: grid;
    grid-template-columns: 1.5rem minmax(0, 1fr) auto;
    gap: 0.55rem;
    align-items: start;
    padding: 0.6rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .leg-list > li:last-child {
    border-bottom: 0;
  }

  .leg-list > li.leg-cancelled {
    opacity: 0.7;
    text-decoration: line-through;
  }

  .leg-index {
    display: grid;
    place-items: center;
    width: 1.35rem;
    height: 1.35rem;
    border-radius: 50%;
    background: var(--primary-soft);
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
    font-weight: 700;
  }

  .leg-list strong {
    display: block;
    font-size: var(--text-2xs);
  }

  .leg-stop {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .clock {
    display: inline-block;
    min-width: 2.6rem;
    color: var(--text-secondary);
    font-family: var(--font-mono);
  }

  .delay {
    margin-left: 0.3rem;
    font-family: var(--font-mono);
    font-weight: 700;
  }

  .delay.late {
    color: var(--danger);
  }

  .delay.early {
    color: var(--success);
  }

  .badge {
    display: inline-block;
    margin-top: 0.25rem;
    padding: 0.1rem 0.3rem;
    border-radius: var(--radius-sm);
    font-size: 0.5625rem;
    font-weight: 700;
  }

  .badge.danger {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .leg-list small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-align: right;
  }

  .cell-key {
    margin: 0 0 0.4rem;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(5.5rem, 1fr));
    gap: 0.4rem;
    margin: 0;
  }

  .stats div {
    padding: 0.35rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
  }

  .stats dt {
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  .stats dd {
    margin: 0.1rem 0 0;
    font-size: var(--text-xs);
    font-weight: 700;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .stats dd.good {
    color: var(--success);
  }

  .stats dd.mixed {
    color: var(--warning-ink);
  }

  .stats dd.bad {
    color: var(--danger);
  }

  .spread {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 0.4rem;
    align-items: center;
    margin-top: 0.5rem;
  }

  .spread-end {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
  }

  .track {
    position: relative;
    display: block;
    height: 0.3rem;
    border-radius: 999px;
    background: var(--card-border);
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 999px;
    background: var(--text-tertiary);
  }

  .fill.good {
    background: var(--success);
  }

  .fill.mixed {
    background: var(--warning);
  }

  .fill.bad {
    background: var(--danger);
  }

  .marker {
    position: absolute;
    top: -0.15rem;
    width: 2px;
    height: 0.6rem;
    background: var(--text-primary);
  }

  .confidence {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.5rem 0 0;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .bars {
    display: inline-flex;
    gap: 1px;
  }

  .bars span {
    width: 3px;
    height: 0.6rem;
    border-radius: 1px;
    background: var(--card-border);
  }

  .bars span.on {
    background: var(--primary);
  }

  .caveat {
    margin: 0.35rem 0 0.2rem;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    line-height: 1.45;
  }
</style>
