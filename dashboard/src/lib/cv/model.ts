/**
 * TypeScript mirror of `capabilities/cv/src/model.rs`. The Rust types are the contract:
 * `PUT /cv/model` refuses anything they refuse, so this file only has to be close enough to
 * build the form.
 */

/** Text in both languages; a missing `de` renders the English text. */
export interface Localized {
  en: string;
  de?: string;
}

/** One string for both languages, or `{en, de}`. */
export type Text = string | Localized;

/** 1 is kept longest in a one-page build, 3 is dropped first; or a level per profile. */
export type Priority = number | Record<string, number>;

/** Selection fields every filterable item carries. */
export interface ItemMeta {
  id?: string;
  profiles?: string[];
  priority?: Priority;
  source?: string;
}

export interface LocalizedItem extends Localized, ItemMeta {}

export interface Bullet extends ItemMeta {
  text: Localized;
}

export interface Experience extends ItemMeta {
  company: string;
  role?: Text;
  location: string;
  date: string;
  bullets: Bullet[];
}

export interface Education extends ItemMeta {
  institution: string;
  degree: Localized;
  date: string;
  location?: string;
  details: LocalizedItem[];
}

export interface Skill extends ItemMeta {
  category: Localized;
  items: string[] | { en: string[]; de: string[] };
}

export interface Entry extends ItemMeta {
  label: Text;
  meta?: Text;
  detail?: Localized;
}

export interface SectionBullet extends ItemMeta {
  label?: Text;
  text: Localized;
}

export interface Section extends ItemMeta {
  title: Localized;
  intro?: LocalizedItem;
  entries?: Entry[];
  bullets?: SectionBullet[];
  prose?: Localized;
}

export interface Master {
  profile_langs?: Record<string, ("en" | "de")[]>;
  page_limit?: Record<string, number>;
  name: string;
  contact: { email: string; phone?: string; location: string; linkedin: string; github: string };
  title: Record<string, Localized>;
  summary: Record<string, Localized>;
  linkedin?: { headline?: Localized; about?: Localized };
  experience: Experience[];
  education: Education[];
  skills: Skill[];
  sections?: Section[];
}

export interface Application {
  company: string;
  role: string;
  url: string;
  profile: string;
  lang: "en" | "de";
  status: string;
  log: { date: string; status: string; note?: string }[];
  title?: Localized;
  summary?: Localized;
  pin?: string[];
  hide?: string[];
  notes?: string;
}

/** Optional string keys whose empty value means "absent". */
const OPTIONAL_TEXT = new Set(["id", "source", "de", "phone", "notes", "note"]);
/** Optional list keys whose empty value means "absent". */
const OPTIONAL_LIST = new Set(["profiles", "pin", "hide", "entries", "bullets"]);
/** Optional `Text` / `Localized` keys that are absent when their English text is empty. */
const OPTIONAL_TEXT_OBJECT = new Set(["role", "meta", "detail", "label", "intro", "prose", "title", "summary", "headline", "about"]);

const isEmptyText = (v: unknown) =>
  v === "" || (typeof v === "object" && v !== null && "en" in v && (v as Localized).en === "");

/**
 * A copy with empty optional fields removed, so the form can leave a field blank instead of
 * deleting it. `title` and `summary` maps on the master are keyed by profile and keep their keys.
 */
export function clean<T>(value: T, key = ""): T {
  if (Array.isArray(value)) return value.map((v) => clean(v)) as T;
  if (value === null || typeof value !== "object") return value;
  const out: Record<string, unknown> = {};
  const profileMap = key === "title" || key === "summary";
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    if (v === undefined || v === null) continue;
    if (!profileMap) {
      if (OPTIONAL_TEXT.has(k) && v === "") continue;
      if (OPTIONAL_LIST.has(k) && Array.isArray(v) && v.length === 0) continue;
      if (OPTIONAL_TEXT_OBJECT.has(k) && isEmptyText(v)) continue;
    }
    out[k] = clean(v, k);
  }
  return out as T;
}

/** Every item `id` in the master, for an application's pin and hide pickers. */
export function itemIds(master: Master): string[] {
  const ids: string[] = [];
  const visit = (v: unknown) => {
    if (Array.isArray(v)) v.forEach(visit);
    else if (v && typeof v === "object") {
      const id = (v as ItemMeta).id;
      if (typeof id === "string" && id) ids.push(id);
      Object.values(v).forEach(visit);
    }
  };
  visit(master);
  return [...new Set(ids)].sort();
}
