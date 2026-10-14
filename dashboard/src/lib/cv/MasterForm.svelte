<script lang="ts">
  /** Every field of `master_cv.yaml` as a form. Groups collapse; nothing saves until Save. */
  import type { Master } from "./model";
  import ItemMeta from "./ItemMeta.svelte";
  import ListTools from "./ListTools.svelte";
  import LocField from "./LocField.svelte";
  import SectionForm from "./SectionForm.svelte";
  import TextField from "./TextField.svelte";

  let { master = $bindable(), profiles }: { master: Master; profiles: string[] } = $props();

  const variantsMissing = (map: Record<string, unknown>) => profiles.filter((p) => !(p in map));
  const roleOf = (role: unknown) => (typeof role === "string" ? role : ((role as { en?: string })?.en ?? ""));

  /** Skill items edited as one comma-separated line per language. */
  const joinItems = (v: string[]) => v.join(", ");
  const splitItems = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
</script>

<details open>
  <summary>Basics</summary>
  <div class="grid">
    <label>Name <input bind:value={master.name} /></label>
    <label>Email <input bind:value={master.contact.email} /></label>
    <label>Phone <input bind:value={() => master.contact.phone ?? "", (v) => (master.contact.phone = v || undefined)} /></label>
    <label>Location <input bind:value={master.contact.location} /></label>
    <label>LinkedIn <input bind:value={master.contact.linkedin} /></label>
    <label>GitHub <input bind:value={master.contact.github} /></label>
  </div>
</details>

{#each [["title", "Title under the name"], ["summary", "Summary paragraph"]] as [key, label] (key)}
  {@const map = master[key as "title" | "summary"]}
  <details>
    <summary>{label} <span class="hint">per profile, falls back to default</span></summary>
    {#each Object.keys(map) as p (p)}
      <div class="item">
        <div class="head"><strong>{p}</strong>
          {#if p !== "default"}<button class="x" aria-label="Remove {p} variant" onclick={() => { delete map[p]; master[key as "title"] = { ...map }; }}>×</button>{/if}
        </div>
        <LocField bind:value={map[p]} label={label} multiline={key === "summary"} />
      </div>
    {/each}
    {#if variantsMissing(map).length}
      <select aria-label="Add {label} variant" value="" onchange={(e) => { const p = e.currentTarget.value; if (p) master[key as "title"] = { ...map, [p]: { en: "" } }; }}>
        <option value="">+ variant for profile…</option>
        {#each variantsMissing(map) as p (p)}<option>{p}</option>{/each}
      </select>
    {/if}
  </details>
{/each}

<details>
  <summary>LinkedIn <span class="hint">headline and About; not in the PDF</span></summary>
  {#if master.linkedin}
    {#if master.linkedin.headline}<LocField bind:value={master.linkedin.headline} label="Headline" />{/if}
    {#if master.linkedin.about}<LocField bind:value={master.linkedin.about} label="About" multiline />{/if}
  {:else}
    <button class="btn btn-outline" onclick={() => (master.linkedin = { headline: { en: "" }, about: { en: "" } })}>+ LinkedIn text</button>
  {/if}
</details>

<details>
  <summary>Experience <span class="hint">{master.experience.length} positions</span></summary>
  {#each master.experience as e, i (i)}
    <details class="item">
      <summary class="head">{roleOf(e.role) || "—"} @ {e.company} <span class="hint">{e.date}</span>
        <ListTools bind:list={master.experience} index={i} what="position" />
      </summary>
      <div class="grid">
        <label>Company <input bind:value={e.company} /></label>
        <label>Location <input bind:value={e.location} /></label>
        <label>Dates <input bind:value={e.date} /></label>
      </div>
      <TextField bind:value={e.role} label="Role" />
      <ItemMeta bind:item={master.experience[i]} {profiles} />
      <h5>Bullets</h5>
      {#each e.bullets as _, j (j)}
        <div class="item">
          <div class="head"><span class="hint">Bullet {j + 1}</span><ListTools bind:list={e.bullets} index={j} what="bullet" /></div>
          <LocField bind:value={e.bullets[j].text} label="Text" multiline />
          <ItemMeta bind:item={e.bullets[j]} {profiles} />
        </div>
      {/each}
      <button class="btn btn-outline" onclick={() => (e.bullets = [...e.bullets, { text: { en: "" } }])}>+ Bullet</button>
    </details>
  {/each}
  <button class="btn btn-outline" onclick={() => (master.experience = [{ company: "", location: "", date: "", bullets: [] }, ...master.experience])}>+ Position (top)</button>
</details>

<details>
  <summary>Education</summary>
  {#each master.education as ed, i (i)}
    <details class="item">
      <summary class="head">{ed.institution} <span class="hint">{ed.date}</span><ListTools bind:list={master.education} index={i} what="school" /></summary>
      <div class="grid">
        <label>Institution <input bind:value={ed.institution} /></label>
        <label>Dates <input bind:value={ed.date} /></label>
        <label>Location <input bind:value={() => ed.location ?? "", (v) => (ed.location = v || undefined)} /></label>
      </div>
      <LocField bind:value={ed.degree} label="Degree" />
      <ItemMeta bind:item={master.education[i]} {profiles} />
      <h5>Details</h5>
      {#each ed.details as _, j (j)}
        <div class="item">
          <div class="head"><span class="hint">Detail {j + 1}</span><ListTools bind:list={ed.details} index={j} what="detail" /></div>
          <LocField bind:value={ed.details[j]} label="Text" />
          <ItemMeta bind:item={ed.details[j]} {profiles} />
        </div>
      {/each}
      <button class="btn btn-outline" onclick={() => (ed.details = [...ed.details, { en: "" }])}>+ Detail</button>
    </details>
  {/each}
  <button class="btn btn-outline" onclick={() => (master.education = [...master.education, { institution: "", degree: { en: "" }, date: "", details: [] }])}>+ School</button>
</details>

<details>
  <summary>Skills</summary>
  {#each master.skills as s, i (i)}
    <div class="item">
      <div class="head"><strong>{s.category.en}</strong><ListTools bind:list={master.skills} index={i} what="skill row" /></div>
      <LocField bind:value={s.category} label="Row label" />
      {#if Array.isArray(s.items)}
        {@const shared = s.items}
        <label class="wide">Items, comma-separated
          <input bind:value={() => joinItems(shared), (v) => (s.items = splitItems(v))} /></label>
      {:else}
        {@const per = s.items}
        <label class="wide">EN items <input bind:value={() => joinItems(per.en), (v) => (per.en = splitItems(v))} /></label>
        <label class="wide">DE items <input bind:value={() => joinItems(per.de), (v) => (per.de = splitItems(v))} /></label>
      {/if}
      <ItemMeta bind:item={master.skills[i]} {profiles} />
    </div>
  {/each}
  <button class="btn btn-outline" onclick={() => (master.skills = [...master.skills, { category: { en: "" }, items: [] }])}>+ Skill row</button>
</details>

<details>
  <summary>Sections <span class="hint">hackathons, soft skills, community, interests, anything else</span></summary>
  {#each master.sections ?? [] as s, i (i)}
    <details class="item">
      <summary class="head">{s.title.en}<ListTools bind:list={() => master.sections ?? [], (v) => (master.sections = v as typeof s[])} index={i} what="section" /></summary>
      <SectionForm bind:section={master.sections![i]} {profiles} />
    </details>
  {/each}
  <button class="btn btn-outline" onclick={() => (master.sections = [...(master.sections ?? []), { title: { en: "New section" } }])}>+ Section</button>
</details>

<details>
  <summary>Page limits and languages</summary>
  <p class="hint">1 page unless set; 0 means no limit (the full CV). Untick a language to stop building it.</p>
  {#each profiles as p (p)}
    {@const langs = master.profile_langs?.[p] ?? ["en", "de"]}
    <div class="limits">
      <strong>{p}</strong>
      <label>pages <input type="number" min="0" max="5" value={master.page_limit?.[p] ?? 1}
        onchange={(e) => { const n = +e.currentTarget.value; const all = { ...(master.page_limit ?? {}) }; if (n === 1) delete all[p]; else all[p] = n; master.page_limit = all; }} /></label>
      {#each ["en", "de"] as l (l)}
        <label><input type="checkbox" checked={langs.includes(l as "en")}
          onchange={(e) => { const next = e.currentTarget.checked ? [...langs, l] : langs.filter((x) => x !== l); const all = { ...(master.profile_langs ?? {}) }; if (next.length === 2) delete all[p]; else all[p] = next as ("en" | "de")[]; master.profile_langs = all; }} /> {l}</label>
      {/each}
    </div>
  {/each}
</details>

<style>
  details { margin: 0.35rem 0; }
  summary { cursor: pointer; font-size: 0.82rem; font-weight: 600; }
  summary.head { font-weight: 500; }
  h5 { font-size: 0.75rem; margin: 0.6rem 0 0.2rem; color: var(--text-secondary); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr)); gap: 0.35rem; margin: 0.35rem 0; }
  label { display: flex; flex-direction: column; gap: 0.1rem; font-size: var(--text-xs); color: var(--text-secondary); }
  label.wide { margin: 0.25rem 0; }
  .item { border-left: 2px solid var(--card-border); padding: 0.2rem 0 0.4rem 0.6rem; margin: 0.4rem 0; }
  .head { display: flex; align-items: center; gap: 0.4rem; font-size: 0.78rem; }
  .hint { font-weight: normal; color: var(--text-tertiary); font-size: var(--text-xs); }
  .x { border: 0; background: none; cursor: pointer; color: var(--text-tertiary); }
  .limits { display: flex; gap: 0.8rem; align-items: center; font-size: var(--text-xs); margin: 0.2rem 0; }
  .limits strong { width: 6rem; }
  .limits label { flex-direction: row; align-items: center; gap: 0.25rem; }
  .limits input[type="number"] { width: 3rem; }
</style>
