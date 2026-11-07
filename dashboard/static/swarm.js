// A projection of fields and explicitly logged trace checks, never a trading decision.
const nodeForCheck = { regime:'market_reader', long_ok:'market_reader', short_ok:'market_reader', max_pos:'market_reader', chop:'market_reader', trend:'screener', session:'s1', srv:'s1', atr_pctile:'s1', fvg:'s1', direct:'s1', pivot:'s1', ofcc:'ofcc', maxpos:'risk', daily_sl:'risk', daily_loss:'risk', cooldown:'risk', size:'risk', outcome:'audit' };
const titles = { regime:'Regime check', long_ok:'long_ok', short_ok:'short_ok', max_pos:'Max positions', chop:'CHOP filter', trend:'Trend check', session:'Session', srv:'SRV', atr_pctile:'ATR percentile', fvg:'FVG', direct:'Direct limit', pivot:'Pivot / SL', ofcc:'S1 closed-bar', maxpos:'Max positions', daily_sl:'Daily SL', daily_loss:'Daily PnL', cooldown:'Cooldown', size:'Sizing', outcome:'Outcome' };
const bad = s => /^(block|fail|stop)$/i.test(String(s || ''));
export function deriveSwarm(graph, checks, alive) {
  const nodes = graph?.nodes || [], edges = graph?.edges || [], summary = graph?.summary || {};
  const ids = new Set(nodes.map(n=>n.id));
  const regimeBlocked = /CHOP|NO_TRADE|OFF/i.test(String(summary.regime || ''));
  const explicitStops = nodes.filter(n=>bad(n.status)).map(n=>n.id);
  if(regimeBlocked && ids.has('market_reader')) explicitStops.push('market_reader');
  const downstream = new Set(), queue = [...explicitStops];
  while(queue.length){const current=queue.shift(); for(const edge of edges){const [a,b]=Array.isArray(edge)?edge:[edge.source,edge.target]; if(a===current&&!downstream.has(b)){downstream.add(b);queue.push(b);}}}
  const items=[], occurrences=new Map();
  const add=(parent,label,value,status,source,kind,key)=>{if(ids.has(parent)&&label){const prefix=`sw-${parent}-${kind}-${encodeURIComponent(key)}`, occurrence=occurrences.get(prefix)||0;occurrences.set(prefix,occurrence+1);items.push({id:`${prefix}-${occurrence}`,parent,label:String(label),value:String(value??''),status,source,kind,visualKind:/^(blocked|skipped|fail)$/.test(status)?'blocked':'category'});}};
  for(const n of nodes){
    const value=String(n.value || n.detail || '');
    const pieces=value.split(/\s*·\s*/).filter(Boolean);
    for(const [index,part] of pieces.entries()){
      let state='context';
      if(bad(n.status)||explicitStops.includes(n.id)) state='blocked';
      else if(n.status==='off') state='off';
      else if(downstream.has(n.id)) state='skipped';
      add(n.id,part,'',state,'Node field · status inferred from parent','field',index);
    }
  }
  for(const check of checks || []){
    const parent=nodeForCheck[check.key]; if(!parent) continue;
    let state=/^(pass|ok)$/i.test(check.status||'')?'pass':bad(check.status)?'fail':'unknown';
    if(explicitStops.includes(parent)) state='blocked';
    else if(downstream.has(parent)) state='skipped';
    else if(nodes.find(n=>n.id===parent)?.status==='off') state='off';
    add(parent,titles[check.key] || check.label,check.value,state,check.source || 'decision trace','check',check.key);
  }
  const counts={pass:items.filter(i=>i.kind==='check'&&i.status==='pass').length, blocked:items.filter(i=>i.kind==='check'&&['blocked','skipped','fail'].includes(i.status)).length, unknown:items.filter(i=>i.kind==='check'&&i.status==='unknown').length};
  const stop=regimeBlocked?'regime '+summary.regime:(summary.stop_reason || nodes.find(n=>bad(n.status))?.reason || (explicitStops.length?'node blocked':''));
  const verified=summary.verified===true || summary.cycle_complete===true;
  const output=stop?'blocked':verified?'verified':summary.cycle_n!=null?'observed':'unknown';
  return {items,counts,stop,output,alive:!!alive,active:nodes.filter(n=>/^(ok|pass|live)$/i.test(n.status||'')).length};
}
