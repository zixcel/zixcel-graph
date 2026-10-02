# zixcel-graph interface reference

Use the [usage guide](getting-started.md) for the first steps. This reference preserves the current interface details and operational limits. Run command examples from the repository root, after preparing the exact declared dependencies and registered configuration.

## v1 boundary

- Primary Node/Edge records and derived label/adjacency indexes
- Atomic writes, concurrent snapshot reads and revision conflict detection
- Directed/undirected logical edges and parallel edges
- Label lookup, incoming/outgoing edges, neighbors and bounded traversal/path queries
- Deterministic index rebuild from primary records
- Explicit logical format/schema/index/backend metadata
- In-memory/redb backend parity

Custom pagers, B-trees, WALs, servers, network protocols, SQL/Cypher/SPARQL, full-text/vector search and domain inference are out of scope. redb types are not exposed in the public API.

## Atomic publication port (C3 prerequisite)

The caller fingerprints the complete formal input, including Subject, selected
references and decisions. This fingerprint is not the Foundation RevisionRef.
Graph calculates no authorization or semantic meaning.

`replay` checks known receipt/payload identity and new-operation admissibility on
a disposable read image, without canonical mutation. `publish` rechecks under
the actual storage writer. Known replay returns the original receipt before
the calculate/authorization/prepare callbacks; a new stale request returns the
Foundation Stale variant. The original graph delta can be read by its exact
receipt with `committed_changes`, independent of current records.

The memory writer and the single redb transaction publish graph nodes, edges,
indexes, Foundation image/head/receipt together. There is no additional database,
replay index owned by the graph layer, or external-effect transaction.
Receipt encoding is bounded by the Foundation 1 MiB payload limit; external
semantic memory stays in its owner's repository.

Immutable preparation can leave an external orphan after a later storage failure.
The repository alone owns actual reclamation, using Foundation reservation,
retention and fenced reclamation permits (see the commit contract).
Old databases without the current commits table reject instead of repairing or
reading a legacy fallback. Development data must be recreated explicitly.

## ZG1 external registration lifecycle

Commit receipts and DAG metadata remain immutable replay roots. External byte
availability is a separate owner-controlled association: `reserve_external`
registers before creation; `register_committed_external` explicitly registers
historical dependencies after the owner validates its exact original receipt and
available bytes. Neither read nor retain implicitly registers.

`external_registration` returns an exact preparation/object/generation fence.
`retain_registration` and `release_registration` use this fence. An explicit
`retire_external_registration` returns a distinct, stable `RetirementReceipt`,
rejects new holds, and preserves existing ones until they drain. Status is
Active → Retiring → Retired (Retiring may be skipped when there are no holds).
Shared objects remain protected until all associations and holds allow reclaim.
The original commit, parents, payload, head and receipt never change.

`reregister_external` is an explicit new generation, permitted only after the
old generation's holds drain and before a reclamation permit is issued. Old
retirement replay cannot affect the replacement; old releases cannot remove
replacement holds. Once physical reclamation is acknowledged, the immutable
object identity cannot be resurrected through reservation. Source materialization
is owner IO and may fail after reclamation; receipt replay still succeeds.

The existing unscoped C4 hold API remains valid for unretired associations. It
rejects an object with a retirement fence; shared/re-registered consumers must
use the exact generation API. Abandoned-preparation grace behavior is unchanged.
Graph never supplies source lifetime policy, authentication or lease expiry.

All metadata uses the existing atomic backend image. There are at most 256 active
external objects, 64 associations per object, 64 total holds per object and 4096
retirement receipts/tombstones. Capacity rejects without eviction; generations
use a checked persisted monotonic counter. These tombstones hold references, not
external bytes. No receipt/DAG TTL, scheduler or independent retention database.

The development commit image format is 2; previous layouts are rejected, not
silently defaulted/migrated. The crate version remains 0.10.0. Exact historical
registration within a valid current image is distinct from storage migration.
