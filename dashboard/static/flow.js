/* Control Room: a visual projection of the engine's nodes, edges and events. */
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { ReactFlow, Handle, Position, Controls, Panel, ReactFlowProvider, useReactFlow } from "@xyflow/react";
import { motion, useReducedMotion } from "framer-motion";
import { deriveSwarm } from "./swarm.js?v=5";
import { MeshCanvas } from "./mesh-canvas.js?v=1";

const e = React.createElement;
const colors = { feed: "#00f5ff", market_reader: "#00f5ff", mr09: "#a855f7", screener: "#ff2a85", s1: "#a855f7", s2s5: "#ff2a85", ofcc: "#00f5ff", risk: "#f59e0b", kill: "#ff3b5c", oms: "#10b981", audit: "#10b981" };
const colorFor = id => colors[id] || "#00f5ff";
const blockColor = "#ff3b5c";
const orderedIds = ["feed","market_reader","mr09","screener","s1","s2s5","ofcc","risk","kill","oms","audit"];
const statusClass = st => /^(ok|pass|live)$/i.test(st || "") ? "ok" : /^(block|fail|stop|chop_no_trade)$/i.test(st || "") ? "block" : /warn/i.test(st || "") ? "warn" : st === "off" ? "off" : "idle";
const roundNums = s => String(s ?? "").replace(/-?\d+\.\d{3,}/g, m => Number(m).toFixed(2));

function ApexNode({ data, selected }) {
  const st = statusClass(data.status);
  const [compact, setCompact] = useState(!!data.compact);
  const reduce = useReducedMotion();
  return e(motion.div, { layout: !reduce, transition: { duration: reduce ? 0 : .25 }, style: { "--accent": data.color }, className: `af-node ${st}${selected ? " sel" : ""}${compact ? " compact" : ""}${data.pulse ? " pulse" : ""}` },
    e(Handle, { type: "target", position: Position.Left, className: "af-handle" }),
    e("div", { className: "af-top" }, e("span", { className: `af-dot ${st}`, "aria-hidden": true }), e("span", { className: "af-title" }, data.label),
      e("button", { type: "button", className: "af-toggle nodrag nopan", "aria-label": `${compact ? "Expand" : "Compact"} ${data.label}`, "aria-pressed": compact, onClick: ev => { ev.stopPropagation(); setCompact(v => !v); } }, e("span"))),
    e("div", { className: "af-meta" }, e("span", { className: "af-badge" }, data.ref ? data.ref.split("::")[0] : "module"), e("span", { className: `af-status ${st}` }, data.status || "idle")),
    !compact && data.value ? e("div", { className: "af-value" }, roundNums(data.value)) : null,
    !compact && data.ref ? e("div", { className: "af-ref" }, data.ref) : null,
    !compact && data.tsWib ? e("div", { className: "af-ts" }, data.tsWib) : null,
    data.reason ? e("div", { className: "af-reason" }, roundNums(data.reason)) : null,
    e(Handle, { type: "source", position: Position.Right, className: "af-handle" }));
}
function SwarmNode({ data }) {
  return e("div", { className:`swarm-pill ${data.status}`, style:{"--accent":data.color}, title:`${data.source} · ${data.status}${data.value ? ' · '+data.value : ''}` },
    e(Handle,{type:'target',position:Position.Left,className:'swarm-handle'}),
    e('div',{className:'swarm-label'},data.label),
    e('div',{className:'swarm-caption'},`${data.status}${/^(blocked|skipped)$/.test(data.status)?'*':''}${data.value ? ' · '+data.value : ''}`),
    e(Handle,{type:'source',position:Position.Right,className:'swarm-handle'}));
}
function OutputNode({ data }) {
  const label={blocked:'BLOCKED',verified:'VERIFIED',observed:'OBSERVED',unknown:'UNKNOWN'}[data.output];
  return e('div',{className:`swarm-output ${data.output}`},
    e(Handle,{type:'target',position:Position.Left,className:'swarm-handle output-target',style:{left:'50%',top:'98px'}}),
    e('div',{className:'output-kicker'},'FAN-IN / CYCLE VIEW'),
    e('div',{className:'output-ring'},e('div',null,e('span',null,data.output==='verified'?'✓':data.output==='blocked'?'×':'·'),e('b',null,label))),
    e('div',{className:'output-title'},'Verified Output / Cycle Complete'),
    e('div',{className:'output-note'},data.output==='verified'?'Explicit completion flag in payload':data.output==='blocked'?'Blocked by reported gate':'Completion is not confirmed by payload'),
    e('div',{className:'output-bento'},
      e('div',null,e('b',null,data.active),e('span',null,'modules active')),
      e('div',null,e('b',null,data.counts.pass),e('span',null,'checks passed')),
      e('div',null,e('b',null,data.counts.blocked),e('span',null,'blocked / skipped*')),
      e('div',null,e('b',null,data.counts.unknown),e('span',null,'checks unknown'))),
    e('div',{className:'output-stop'},data.stop ? `stop: ${data.stop}` : 'No reported stop'),
    e('div',{className:'output-note'},`${data.alive?'Engine reports running':'Engine reports offline'} · *inferred from parent gate`));
}
const nodeTypes = { apex: React.memo(ApexNode), swarm: React.memo(SwarmNode), cycleOutput: React.memo(OutputNode) };
const noEdges = [];

function FitGraph({ topology }) {
  const { setViewport } = useReactFlow();
  useEffect(() => {
    const timer = setTimeout(() => setViewport({ x: 24, y: 60, zoom: .7 }, { duration: 0 }), 100);
    return () => clearTimeout(timer);
  }, [topology, setViewport]);
  return null;
}
function GraphNav({ swarmX, outputX, outputY }) {
  const { setViewport, setCenter } = useReactFlow();
  return e(Panel,{position:'top-right',className:'cr-graph-nav'},
    e('button',{type:'button',onClick:()=>setViewport({x:24,y:60,zoom:.7},{duration:250})},'Pipeline'),
    e('button',{type:'button',onClick:()=>setCenter(swarmX+380,300,{zoom:.65,duration:250})},'Swarm'),
    e('button',{type:'button',onClick:()=>setCenter(outputX+120,outputY+200,{zoom:.8,duration:250})},'Output'));
}
function FlowInner({ graph, checks, alive, onSelect, pulse }) {
  const nodeCache=useRef(new Map());
  const topology = JSON.stringify([(graph?.nodes || []).map(n => n.id), graph?.edges || []]);
  const { nodes, edges, sceneKey, swarmX, outputX, outputY } = useMemo(() => {
    const raw = [...(graph?.nodes || [])].sort((a,b)=>{
      const ia=orderedIds.indexOf(a.id), ib=orderedIds.indexOf(b.id);
      return (ia<0?orderedIds.length:ia)-(ib<0?orderedIds.length:ib);
    });
    const byId = new Map(raw.map(n => [n.id, n]));
    const blocked = n => n && (statusClass(n.status) === "block" || (n.id==='market_reader' && /CHOP|NO_TRADE|OFF/i.test(String(graph?.summary?.regime||''))) || /CHOP_NO_TRADE/i.test(String(n.value || n.detail || "")));
    const edges = (graph?.edges || []).map((edge, i) => {
      const [source, target] = Array.isArray(edge) ? edge : [edge.source, edge.target];
      return { id: `e-${source}-${target}-${i}`, source, target, type: "mesh", data: { color: blocked(byId.get(source)) || blocked(byId.get(target)) ? blockColor : colorFor(source) } };
    });
    const rowHeight = 152;
    const nodes = raw.map((n,i) => ({ id:n.id, type:"apex", position:{x:30,y:35+i*rowHeight}, data:{...n,color:blocked(n)?blockColor:colorFor(n.id),status:blocked(n)?"block":n.status,pulse:pulse?.node===n.id} }));
    const swarm=deriveSwarm(graph,checks,alive);
    const maxX=Math.max(250,...nodes.map(n=>n.position.x+220));
    const x0=maxX+220, cols=4, gapX=200, gapY=138;
    const items=[...swarm.items].sort((a,b)=>raw.findIndex(n=>n.id===a.parent)-raw.findIndex(n=>n.id===b.parent));
    items.forEach((item,i)=>{
      const color=item.visualKind==='blocked'?blockColor:colorFor(item.parent), x=x0+(i%cols)*gapX, y=50+Math.floor(i/cols)*gapY;
      nodes.push({id:item.id,type:'swarm',position:{x,y},data:{...item,color},selectable:false});
      edges.push({id:`field-${item.id}`,source:item.parent,target:item.id,type:'mesh',selectable:false,data:{color,inferred:true}});
      edges.push({id:`fan-${item.id}`,source:item.id,target:'cycle-output',type:'mesh',selectable:false,data:{color,inferred:true}});
    });
    const outputX=x0+cols*gapX+150, outputY=Math.max(30,Math.floor(swarm.items.length/cols)*gapY/2-140);
    nodes.push({id:'cycle-output',type:'cycleOutput',position:{x:outputX,y:outputY},selectable:false,data:swarm});
    const nextCache=new Map();
    const stableNodes=nodes.map(node=>{
      const key=JSON.stringify([node.type,node.position,node.data,node.selectable]);
      const old=nodeCache.current.get(node.id);
      const value=old && old.key===key ? old.value : node;
      nextCache.set(node.id,{key,value});
      return value;
    });
    nodeCache.current=nextCache;
    const sceneKey=JSON.stringify([stableNodes.map(n=>[n.id,n.position.x,n.position.y]),edges.map(ed=>[ed.id,ed.source,ed.target,ed.data.color])]);
    return { nodes:stableNodes, edges, sceneKey, swarmX:x0, outputX, outputY };
  }, [graph, checks, alive, pulse]);
  const click = useCallback((_, n) => { if(n.type==='apex') onSelect?.(n.id, n.data); }, [onSelect]);
  return e(ReactFlow, { nodes, edges:noEdges, nodeTypes, onNodeClick: click, nodesDraggable: false, nodesConnectable: false, edgesFocusable: false, deleteKeyCode: null, elementsSelectable: true, panOnScroll: true, minZoom: .3, maxZoom: 1.5, proOptions: { hideAttribution: true } }, e(MeshCanvas,{edges,sceneKey}), e(Controls, { showInteractive: false }), e(GraphNav,{swarmX,outputX,outputY}), e(FitGraph, { topology }));
}
function mount(el, opts = {}) {
  if (!el) return null;
  const root = el._apexRoot || createRoot(el);
  el._apexRoot = root;
  const api = { graph: opts.graph || { nodes: [], edges: [] }, checks: opts.checks || [], alive: !!opts.alive, pulse: opts.pulse || null, onSelect: opts.onSelect || null };
  const render = () => root.render(e(ReactFlowProvider, null, e(FlowInner, api)));
  render();
  return { update(next) { Object.assign(api, next); render(); }, unmount() { root.unmount(); el._apexRoot = null; } };
}
window.ApexFlow = { mount };
export { mount };
