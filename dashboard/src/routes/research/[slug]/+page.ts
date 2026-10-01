import { error } from "@sveltejs/kit";
import { toEntry } from "$lib/research/content";
import { entries } from "virtual:sjel-research";

export function load({ params }: { params: { slug: string } }) {
  const found = entries.find((e) => e.file === `${params.slug}.md` && e.file !== "README.md");
  if (!found) error(404, `No research entry named ${params.slug}`);
  return { entry: toEntry(found.file, found.markdown) };
}
