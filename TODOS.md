# TODOS

Deferred work from the 2026-05-21 CEO review of the grooming substrate.
Full strategy: [doc/designs/grooming-substrate.md](doc/designs/grooming-substrate.md).

## Deferred (post grooming-substrate cycle)

### Delivery tree (milestones as a second hierarchy) — P2
- **What:** A second hierarchy orthogonal to the concept tree: the "tree of delivery"
  (what ships, in what order) vs the "tree of concept" (what the system is). A spec can
  serve several milestones; a milestone pulls items from many concept branches.
- **Why:** The most distinctive idea in the product vision; sets rumb apart from flat
  trackers. Deferred because it is the single biggest subsystem (new `milestones` table,
  many-to-many item association, milestone hierarchy + rollup, its own lifecycle) and
  deserves its own design pass.
- **Effort:** L (human ~2-3wk / CC ~3-4d). **Depends on:** the malleable concept tree.

### Parallel frontier + ordinal edges — P2
- **What:** `edges.ordinal` + a `reorder` verb + `rumb frontier` (the claimable,
  non-blocking set sized for parallel agent pickup).
- **Why:** Surfaces parallelism for real speed gains across multiple agents.
- **Effort:** M (human ~1wk / CC ~1d). **Depends on:** ordinal edges (schema change).

### `split` verb — P3
- **What:** Decompose one item into several, distributing children and edges.
- **Why:** The analysis/decomposition counterpart to `merge`. Deferred because the
  child/edge distribution semantics and the actual usage are unclear; needs design
  before it earns a place.
- **Effort:** M (human ~1wk / CC ~1d).

### History export / committable state — P2
- **What:** A way to export or commit the `.rumb` history (changesets/deltas/events) so
  it survives beyond one machine.
- **Why:** The "history teaches the next person" thesis only holds single-machine today
  because `.rumb` is gitignored local runtime state. Export/committed state unlocks
  cross-person teaching.
- **Effort:** M. **Depends on:** the history substrate landing first.

### Further future (from the design doc)
- Analysis/synthesis Git resolution bridge (many-to-many item ↔ PR/artifact resolution).
- Capability-tier model dispatch (route routine→cheap, hard→strong models).
- GTD generalization as a separate product (same engine, different product).
