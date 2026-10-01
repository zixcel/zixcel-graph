# Contributing

## Verification environment

Use Rust 1.97 or later, as declared in Cargo.toml. Keep Cargo.lock unchanged unless
a dependency update is part of the contribution. Never put registry credentials,
customer data or runtime state in source, examples, issues or pull requests.

The current manifest requires zixcel-revision =0.10.0 from the zixcel-private
registry. A standalone clone cannot resolve this dependency until the owner
provides an authorized registry configuration. Registry credentials belong in
local credential storage, not committed configuration or public CI logs.

In an authorized environment, run:

```sh
cargo test --locked
cargo test --locked --all-features
```

The first command covers the default feature boundary; the second enables Graph
and persistent storage. Neither command is a secret scan or a backup test.

## Architecture test migration

The current tests/architecture.test.mjs still reads src/commit/* and src/durable.rs,
which are absent from this repository after the Revision extraction. It also
expects a redb feature declaration without the new zixcel-revision/redb entry.
Update those assertions to the independent package boundary before treating
`node --test tests/architecture.test.mjs` as a passing acceptance check. Do not
copy the extracted Revision implementation back here to satisfy stale tests.

## Pull request evidence

Describe the input that triggers the problem, resulting behavior, feature sets
and checks actually run. State unavailable registry or runtime validation
explicitly. Storage, recovery and retention changes require relevant existing
runtime tests; static architecture assertions supplement those tests.
