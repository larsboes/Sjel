<script lang="ts">
  /**
   * A year of sun on four lines: one row per day the check computes — the two solstices and the
   * two equinoxes — and one cell per hour.
   *
   * This is a chart that happens to be a table, and that is why it is its own component rather
   * than a `DataTable`: the cell's fill is the measurement, the column count comes from the
   * data, and there is no header row to name. `DataTable` renders the app's *data* tables; a
   * second renderer with a table in it is what this file is, and `tools/dashboard-markup.test.ts`
   * names it as such instead of letting the exemption live in a comment.
   */
  import { tip } from "$lib/tip";
  import type { InteriorSun } from "$lib/api";

  interface Props {
    sun: InteriorSun;
  }

  let { sun }: Props = $props();

  /** Four days, in the order the report lists them. */
  const days = $derived([...new Set(sun.hours.map((hour) => hour.day))]);

  const label = (entry: InteriorSun["hours"][number]): string =>
    `${entry.hour_local}:00 · ${entry.altitude_deg.toFixed(0)}° up, ${entry.azimuth_deg.toFixed(0)}° · ${entry.hit.join(", ") || "nothing in the sun"}`;
</script>

<table class="sun mono">
  <tbody>
    {#each days as tag (tag)}
      <tr>
        <th scope="row">{tag}</th>
        {#each sun.hours.filter((entry) => entry.day === tag) as entry (entry.hour_local)}
          <td
            class:night={entry.altitude_deg <= 0}
            class:hit={entry.hit.length > 0}
            use:tip={label(entry)}
          >
            {entry.altitude_deg <= 0 ? "·" : entry.hit.length || "–"}
          </td>
        {/each}
      </tr>
    {/each}
  </tbody>
</table>

<style>
  /* One row per day, one column per hour: a year on four lines. */
  .sun {
    border-collapse: collapse;
    font-size: var(--text-xs);
    margin-top: 0.5rem;
  }
  .sun th {
    color: var(--text-tertiary);
    font-weight: 400;
    padding-right: 0.75rem;
    text-align: left;
  }
  .sun td {
    border: 1px solid var(--card-border);
    color: var(--text-tertiary);
    min-width: 1.5rem;
    padding: 0.15rem 0.3rem;
    text-align: center;
  }
  .sun td.night {
    background: var(--surface);
    color: var(--text-tertiary);
  }
  .sun td.hit {
    background: color-mix(in srgb, var(--warning) 22%, transparent);
    color: var(--text-primary);
  }
</style>
