---
name: web-research
description: Routes public web research and prepares Google AI Studio prompts, then retrieves the matching Sjel Clipper answer when the user says done, clipped or saved during an active research handoff. Use for public product comparisons, fact checks, AI Studio or Gemini prompts, and reading their saved answers. Do not use for code investigation, exporting private records or saving research into the vault. Use interceptor-research for a wide investigation performed by the agent.
---

# Web research

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
user's Google account. The user runs the prompt in AI Studio. The agent drafts the prompt and
reads the saved clip.

## Choose the route

| The user needs | Route |
|---|---|
| A few facts with sources, now | Built-in web search and fetch tools (`WebSearch`, `WebFetch`; in pi, `fetch_content`) |
| A wide investigation across many sources | Load `interceptor-research` |
| An answer verified by Google Search, or the user asks for an AI Studio prompt | The AI Studio route below |

If the user does not say, use the first route and offer the AI Studio route at the end.

## The AI Studio route

Read [references/ai-studio.md](references/ai-studio.md) when preparing the prompt or resuming
the handoff. During an active handoff, "done", "clipped" or "saved" requests retrieval of the
matching clip; it does not require the user to paste the answer into chat.

## Grounded answers are a record, not a data source

The Gemini API terms limit Grounding with Google Search results: show them to the user who
asked, do not cache, analyze, train on or otherwise learn from them, and keep them at most two
years. Do not copy a grounded answer into another store, a price watch or a dataset. Quote the
facts the user needs, with their original source URLs, and leave the clip as the record.
Terms: https://aistudio.google.com/docs/terms (section "Grounding with Google Search").
