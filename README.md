# zixcel-graph

Store a reviewed set of relationships as an exact snapshot and update it with revision checks.

## What you can do

- Create graph spaces and immutable snapshots.
- Apply bounded changes and detect competing updates.

## Current scope

Applications determine the meaning and authority of graph data. Storing a relationship does not make it authoritative.

Package distribution is not activated by this documentation. Use the checked-in source and the declared dependency versions; published availability must be verified separately.

## Getting started

Install Rust 1.97 or newer and make the declared dependencies available. Use the configured private registry when a dependency is not distributed publicly. Run from this repository:

```sh
cargo test --locked
```

## Examples and interface details

## Exact cross-space publication (0.10.0 development contract)

`read_exact` returns one immutable Graph snapshot and its covering `RevisionRef`.
`PublicationRequest::with_dependencies` binds up to eight unique, canonically
ordered `GraphReadDependency` values into exact request identity. The target base
already fences the target; target-space dependencies and duplicate spaces reject.
Committed exact replay precedes source freshness. For a new publication, source
revisions are compared inside the same physical writer transaction as target
publication. A mismatch returns `GraphError::DependencyConflict` with source,
expected and actual references, without target mutation or receipt.

Calculation and immutable durability callbacks execute without internal storage
locks. Final CAS may lose after speculative preparation: callbacks are not an
acceptance signal and must not publish mutable owner state or provider effects.
Only the returned canonical receipt proves acceptance. Existing domain consumers
must establish their own preparation/visibility contract before adopting this
change. There is no recursive mutex, auxiliary Handle or read repair. Serialized
requests/publications declare dependencies explicitly; old images are not decoded
through defaults or compatibility fallback. No user-state migration is implicit.

An embedded typed property multigraph in Rust using the independent `zixcel-revision` commit foundation. Memory and redb backends share one Rust API. Callers supply application vocabulary and compose storage with their runtime.

## Features and ownership

- Default / `default-features = false`: no graph implementation. Revision-only
  consumers depend directly on `zixcel-revision`; no `commit` re-export exists.
- `redb`: persistent backend support; select `graph` for the Graph API.
- `graph`: property graph and memory storage.
- `graph,redb`: property graph with persistent storage.

Graph consumers must select their features explicitly; no legacy default alias
is retained. All development packages remain `0.10.0`.
See the `zixcel-revision` package's `docs/commit-foundation.md` for atomic boundaries, limits,
owner obligations, and the independent archive conformance executable.
The Graph-owned `publish` port atomically stores reference changes and an actual
Foundation receipt. Ordinary `write/commit` and durable `prepare_publication /
publish_prepared` use this same lifecycle; the old callback CAS API is removed.

```rust
use zixcel_graph::{Edge, Graph, GraphSpace, Node, GraphChanges, PublicationRequest};
use sha2::{Digest, Sha256};

let graph = Graph::create("state.graph")?;
let space = GraphSpace::new("application")?;
let command = "create example nodes a and b and directed example.link a-b";
let request = PublicationRequest {
    operation_id: "example/1".into(),
    expected_commit_revision: graph.commit_revision(&space)?,
    request_digest: format!("{:x}", Sha256::digest(command)),
};
let result = graph.publish(&space, &request, |_current| Ok(GraphChanges {
    nodes: vec![Node::new("a", ["example.node"])?, Node::new("b", ["example.node"])?],
    edges: vec![Edge::directed("a-b", "a", "b", "example.link")?],
    output: b"a,b".to_vec(),
    ..Default::default()
}), || Ok(()))?;
// Match result.outcome exhaustively. A Foundation Conflict is not semantic uncertainty.
# Ok::<(), zixcel_graph::GraphError>(())
```

A Graph Space is only a storage namespace. Higher adapters map subjects, HATs, workspaces, devices and repositories to labels/properties; this crate does not interpret their meaning.

Use `Graph::open_existing(path)` to read existing data. Missing or incomplete databases fail without creating directories or tables.
Read-only redb handles never repair on open. After an unclean shutdown, explicit
`Graph::recover_existing` performs backend recovery, without logical initialization.
`Failure::RecoveryRequired` preserves the backend's explicit `RepairAborted`
signal separately from `Failure::Corrupt` (invalid stored image/format). It does
not claim which process crashed or prove that the logical image is recoverable.
Its retry class is `None`: readers and UI rendering must not repair or retry it.
Explicit owner startup uses `Graph::start_existing(path, observe)` before serving
queries. This is a lifecycle mutation boundary, not a read API. It attempts a
normal read-only open and recovers only `RecoveryRequired`, validates all Graph
spaces and revision images, closes the recovery writer and reopens read-only.
It reports `Opening → [RecoveryRequired → Recovering] → Ready`, or `Unavailable`
with the original typed failure. Missing storage is `GraphError::Missing`;
corruption, recovery-required and storage failures remain distinct. No missing
database, schema, space or domain record is initialized. Prepared state is neither
published nor deleted. Logical head/receipt/Graph equality, not physical DB byte
equality, is the recovery invariant.

The owner obtains an exclusive non-blocking OS lease on the canonical database
path's empty `.lifecycle.lock` inode. This is coordination only, not another
canonical state machine. Readers never create it; while recovery owns it, readers
and competing starts receive `GraphError::Unavailable`. Symlink/hard-link aliases
are rejected. The lease is released after writer close and read-only reopen;
redb continues to enforce handle-level reader/writer exclusion. The explicit
`recover_existing` maintenance primitive also takes this startup lease. Recovery
failure returns no usable Graph, and interruption permits a later explicit owner
start. Standalone revision consumers retain their own recovery policy.

`startup::tests::process_matrix` tests real child process graceful/forced shutdown,
exact whole logical images, competing processes, interruption inside the actual
physical recovery sync, and injected recovery IO failure. Graceful owners stop
intake, drain accepted writes and drop
all writer handles before reporting normal shutdown. Forced process termination
does not make that guarantee. See `tests/shutdown_barriers.rs` for bounded child
process EOF/TERM/KILL/crash checks at exact prepared/committed barriers.
`Graph::create(path)` explicitly permits creation. It does not initialize existing databases; it preserves migration records and validates every table and version without silently repairing missing data.

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

## Documentation and source

[Interface reference](docs/interface-reference.md)

[Usage guide](docs/getting-started.md)

[Detailed documentation](docs) · [Implementation and public interfaces](src) · [Verification cases](tests) · [Contributing](CONTRIBUTING.md) · [Security reporting](SECURITY.md) · [License](LICENSE) · [Attribution notices](NOTICE)
