<script lang="ts">
  /**
   * Which Sjel is this, and where does it live.
   *
   * One component, two densities, because the answer is the same fact in both places:
   * the home page wants the link and a word of state at a glance, /self wants the full
   * version identity next to the rest of the self-model. Two components would be two
   * places to fix the day a field is added.
   *
   * Read-only — see RepoStatus in $lib/api.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { axonStatus, type RepoStatus } from "$lib/api";

  let { detailed = false }: { detailed?: boolean } = $props();

  let repos = $state<RepoStatus[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);

  const ROLE_LABEL: Record<RepoStatus["role"], string> = {
    spine: "Spine · public-safe",
    overlay: "Overlay · private",
  };

  onMount(async () => {
    try {
      repos = (await axonStatus.repos()).repos;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      loading = false;
    }
  });

  /** The one-line version identity. An untagged repo says so rather than showing "0". */
  function version(r: RepoStatus): string {
    if (r.tag) return `${r.tag} +${r.commits_since_tag ?? 0}`;
    return r.describe ?? "unknown";
  }

  /** Only what is actually true: no upstream is a different state from being in sync. */
  function sync(r: RepoStatus): string[] {
    const parts: string[] = [];
    if (r.ahead === null) parts.push("no upstream");
    else {
      if (r.ahead > 0) parts.push(`${r.ahead} unpushed`);
      if ((r.behind ?? 0) > 0) parts.push(`${r.behind} behind`);
      if (r.ahead === 0 && (r.behind ?? 0) === 0) parts.push("in sync");
    }
    if (r.dirty) parts.push("uncommitted");
    return parts;
  }

  const relative = (iso: string | null): string => {
    if (!iso) return "";
    const days = Math.floor((Date.now() - new Date(iso).getTime()) / 86_400_000);
    if (days <= 0) return "today";
    if (days === 1) return "yesterday";
    return `${days} days ago`;
  };
</script>

<!--
  The kicker and heading are styled HERE, not borrowed from whichever page mounts this.
  Svelte scopes styles per component, so the home page's .section-kicker rule never
  reached this markup — the class was on the element and did nothing, which rendered as
  a plain 15px "Herkunft" instead of a kicker. Matching values, one owner.
-->
<section class="repos" class:detailed>
  <div class="head">
    <span class="kicker">Source</span>
    <h2>Repos</h2>
  </div>

  {#if loading}
    <p class="muted">Reading version…</p>
  {:else if error}
    <p class="muted"><Icon name="alert" size={13} /> {error}</p>
  {:else}
    <ul>
      {#each repos as repo (repo.name)}
        <li>
          {#if repo.error}
            <span class="repo-copy"><strong>{repo.name}</strong><small>{repo.error}</small></span>
          {:else}
            <span class="repo-mark"><Icon name="git-branch" size={15} /></span>
            <span class="repo-copy">
              <strong>{repo.name}</strong>
              <small class="mono">{version(repo)}</small>
              {#if detailed}
                <small>{ROLE_LABEL[repo.role]} · {repo.branch ?? "detached"}</small>
                <small>{sync(repo).join(" · ")}</small>
                <small>last commit {relative(repo.last_commit_date)}</small>
              {:else}
                <small>{sync(repo).join(" · ")}</small>
              {/if}
            </span>
            {#if repo.remote_url}
              <a
                class="btn icon-action"
                href={repo.remote_url}
                target="_blank"
                rel="noreferrer"
                aria-label={`Open ${repo.name} on GitHub`}
                title="Open on GitHub"
              >
                <Icon name="external" size={13} />
              </a>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    {#if detailed && repos.every((r) => !r.tag)}
      <p class="muted">
        Neither repository has a tag yet, so the commit identifies the version. Tags are
        created through the CLI, not this page.
      </p>
    {/if}
  {/if}
</section>

<style>
  .repos {
    min-width: 0;
  }

  /* The detailed variant is a page section, not a sidebar rail: unconstrained it ran
     to 1300px for four short lines of text. */
  .repos.detailed {
    max-width: 52rem;
    margin-bottom: 1.5rem;
  }

  .head {
    margin-bottom: 0.8rem;
  }

  .kicker {
    display: block;
    margin: 0 0 0.15rem;
    color: var(--primary);
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 650;
    letter-spacing: -0.015em;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .detailed ul {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.6rem 0.7rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
  }

  .repo-mark {
    display: grid;
    place-items: center;
    width: 1.9rem;
    height: 1.9rem;
    flex-shrink: 0;
    border-radius: var(--radius-md);
    background: var(--primary-soft);
    color: var(--primary);
  }

  .repo-copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .repo-copy strong {
    font-size: var(--text-sm);
  }

  .repo-copy small {
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .muted {
    margin: 0.5rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }
</style>
