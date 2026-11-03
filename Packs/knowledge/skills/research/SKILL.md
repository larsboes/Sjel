---
name: research
description: Chooses the route for a research question and drafts the prompt when the answer must come from Google AI Studio with Search grounding. Routes quick cited facts to the built-in web search and fetch tools, long multi-source investigations to interceptor-research, and Google-verified answers to the user, who types the drafted prompt into AI Studio and clips the answer. Use when the user asks to research, look up, compare, fact-check or verify something on the web, asks for a prompt for AI Studio or Gemini, or asks to read back an AI Studio answer. Do not use for running a deep multi-source investigation itself (use interceptor-research), for personal or private data, or for saving knowledge into the vault (use obsidian).
---

# research

Pick one route, then follow it. Cite every fact with its source URL and the date you read it.

## Rule 1: public data only

A research question goes to a cloud service. Put only public (`c0`) content in it: products,
prices, specifications, public documentation, public events. Do not put names of people,
addresses, account or order numbers, mail content, health or financial details of the user
in a query or a prompt. If the question needs them, ask the user to remove them first.

## Rule 2: do not automate AI Studio

Do not open, type into, click or scroll an AI Studio chat page (`aistudio.google.com/prompts/`,
`/app`) with a browser tool. Its `robots.txt` disallows these paths, and the Google Terms of
Service prohibit automated access against such instructions. The risk is a suspension of the
user's Google account. The user types in AI Studio. You only draft the prompt and read the
saved clip.

## Choose the route

| The user needs | Route |
|---|---|
| A few facts with sources, now | Built-in web search and fetch tools (`WebSearch`, `WebFetch`; in pi, `fetch_content`) |
| A wide investigation across many sources | Load `interceptor-research` |
| An answer verified by Google Search, or the user asks for an AI Studio prompt | The AI Studio route below |

If the user does not say, use the first route and offer the AI Studio route at the end.

## The AI Studio route

1. **Draft the prompt.** Write it in one fenced code block so the user can copy it. Include:
   - the question and the decision it serves;
   - the constraints from the conversation: budget, country, deadline, minimum specification;
   - the instruction to use Google Search for every factual claim, cite the source and its date
     inline, and mark what it could not verify as "unverified";
   - the output shape: a comparison table with named columns, then a recommendation that uses
     the numbers from the table;
   - placeholders in `<ANGLE_BRACKETS>` for facts you do not know, and tell the user to fill them.
2. **Tell the user** to turn on "Grounding with Google Search" in AI Studio, send the prompt,
   and save the answer with the Sjel Clipper.
3. **Read the clip** when the user says it is saved:

   ```bash
   comms clips --limit 5             # newest first; find the document key by its title
   comms clips show <document-key>   # the conversation as Markdown
   ```

   If `comms` is not on `PATH`, build it from the Sjel checkout:
   `cargo build --bin comms -p sjel-comms`, then run `target/debug/comms`.
4. **Report** the answer to the user with its sources. Check the claims that decide the
   outcome against their cited pages with the fetch tool, and say which ones you checked.

## Grounded answers are a record, not a data source

The Gemini API terms limit Grounding with Google Search results: show them to the user who
asked, do not cache, analyze, train on or otherwise learn from them, and keep them at most two
years. Do not copy a grounded answer into another store, a price watch or a dataset. Quote the
facts the user needs, with their original source URLs, and leave the clip as the record.
Terms: https://aistudio.google.com/docs/terms (section "Grounding with Google Search").
