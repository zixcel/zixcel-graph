// C4 negative boundaries supplement (never replace) runtime acceptance.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
const root = new URL('../', import.meta.url)
const read = name => fs.readFileSync(new URL(name, root), 'utf8')
test('Foundation stays generic, feature-minimal and cannot delete owner data', () => {
  const files = ['model.rs', 'state.rs', 'store.rs', 'retention.rs', 'backend.rs', 'backend/durable.rs']
  for (const file of files) {
    const code = read(`src/commit/${file}`).replace(/\/\/[^\n]*/gu, '')
    assert.doesNotMatch(code, /SemanticBinding|RuntimeRole|ExecutionGrant|CapabilityActivation|FsRepository|hatter::|sem_lang::/u, file)
    assert.doesNotMatch(code, /remove_file|remove_dir|unlink|SystemTime|chrono::/u, file)
  }
  assert.match(read('Cargo.toml'), /default = \[\]/u)
  assert.match(read('Cargo.toml'), /graph = \["dep:serde_json"\]/u)
  assert.match(read('Cargo.toml'), /redb = \["dep:redb", "dep:serde_json"\]/u)
})
test('Graph delegates lifecycle, while read-only open and physical recovery are separated', () => {
  for (const file of ['src/backend/mod.rs', 'src/backend/memory.rs', 'src/backend/redb.rs']) {
    assert.doesNotMatch(read(file), /fn commit\(|expected_revision: u64|commit_prepared/u, file)
  }
  assert.doesNotMatch(read('src/transaction.rs'), /commit_prepared|RevisionConflict/u)
  assert.match(read('src/transaction.rs'), /\.publish\(/u)
  assert.match(read('src/publication.rs'), /self\.replay\(space, request\)/u)
  assert.match(read('src/durable.rs'), /ReadOnlyDatabase::open/u)
  assert.match(read('src/durable.rs'), /fn recover\(/u)
  const open = read('src/backend/redb.rs').split('fn existing(')[1].split('fn initialize(')[0]
  assert.doesNotMatch(open, /initialize\(|create_dir|create_new|remove_|begin_write/u)
})
