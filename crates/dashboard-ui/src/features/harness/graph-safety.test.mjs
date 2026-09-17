import test from 'node:test';
import assert from 'node:assert/strict';
import { validateGraph, toFlow, aggregate, canResolve } from './graph-model.mjs';

const work = (id, deps = []) => ({ id, kind: { type: 'work', agent: 'worker', prompt: 'task' }, depends_on: deps.map(node => ({ node })) });
const graph = (nodes = [work('a')]) => ({ version: 1, name: 'test', max_parallel: 4, nodes });
const checkpoint = (g = graph()) => ({
  version: 1, run_id: '3521c4b7-99c9-4a0a-a69e-a39a665a95df', revision: 3,
  definition_digest: 'a'.repeat(64), scope_digest: 'b'.repeat(64),
  budget: { limit: 100, spent: 10, reserved: 0, overdrawn: false },
  nodes: Object.fromEntries(g.nodes.map(n => [n.id, { status: 'pending', attempts: 0, output: null, error: null }]))
});

for (const [name, edit] of [
  ['graph unknown field', g => { g.tenant_id = 'untrusted'; }],
  ['node unknown field', g => { g.nodes[0].tool_allowlist = ['Bash']; }],
  ['kind unknown field', g => { g.nodes[0].kind.verifier = 'fake'; }],
  ['edge unknown field', g => { g.nodes.push(work('b', ['a'])); g.nodes[1].depends_on[0].unless = true; }],
  ['null concurrency', g => { g.max_parallel = null; }],
  ['null join', g => { g.nodes[0].join = null; }],
  ['null attempts', g => { g.nodes[0].max_attempts = null; }],
  ['null dependencies', g => { g.nodes[0].depends_on = null; }],
  ['null edge condition', g => { g.nodes.push(work('b', ['a'])); g.nodes[1].depends_on[0].on = null; }],
  ['UTF-8 graph name exceeds Rust byte limit', g => { g.name = '图'.repeat(43); }],
  ['agent exceeds Rust byte limit', g => { g.nodes[0].kind.agent = 'a'.repeat(129); }],
  ['prompt exceeds Rust byte limit', g => { g.nodes[0].kind.prompt = '界'.repeat(43691); }],
  ['question exceeds Rust byte limit', g => { g.nodes[0].kind = { type: 'human', question: 'a'.repeat(8193) }; }],
  ['verifier exceeds Rust byte limit', g => { g.nodes[0].kind = { type: 'gate', verifier: 'a'.repeat(129) }; }],
]) {
  test(`transport rejects ${name}`, () => {
    const g = graph(); edit(g);
    assert.throws(() => validateGraph(g), Error);
  });
}

for (const [name, edit] of [
  ['missing nodes', c => { delete c.nodes.a; }],
  ['extra nodes', c => { c.nodes.other = c.nodes.a; }],
  ['invalid run ID', c => { c.run_id = 'not-a-run'; }],
  ['unsafe revision', c => { c.revision = Number.MAX_SAFE_INTEGER + 1; }],
  ['bad digest', c => { c.definition_digest = 'unbound'; }],
  ['missing budget', c => { delete c.budget; }],
  ['negative attempts', c => { c.nodes.a.attempts = -1; }],
  ['missing output', c => { delete c.nodes.a.output; }],
]) {
  test(`checkpoint rejects ${name}`, () => {
    const g = graph(), c = checkpoint(g); edit(c);
    assert.throws(() => toFlow(g, c), Error);
  });
}

test('aggregate marks an unstarted checkpoint pending', () => assert.equal(aggregate(checkpoint()), 'pending'));
test('aggregate handles null node state without crashing', () => assert.equal(aggregate({ nodes: { a: null } }), 'invalid'));
test('approval cannot resolve a work node merely marked waiting', () => {
  const g = graph(), c = checkpoint(g); c.nodes.a.status = 'waiting';
  assert.equal(canResolve(c, 'a', 3, g), false);
});

test('kind type cannot exploit JS property-name coercion', () => {
  const g = graph(); g.nodes[0].kind.type = ['work'];
  assert.throws(() => validateGraph(g));
});
