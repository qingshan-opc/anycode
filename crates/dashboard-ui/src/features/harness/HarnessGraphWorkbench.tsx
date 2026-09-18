import { useMemo, useRef, useState } from 'react';
import ReactFlow, { Background, Controls, MiniMap } from 'reactflow';
import 'reactflow/dist/style.css';
import { aggregate, canResolve, parseGraph, prepareGraphStart, toFlow, type Graph, type Checkpoint } from './graph-model.mjs';

interface Props {
  /** Host must supply the canonical definition corresponding to checkpoint. */
  graph: Graph;
  checkpoint?: Checkpoint | null;
  /** Only an authenticated host/BFF may implement these callbacks. */
  onStart?: (graph: Graph) => Promise<void>;
  onResume?: (runId: string, revision: number) => Promise<void>;
  onResolve?: (runId: string, revision: number, nodeId: string, approved: boolean) => Promise<void>;
}

export function HarnessGraphWorkbench(props: Props) {
  // A different host graph/run must not reuse local editor state from a prior run.
  const key = JSON.stringify([props.checkpoint?.run_id ?? null, props.graph]);
  return <GraphEditor key={key} {...props} />;
}

function GraphEditor({ graph, checkpoint = null, onStart, onResume, onResolve }: Props) {
  const initialText = JSON.stringify(graph, null, 2);
  const [text, setText] = useState(initialText);
  const [validatedText, setValidatedText] = useState(initialText);
  const [current, setCurrent] = useState(graph);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const view = useMemo(() => {
    try { return { flow: toFlow(current, checkpoint), error: '' }; }
    catch (e) { return { flow: null, error: e instanceof Error ? e.message : 'Invalid graph snapshot' }; }
  }, [current, checkpoint]);
  const state = view.error ? 'invalid' : aggregate(checkpoint);
  const chosen = view.flow ? current.nodes.find(n => n.id === selected) : undefined;
  const dirty = text !== validatedText;
  const blocked = busy || !!view.error;
  const resumable = state === 'pending' || state === 'running';
  const resolvable = !!chosen && !!checkpoint && !view.error
    && canResolve(checkpoint, chosen.id, checkpoint.revision, current);

  async function action(fn: () => Promise<void>) {
    // React state updates alone do not guard two clicks before the next render.
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError('');
    try { await fn(); }
    catch (e) { setError(e instanceof Error ? e.message : 'Request failed'); }
    finally { inFlight.current = false; setBusy(false); }
  }
  function validate() {
    try {
      if (checkpoint) throw new Error('Active definition is immutable. Start a new run to edit.');
      setCurrent(parseGraph(text)); setValidatedText(text); setError('');
    } catch (e) { setError(e instanceof Error ? e.message : 'Invalid graph'); }
  }

  return <section aria-label="Harness graph" style={{ display: 'grid', gap: 12 }}>
    <header>
      <h2>AnyCode · Harness Graph</h2>
      <p>{checkpoint ? `Run ${checkpoint.run_id} · revision ${checkpoint.revision} · ${state}` : 'Not executed. Preview only until a host is connected.'}</p>
    </header>
    <div style={{ display: 'flex', gap: 8 }}>
      <button disabled={busy || !!checkpoint} onClick={validate}>Validate JSON</button>
      <button disabled={blocked || dirty || !onStart || !!checkpoint}
        onClick={() => onStart && action(() => onStart(prepareGraphStart(text, validatedText, checkpoint)))}>Start validated graph</button>
      <button disabled={blocked || !onResume || !checkpoint || !resumable}
        onClick={() => onResume && checkpoint && action(() => onResume(checkpoint.run_id, checkpoint.revision))}>Resume explicitly</button>
    </div>
    {dirty && !checkpoint && <p role="status">Draft changed. Validate the current JSON before starting.</p>}
    {(error || view.error) && <p role="alert">{error || view.error}</p>}
    {view.flow && <div style={{ height: 460, border: '1px solid', borderRadius: 8 }}>
      <ReactFlow nodes={view.flow.nodes} edges={view.flow.edges} fitView nodesConnectable={false} nodesDraggable={false} onNodeClick={(_, node) => setSelected(node.id)}>
        <Background /><MiniMap /><Controls />
      </ReactFlow>
    </div>}
    {chosen && !view.error && <aside>
      <h3>{chosen.id}</h3>
      <pre style={{ whiteSpace: 'pre-wrap' }}>{JSON.stringify(checkpoint?.nodes[chosen.id] ?? chosen, null, 2)}</pre>
      {resolvable && <div>
        <button disabled={blocked || !onResolve}
          onClick={() => onResolve && checkpoint && action(() => onResolve(checkpoint.run_id, checkpoint.revision, chosen.id, true))}>Approve this node</button>
        <button disabled={blocked || !onResolve}
          onClick={() => onResolve && checkpoint && action(() => onResolve(checkpoint.run_id, checkpoint.revision, chosen.id, false))}>Reject</button>
      </div>}
    </aside>}
    <label>Versioned graph definition<textarea value={text} disabled={busy || !!checkpoint} onChange={e => setText(e.target.value)} spellCheck={false} style={{ display: 'block', width: '100%', minHeight: 260, fontFamily: 'monospace' }} /></label>
  </section>;
}
