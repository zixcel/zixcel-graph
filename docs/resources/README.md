# Graph resource boundary

Durable reads range over the exact space prefix instead of scanning unrelated node,
edge and derived-index tables. Durable publication removes only deleted entries
inside that space, and writes node/edge bytes only when changed. Unchanged index
keys are retained. Existing revision, atomic commit, conflict and recovery checks
remain in place. Memory backend remains supported for ephemeral consumers.

Run `cargo run --locked --offline --features graph,redb --example resource-budget -- 1000 4`.
It uses a new temporary DB, fixed chain seed 0, 128-byte attributes, explicit small
fixture bounds and cleanup. No existing DB or customer data is opened. The fixture
emits wall time, warm read p50/p95 over 20 samples, Linux process peak RSS, OS write
bytes and DB size. Those are process/OS observations, not allocator attribution,
backend cache isolation or application latency guarantees.

The checked-in development observations are three shared-host debug samples with
concurrent compilation. They do not establish an improvement ratio or production
resource budget. The differential sample reduces observed update write bytes but
has higher measured latency; causal performance claims require isolated repeated
runs. Current Graph transactions still materialize a complete selected SpaceState;
space range reads and differential writes do not solve that RAM constraint.

Remaining owner work is tracked in coela-oss/coela-release#113: bounded load/storage
API, allocator and cache attribution, cold/warm repetition, cancellation/ENOSPC and
crash acceptance, disk admission, target-device budgets and product projection
rebuild. No OOM, partial commit or memory fallback is an acceptable quota policy.

## Explicit per-read admission

`Graph::read_budgeted(space, ReadBudget { max_nodes, max_edges,
max_index_entries, max_source_bytes })` is source API pending release. It returns
an entire valid selected-space snapshot or the existing typed `Failure::Capacity`.
Redb charges source bytes and record counts before decoding each record, including
the commit image; memory checks the stored representation before cloning. Index
keys also consume the explicit budget. The caller supplies limits; there is no
claim that a particular limit fits a hardware class. No partial result, DB write,
repair or memory fallback occurs on refusal. Existing unbudgeted APIs, writes,
startup validation, snapshot retention and allocator overhead remain outside this
per-read admission contract. Use external process limits for total RSS until those
separate resource paths have explicit budgets too.

`Graph::read_exact_budgeted(space, budget)` applies the same admission to data
and the covering canonical revision from one physical storage snapshot. A refusal
returns neither data nor revision; retained earlier snapshots keep their earlier
values. This API is also unreleased. Neither API bounds the number of snapshots a
consumer retains, or replaces the outstanding write/startup resource work.

## Explicit write admission

`Graph::write_budgeted(space, WriteBudget { snapshot, max_updates,
max_update_bytes })` is an unreleased source API. The initial snapshot is admitted
before decoding/cloning. Mutation calls cumulatively consume count and streamed
JSON byte limits before their input is retained. Replacements/removals count too.
Any refused mutation poisons the transaction, so an already staged prefix cannot
be committed. The final state, derived indexes and canonical receipt image must
fit `snapshot` limits before publication; the backend checks these limits again
under its atomic writer fence. An existing concurrent revision is never overwritten.

Run `cargo run --locked --offline --features graph --example budgeted-write` for
an explicit synthetic use case. Five memory/durable journeys check normal writes,
quota refusal, unchanged canonical values/revisions, retained snapshots, attached
edge errors, final receipt bytes and concurrent conflicts. Inputs are caller-owned
before the API receives them; this contract bounds retained source/mutation data,
not allocator overhead, total RSS, database cache, startup, snapshot retention or
aggregate space count. These remaining resource paths stay in #113.
