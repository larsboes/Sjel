# What reaches a cloud model, and does pseudonymizing it help?

First written 2026-09-27.

## What the sources show

**People share personal data with chatbots in places nobody expects.** In real chatbot logs,
personal information appeared in translation requests 48% of the time and in code-editing
requests 16% of the time. The authors add that standard PII detection does not catch all of it
([Mireshghallah, Antoniak, More, Choi and Farnadi, Trust No Bot, arXiv:2407.11438](https://arxiv.org/abs/2407.11438)).

## Open challenge

**Replacing names does not stop inference.** Large models inferred personal attributes such as
location and income from text "with up to 85% top-1 and 95% top-3 accuracy", at about 100× less
cost and 240× less time than human annotators. The authors found "text anonymization and model
alignment" "currently ineffective" against this
([Staab, Vero, Balunović and Vechev, Beyond Memorization, arXiv:2310.07298](https://arxiv.org/abs/2310.07298)).
A pseudonymized request can still reveal the person it is about.

## What the sources do not show

- How much a Sjel request reveals after pseudonymization. Not measured.
