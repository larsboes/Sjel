# Controlled-Language Discipline

This module borrows ASD-STE100 rule categories. It does not claim certified ASD-STE100 lexical compliance because the official approved-word dictionary is not bundled.

## Choose intensity

- **Strict:** instructions, procedures, safety text, error/tool/system messages, machine-consumed text. Apply all structural rules and the lexical discipline below.
- **Flavored:** README/explanatory technical prose. Apply structural rules strongly; use lexical rules as guidance when strict wording would damage useful prose.

## Structural rules

- Prefer active voice for procedures/instructions. Passive voice is acceptable when the actor is genuinely unknown or irrelevant.
- Put one instruction per sentence.
- Target <=20 words for instructions/procedures and <=25 words for descriptive text when precision survives.
- Do not remove subject, verb, article, condition, or qualifier merely to shorten a sentence.
- Avoid noun clusters longer than three stacked nouns; expand them into clearer relationships.
- Do not use semicolons; split the sentence.
- Prefer a list for three or more steps/conditions.
- Keep one topic per paragraph and usually <=6 sentences.
- Put safety-critical commands/conditions first.

## Lexical discipline

- Use one term for one concept. Do not rotate synonyms for style.
- Prefer the plain, common, single-meaning word.
- Prefer a direct verb over an action noun: "analyze" rather than "perform an analysis".
- Avoid phrasal verbs when a single literal verb exists: "remove" rather than "take off", "start" rather than "spin up".
- Keep necessary domain terms; define uncommon terms once when helpful.
- Delete unsupported marketing adjectives such as seamless, robust, powerful, cutting-edge, effortless, or blazing-fast. Replace them with evidence if the claim matters.

## Tense and modality

Prefer simple present/past/future and imperatives where they preserve meaning.

Do **not** simplify away information carried by modality or aspect. Keep wording such as "may have failed" when it expresses uncertainty/current relevance that "failed" would lose. Evidence strength is content.

Never add a cause, frequency, mechanism, requirement, or certainty that the source did not state.

## Mechanical scan

Look for:

1. synonym rotation;
2. hedge stacking that adds no information;
3. nominalizations;
4. unsupported quality adjectives;
5. run-on sentences/clause stacks;
6. soft phrasal verbs/idioms.

Delete only empty hedging. Preserve real uncertainty.

## Rewrite process

1. Read once for exact meaning before editing.
2. Identify ambiguity/structure problems.
3. Rewrite only what needs fixing.
4. Preserve every fact, condition, scope qualifier, safety condition, version, and hedge.
5. If shortening would lose precision, keep the longer wording.
6. If the input already complies, do not force changes.

By default return the rewritten text only. If the user asks for the reasoning/diff, show the relevant rule, original, and rewrite.
