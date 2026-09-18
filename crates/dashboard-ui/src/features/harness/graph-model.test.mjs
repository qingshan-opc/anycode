import test from 'node:test';import assert from 'node:assert/strict';
import {validateGraph,toFlow,aggregate,parseGraph,canResolve,prepareGraphStart} from './graph-model.mjs';
const work=(id,deps=[])=>({id,kind:{type:'work',agent:'worker',prompt:'task'},depends_on:deps.map(node=>({node}))});
const graph=(nodes)=>({version:1,name:'test',max_parallel:4,nodes});
const checkpoint=(g)=>({version:1,run_id:'3521c4b7-99c9-4a0a-a69e-a39a665a95df',revision:3,
 definition_digest:'a'.repeat(64),scope_digest:'b'.repeat(64),budget:{limit:100,spent:10,reserved:0,overdrawn:false},
 nodes:Object.fromEntries(g.nodes.map(n=>[n.id,{status:'pending',attempts:0,output:null,error:null}]))});
test('linear topological layers',()=>assert.deepEqual(validateGraph(graph([work('a'),work('b',['a'])])).layers,[['a'],['b']]));
test('siblings share one layer',()=>assert.deepEqual(validateGraph(graph([work('a'),work('b')])).layers,[['a','b']]));
test('cycle rejected',()=>assert.throws(()=>validateGraph(graph([work('a',['b']),work('b',['a'])]))));
test('duplicate node rejected',()=>assert.throws(()=>validateGraph(graph([work('a'),work('a')]))));
test('unknown predecessor rejected',()=>assert.throws(()=>validateGraph(graph([work('a',['missing'])]))));
test('self dependency rejected',()=>assert.throws(()=>validateGraph(graph([work('a',['a'])]))));
test('boolean edge cannot come from work',()=>{const b=work('b',['a']);b.depends_on[0].on='true';assert.throws(()=>validateGraph(graph([work('a'),b])));});
test('branch needs source as dependency',()=>assert.throws(()=>validateGraph(graph([{id:'b',kind:{type:'branch',source:'a',pointer:'/ok',equals:true}},work('a')]))));
test('flow reflects uncertain instead of success',()=>{const g=graph([work('a')]),c=checkpoint(g);c.nodes.a.status='uncertain';const f=toFlow(g,c);assert.equal(f.nodes[0].data.status,'uncertain');});
test('partial is not completed',()=>assert.equal(aggregate({nodes:{a:{status:'completed'},b:{status:'partial'}}}),'partial'));
test('all skipped/completed means completed',()=>assert.equal(aggregate({nodes:{a:{status:'completed'},b:{status:'skipped'}}}),'completed'));
test('missing state is invalid',()=>assert.equal(aggregate({nodes:{}}),'invalid'));
test('unknown checkpoint state is rejected',()=>{const g=graph([work('a')]),c=checkpoint(g);c.nodes.a.status='magic';assert.throws(()=>toFlow(g,c));});
test('human resolution needs matching revision',()=>{const g=graph([{id:'a',kind:{type:'human',question:'Approve?'}}]),c=checkpoint(g);c.nodes.a.status='waiting';assert.equal(canResolve(c,'a',3,g),true);assert.equal(canResolve(c,'a',2,g),false);});
test('JSON parse validates definition',()=>assert.equal(parseGraph(JSON.stringify(graph([work('a')]))).nodes.length,1));
test('oversized JSON rejected',()=>assert.throws(()=>parseGraph(' '.repeat(2*1024*1024+1))));
test('prototype names do not read inherited checkpoint state',()=>assert.equal(toFlow(graph([work('constructor')])).nodes[0].data.status,'pending'));
test('unknown node kinds rejected',()=>assert.throws(()=>validateGraph(graph([{id:'a',kind:{type:'computer-root'}}]))));

test('start uses the current validated draft, not a cached graph',()=>{
 const draft=JSON.stringify(graph([work('updated')]));
 assert.equal(prepareGraphStart(draft,draft).nodes[0].id,'updated');
});
test('changed draft cannot start without validation',()=>{
 const old=JSON.stringify(graph([work('old')])),draft=JSON.stringify(graph([work('new')]));
 assert.throws(()=>prepareGraphStart(draft,old),/validate/);
});
test('start revalidates malformed JSON even when strings match',()=>assert.throws(()=>prepareGraphStart('{','{')));
test('an active checkpoint prevents starting another draft',()=>{
 const g=graph([work('a')]),draft=JSON.stringify(g);
 assert.throws(()=>prepareGraphStart(draft,draft,checkpoint(g)),/immutable/);
});
test('invalid checkpoint cannot be human-approved',()=>{
 const g=graph([{id:'a',kind:{type:'human',question:'Approve?'}}]),c=checkpoint(g);
 c.nodes.a.status='waiting';delete c.budget;
 assert.equal(canResolve(c,'a',c.revision,g),false);
});
test('preview remains valid without any checkpoint',()=>assert.equal(toFlow(graph([work('a')]),null).nodes[0].data.status,'pending'));
test('finite JSON branch values accept null and nested data',()=>{
 const g=graph([work('a'),{id:'b',kind:{type:'branch',source:'a',pointer:'',equals:{ok:[null,true,1]}},depends_on:[{node:'a'}]}]);
 assert.equal(validateGraph(g).layers.length,2);
});
test('NaN cannot silently change a branch value into null',()=>{
 const g=graph([work('a'),{id:'b',kind:{type:'branch',source:'a',pointer:'',equals:NaN},depends_on:[{node:'a'}]}]);
 assert.throws(()=>validateGraph(g));
});
