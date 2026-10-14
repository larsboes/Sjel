import { error } from "@sveltejs/kit";
import { entries } from "virtual:sjel-docs";
import { toDocEntry, groupDocs } from "$lib/docs/content";

export function load({ params }: { params: { slug?: string } }) {
  const rawSlug = params.slug ?? "index";
  const slug = rawSlug === "README" ? "index" : rawSlug;

  const found = entries.find(
    (e) => e.slug === slug || (slug === "index" && (e.slug === "index" || e.slug === "README")),
  );

  if (!found) {
    error(404, `No documentation page found for '${rawSlug}'`);
  }

  const allDocs = entries.map(toDocEntry);
  const sections = groupDocs(allDocs);

  return {
    doc: toDocEntry(found),
    sections,
  };
}
