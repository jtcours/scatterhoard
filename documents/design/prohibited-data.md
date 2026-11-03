# Prohibited Data

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Scatterhoard should block certain sensitive information from the memory in both the conventions and, to the extent possible, in software, to prevent it from leaking into a project's repository:

- Secret API keys
- Passwords
- Credit card numbers

If someone needs to store this sort of sensitive data, they can put it in a file and let the software refer to the file.

## Resolved 2026-08-20: what the software half can and cannot do

Built as `src/prohibited.rs`, and the honest division is worth stating because *to the extent possible* above is doing real work:

| Category | Covered | How |
|---|---|---|
| Secret API keys | Partly | Known vendor prefixes (`sk-ant-`, `ghp_`, `AKIA`, `AIza`, `xoxb-`, …) followed by a minimum run of key characters, **where the prefix begins a word** -- not preceded by a letter, digit, `_` or `-`, so `ask-` and `task-` in a hyphenated slug are not keys (since 2026-10-16, `vendor-prefix-scan-matches-inside-words`). An unfamiliar vendor's format is not recognised. |
| Credit card numbers | Well | Digit runs of 13–19, separators allowed inside, satisfying the Luhn checksum. |
| Passwords | **Not at all** | A password has no shape. `hunter2` and four ordinary words are indistinguishable from prose, and a rule that guessed would fire on ordinary memories while still missing most real passwords. |

PEM private-key armour is recognised as well, which the list above does not name but which belongs to the same hazard.

**Every rule keys on structure, never on wording**, and that is a requirement rather than a preference. A memory set about a memory system discusses its own prohibitions: the `route-where` convention names all three categories in one sentence, so a vocabulary scan would refuse this repository's own export. Generic high-entropy detection is excluded for the same class of reason — a memory set is full of long opaque identifiers, and a threshold catching an unknown vendor's key catches memory ids and commit hashes too.

So the conventions half of this document is not a fallback for the software half; for passwords it is the whole of the protection. Where the check runs and why it refuses at a write but only warns at an export is settled in [Export Review](export-review.md).
