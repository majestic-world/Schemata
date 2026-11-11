## Language

- Write code and comments in English; respond to the user in Brazilian Portuguese.
- Inflect nouns for the actual count: `1 classe`, `2 classes`, rather than `classe(s)`.

## Commits

- Create commits only when the user requests them or a manually invoked skill explicitly instructs you to.
- Use Conventional Commits with a Brazilian Portuguese description.

## Validation and desktop interaction

- Preserve validation quality while minimizing disruption: prefer checks without desktop control; use the UI when needed to prove behavior.
- Prefer background desktop interaction. Group predictable actions into short sessions, take focus, mouse, or keyboard only as long as necessary, and release control after verification.
- In omp sessions, use omp's built-in computer use for desktop interaction. Necessary use is already authorized; proceed without another permission request, subject to tool-required confirmations.
- If omp's computer use is disabled or unavailable, notify the user once, continue with available non-desktop checks, and report pending UI validation. Leave desktop control to omp's computer use.
- Outside omp, use the environment's available tools with the same minimal-disruption policy; no omp-specific availability check or warning is needed.

## Agent skills

### Issue tracker

Issues and specs live under `.scratch/<feature>/`. Before creating, fetching, or updating tickets, read `docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage labels. Before triaging issues or changing their status, read `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: root `GLOSSARY.md` and `docs/adr/`. Before exploring the codebase, read `docs/agents/domain.md`.
