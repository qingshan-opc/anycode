/** Versioned editor transport. These checks do not replace host authentication or ACLs. */
export const statuses = new Set(['pending', 'running', 'completed', 'skipped', 'partial', 'failed', 'waiting', 'uncertain', 'cancelled']);
const kindKeys = {
  work: ['type', 'agent', 'prompt'],
  gate: ['type', 'verifier'],
  branch: ['type', 'source', 'pointer', 'equals'],
  human: ['type', 'question'],
};
const own = (obj, key) => Object.prototype.hasOwnProperty.call(obj, key);
const object = (v) => v !== null && typeof v === 'object' && !Array.isArray(v)
  && [Object.prototype, null].includes(Object.getPrototypeOf(v));
const fail = (message) => { throw new Error(message); };
const encoder = new TextEncoder();
const idPattern = /^[A-Za-z0-9_-]{1,64}$/;
const digestPattern = /^[a-fA-F0-9]{64}$/;
const uuidPattern = /^[a-fA-F0-9]{8}-(?:[a-fA-F0-9]{4}-){3}[a-fA-F0-9]{12}$/;

function record(value, allowed, required, label) {
  if (!object(value) || Object.keys(value).some(key => !allowed.includes(key))
    || required.some(key => !own(value, key))) fail(`${label}: missing or unknown fields`);
}
// Defaults apply only to ABSENT fields, never explicit null/undefined.
const optional = (value, key, fallback) => own(value, key) ? value[key] : fallback;
function text(value, maxBytes, label) {
  if (typeof value !== 'string' || !value.length || encoder.encode(value).length > maxBytes) {
    fail(`${label}: invalid UTF-8 byte length`);
  }
}
function integer(value, min, max, label) {
  if (!Number.isSafeInteger(value) || value < min || value > max) fail(`${label}: invalid integer`);
}
function jsonValue(value, ancestors = new Set(), depth = 0) {
  // Browser input is stricter than an arbitrary JS object. No cycles, getters,
  // undefined, NaN or BigInt can be silently changed by JSON.stringify.
  if (depth > 64) fail('JSON value exceeds editor depth limit (64)');
  if (value === null || typeof value === 'boolean' || typeof value === 'string') return;
  if (typeof value === 'number' && Number.isFinite(value)) return;
  if ((!Array.isArray(value) && !object(value)) || ancestors.has(value)) fail('Invalid JSON value');
  ancestors.add(value);
  if (Array.isArray(value)) {
    for (const item of value) jsonValue(item, ancestors, depth + 1);
  } else {
    for (const key of Object.keys(value)) {
      const field = Object.getOwnPropertyDescriptor(value, key);
      if (!field || !own(field, 'value')) fail('JSON getters are not supported');
      jsonValue(field.value, ancestors, depth + 1);
    }
  }
  ancestors.delete(value);
}

export function validateGraph(graph) {
  record(graph, ['version', 'name', 'nodes', 'max_parallel'], ['version', 'name', 'nodes'], 'Graph');
  text(graph.name, 128, 'Graph name');
  if (graph.version !== 1 || !graph.name.trim()) fail('Graph version/name invalid');
  if (!Array.isArray(graph.nodes) || graph.nodes.length === 0 || graph.nodes.length > 512) fail('Graph needs 1..512 nodes');
  integer(optional(graph, 'max_parallel', 1), 1, 32, 'Concurrency');
  const nodes = new Map();
  for (const n of graph.nodes) {
    record(n, ['id', 'kind', 'depends_on', 'join', 'max_attempts'], ['id', 'kind'], 'Node');
    if (typeof n.id !== 'string' || !idPattern.test(n.id) || nodes.has(n.id)) fail('Duplicate/invalid node ID');
    if (!object(n.kind) || typeof n.kind.type !== 'string' || !own(kindKeys, n.kind.type)) fail('Unknown node kind');
    record(n.kind, kindKeys[n.kind.type], kindKeys[n.kind.type], 'Node kind');
    if (!['all', 'any'].includes(optional(n, 'join', 'all'))) fail('Invalid join mode');
    integer(optional(n, 'max_attempts', 1), 1, 3, 'Retry count');
    switch (n.kind.type) {
      case 'work': text(n.kind.agent, 128, 'Agent'); text(n.kind.prompt, 128 * 1024, 'Prompt'); break;
      case 'gate': text(n.kind.verifier, 128, 'Verifier'); break;
      case 'human': text(n.kind.question, 8192, 'Question'); break;
      case 'branch':
        if (typeof n.kind.source !== 'string' || !idPattern.test(n.kind.source)
          || typeof n.kind.pointer !== 'string' || (n.kind.pointer !== '' && !n.kind.pointer.startsWith('/'))) {
          fail('Branch needs source and JSON Pointer');
        }
        jsonValue(n.kind.equals);
        break;
    }
    nodes.set(n.id, n);
  }
  for (const n of nodes.values()) {
    const deps = optional(n, 'depends_on', []);
    if (!Array.isArray(deps)) fail('Dependencies must be an array');
    const seen = new Set();
    for (const d of deps) {
      record(d, ['node', 'on'], ['node'], 'Dependency');
      if (!nodes.has(d.node) || d.node === n.id || seen.has(d.node)) fail('Missing/self/duplicate dependency');
      seen.add(d.node);
      const on = optional(d, 'on', 'completed');
      if (!['completed', 'true', 'false'].includes(on)) fail('Unknown edge condition');
      if (on !== 'completed' && nodes.get(d.node).kind.type !== 'branch') fail('Boolean edge must originate at branch');
    }
    if (n.kind.type === 'branch' && !seen.has(n.kind.source)) fail('Branch source must be a direct dependency');
  }
  const done = new Set(), layers = [];
  while (done.size < nodes.size) {
    const next = [...nodes.values()].filter(n => !done.has(n.id) && (n.depends_on ?? []).every(d => done.has(d.node))).map(n => n.id);
    if (!next.length) fail('Cycles require a bounded-iteration extension; v1 is a DAG');
    layers.push(next); next.forEach(id => done.add(id));
  }
  return { graph, layers };
}

/** Validate a COMPLETE host snapshot; never invent pending states for missing nodes.
 * Digests are shape-checked only. The server must verify definition/scope binding.
 */
export function validateCheckpoint(checkpoint, graph) {
  validateGraph(graph);
  const keys = ['version', 'run_id', 'definition_digest', 'scope_digest', 'revision', 'nodes', 'budget'];
  record(checkpoint, keys, keys, 'Checkpoint');
  if (checkpoint.version !== 1 || typeof checkpoint.run_id !== 'string' || !uuidPattern.test(checkpoint.run_id)
    || typeof checkpoint.definition_digest !== 'string' || !digestPattern.test(checkpoint.definition_digest)
    || typeof checkpoint.scope_digest !== 'string' || !digestPattern.test(checkpoint.scope_digest)) fail('Checkpoint identity/digest invalid');
  integer(checkpoint.revision, 0, Number.MAX_SAFE_INTEGER, 'Checkpoint revision');
  const budgetKeys = ['limit', 'spent', 'reserved', 'overdrawn'];
  record(checkpoint.budget, budgetKeys, budgetKeys, 'Checkpoint budget');
  integer(checkpoint.budget.limit, 1, Number.MAX_SAFE_INTEGER, 'Budget limit');
  for (const key of ['spent', 'reserved']) integer(checkpoint.budget[key], 0, Number.MAX_SAFE_INTEGER, `Budget ${key}`);
  if (typeof checkpoint.budget.overdrawn !== 'boolean') fail('Budget overdrawn must be boolean');
  if (!object(checkpoint.nodes) || Object.keys(checkpoint.nodes).length !== graph.nodes.length) fail('Checkpoint node set mismatch');
  for (const n of graph.nodes) {
    if (!own(checkpoint.nodes, n.id)) fail('Checkpoint node set mismatch');
    const state = checkpoint.nodes[n.id];
    record(state, ['status', 'attempts', 'output', 'error'], ['status', 'attempts', 'output'], 'Node state');
    if (!statuses.has(state.status)) fail('Unknown checkpoint state');
    integer(state.attempts, 0, n.max_attempts ?? 1, 'Node attempts');
    if (own(state, 'error') && state.error !== null && typeof state.error !== 'string') fail('Node error must be string/null');
    jsonValue(state.output);
  }
  return checkpoint;
}

export function toFlow(graph, checkpoint = null) {
  const { layers } = validateGraph(graph);
  if (checkpoint !== null) validateCheckpoint(checkpoint, graph);
  const positions = new Map();
  layers.forEach((layer, x) => layer.forEach((id, y) => positions.set(id, { x: x * 285, y: y * 120 })));
  const nodes = graph.nodes.map(n => {
    const status = checkpoint === null ? 'pending' : checkpoint.nodes[n.id].status;
    return { id: n.id, position: positions.get(n.id), data: { label: `${n.id} · ${n.kind.type} · ${status}`, status, kind: n.kind.type }, type: 'default' };
  });
  const edges = graph.nodes.flatMap(n => (n.depends_on ?? []).map(d => ({
    id: `${d.node}->${n.id}`, source: d.node, target: n.id, label: d.on ?? 'completed',
    animated: checkpoint !== null && checkpoint.nodes[n.id].status === 'running',
  })));
  return { nodes, edges };
}

export function aggregate(checkpoint) {
  if (!object(checkpoint) || !object(checkpoint.nodes)) return 'invalid';
  const states = Object.values(checkpoint.nodes);
  if (!states.length || states.some(n => !object(n) || !statuses.has(n.status))) return 'invalid';
  const values = states.map(n => n.status);
  for (const state of ['uncertain', 'failed', 'partial', 'cancelled', 'waiting']) if (values.includes(state)) return state;
  if (values.every(v => ['completed', 'skipped'].includes(v))) return 'completed';
  return values.includes('running') ? 'running' : 'pending';
}
export function parseGraph(text) {
  if (typeof text !== 'string' || encoder.encode(text).length > 2 * 1024 * 1024) fail('Graph JSON exceeds 2 MiB');
  const graph = JSON.parse(text); validateGraph(graph); return graph;
}
export function prepareGraphStart(text, validatedText, checkpoint = null) {
  if (checkpoint !== null) fail('Active definition is immutable; start a new run to edit');
  if (text !== validatedText) fail('Graph draft changed; validate the current JSON before starting');
  return parseGraph(text);
}
export function canResolve(checkpoint, nodeId, revision, graph) {
  try {
    validateCheckpoint(checkpoint, graph);
    return Number.isSafeInteger(revision) && revision > 0 && checkpoint.revision === revision
      && graph.nodes.some(n => n.id === nodeId && n.kind.type === 'human')
      && checkpoint.nodes[nodeId].status === 'waiting';
  } catch { return false; }
}
