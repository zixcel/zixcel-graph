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
