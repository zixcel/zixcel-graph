# Using zixcel-graph

Store a reviewed set of relationships as an exact snapshot and update it with revision checks.

## Before you start

Applications determine the meaning and authority of graph data. Storing a relationship does not make it authoritative.

## First steps

Run from the repository root:

```sh
cargo test --locked
```

## How to assess the result

- Create graph spaces and immutable snapshots.
- Apply bounded changes and detect competing updates.

A passing source-level check establishes only what that check observes. Keep missing configuration, unavailable services and unverified deployment paths visible.

## Continue reading

[Repository overview](../README.md)
