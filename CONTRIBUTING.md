# Contributing

Discuss substantial API or architecture changes in an issue before implementation.
Keep changes focused, explain observable behavior, and include relevant tests and
validation evidence. Use the package's documented interfaces and versioned dependencies.
Application-specific composition belongs to callers.

Create a topic branch and open a pull request against main. Direct changes to main,
force pushes, and deleting main are restricted. Reviewers check correctness,
compatibility, licensing, security, and documentation. Resolve review conversations
before merging. Maintainers may request changes or decline a contribution.

Run the checks documented in README and the repository's security policy.
Do not commit credentials, customer or personal data, local registration/state,
production certificates or keys, generated archives, caches, or build outputs.
Use synthetic fixtures with documented provenance. Report vulnerabilities privately
as described in SECURITY.md. Passing the repository policy check does not replace
functional testing, secret inspection, or dependency analysis.

By submitting a contribution, you confirm that you are authorized to provide it
under the repository's applicable license. Preserve third-party notices and
existing permissions. Contributors retain their copyrights; submission does not
transfer ownership. Explain any new dependency and its license.

Follow CODE_OF_CONDUCT.md. Maintainers decide releases and compatibility policy;
a merged pull request does not itself promise a release or support commitment.

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
