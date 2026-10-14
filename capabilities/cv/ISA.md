# ISA · cv

Capability-local claims and their falsifiers. The repo-wide `ISA.md` does not own this work.

## Claims

- [ ] ISC-67 — `cv linkedin pull` reads the LinkedIn Member Snapshot API on this machine.
  Operator action: create the developer app on the page "Member Data Portability (Member)
  Default Company", request the product "Member Data Portability API (Member)", generate a token
  with scope `r_dma_portability_self_serve`, and store it with
  `security add-generic-password -U -s sjel-linkedin-dma -a "$USER" -w`
  ([LinkedIn docs](https://learn.microsoft.com/en-us/linkedin/dma/member-data-portability/member-data-portability-member/)).
  Falsifier: `capabilities/cv/cv linkedin pull POSITIONS` exits non-zero or prints `0 record(s)`.
- [ ] ISC-68 — `cv linkedin diff` reads the real snapshot field names (`Headline`, `Summary`,
  `Company Name`, `Title`, `Started On`, `Finished On`, `Name`). They come from the API docs'
  PROFILE sample and the CSV export, not from a real response. Falsifier: after ISC-67, the diff
  prints `?` for a position that LinkedIn shows.
