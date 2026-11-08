# Finance: investment ideas and research

Ledger, import and price work stays in [data](data.md#5-domain-backlog). Current contracts
and design rationale belong to [finance](../../crates/finance/README.md).

## Shipped

- Ideas, triggers, research routing, model reviews and the Mac Ideas view are implemented;
  their write schemas are published (`904c04ff`). The earlier app-implementation task is closed.
- The first live model review is recorded in `a63cdb6c`; it does not establish recommendation quality.

## Open

1. Accept the Ideas view in the running app: create, edit, close and reopen an idea; inspect
   its history, price levels, matched research and review verdict. Compilation does not close
   this check.
2. Add Reddit comments and scores only after an operator registers OAuth access and provisions
   its credential. Until then, the existing public RSS adapter supplies titles and post bodies.
3. Measure the 30-day and 90-day review track record after enough outcomes exist. Report sample
   size and distinguish missing prices from scored proposals; do not infer quality from a live call.

## Constraints

Ideas remain finance-owned. Model output is a proposal requiring a human verdict; Sjel places
no trades. Research must cite supplied source items. Provider access follows the existing
finance configuration and data-admission rules; this maintenance pass changes neither.
