/* Apex Paper Dashboard — multi-page dark terminal (read-only + notes) */
(() => {
"use strict";
const $ = (s, el=document) => el.querySelector(s);
const $$ = (s, el=document) => [...el.querySelectorAll(s)];
const N = v => v==null||v==="" ? NaN : Number(v);
const f2 = v => isFinite(v) ? v.toFixed(2) : "—";
const PRICE_DECIMALS = {
  BTCUSDT: 2, ETHUSDT: 2, WLDUSDT: 7, XRPUSDT: 4,
};
const PRICE_TICK = {
  BTCUSDT: 0.10, ETHUSDT: 0.01, WLDUSDT: 0.0001000, XRPUSDT: 0.0001,
};
const priceMeta = new Map();
const fP = (v, sym, meta) => {
  if (!isFinite(v)) return "—";
  const key = String(sym||"").toUpperCase();
  const m = meta || priceMeta.get(key) || {};
  const d = Number.isFinite(Number(m.precision)) ? Number(m.precision) : (PRICE_DECIMALS[key] ?? (key.includes("BTC") ? 2 : 4));
  return Number(v).toLocaleString("en-US",{minimumFractionDigits:d,maximumFractionDigits:d});
};
const fPExact = (v, sym, meta) => fP(v, sym, meta);
const WIB = "Asia/Jakarta";
const wib = (ms, opt={}) => {
  if (!isFinite(ms)) return "—";
  return new Date(ms).toLocaleString("id-ID",{timeZone:WIB,day:"2-digit",month:"short",hour:"2-digit",minute:"2-digit",second:opt.sec?"2-digit":undefined,...opt});
};
const wibTime = tSec => wib(tSec*1000, {sec:false});
const tfMs = {"1m":60e3,"5m":300e3,"15m":900e3,"1h":3600e3,"4h":14400e3,"1d":86400e3};
const TF_LIST = ["1m","5m","15m","1h","4h","1d"];
const SYM_LIST = ["ETHUSDT","BTCUSDT"];
const titles = {
  control:"CONTROL ROOM", terminal:"TERMINAL", research:"RESEARCH",
  portfolio:"PORTOFOLIO", heatmap:"HEATMAP", market:"MARKET",
  learning:"LEARNING & EVALUASI", library:"PERPUSTAKAAN"
};


function fmt2(x, nd=2){
  if(x==null||x==="") return "—";
  const n=Number(x);
  if(!isFinite(n)) return String(x);
  return n.toFixed(nd);
}
function fmtPct(x, nd=2){
  if(x==null||x==="") return "—";
  const n=Number(x);
  if(!isFinite(n)) return String(x);
  return (n>=0?"+":"")+n.toFixed(nd)+"%";
}

let S=null, RESEARCH=null, LEARN=null, LIB=null, MARKET=null, HEAT=null, ANATOMY=null;
let page="control";
let lastWsMsg=0;
let pollBan=0;

/* ---------- nav ---------- */
$$(".nav-item").forEach(el => el.onclick = () => go(el.dataset.page));
function go(p){
  try{ if(p!=="heatmap" && window.microHeatmapStop) window.microHeatmapStop(); }catch(e){}
  page=p;
  $$(".nav-item").forEach(x=>x.classList.toggle("on",x.dataset.page===p));
  $$(".page").forEach(x=>x.classList.toggle("on",x.id==="page-"+p));
  $("#pageTitle").textContent = titles[p]||p;
  if(p==="terminal") termEnsure();
  if(p==="control") crRender();
  if(p==="research") researchRender();
  if(p==="portfolio") portfolioRender();
  if(p==="heatmap") heatmapRender();
  if(p==="market") marketRender();
  if(p==="learning") learningRender();
  if(p==="library") libraryRender();
  const pg=$("#page-"+p);
  if(pg){ pg.style.opacity=""; pg.style.transform=""; }
  if(window.lucide) try{ lucide.createIcons(); }catch(e){}
}

/* ---------- fetch helpers ---------- */
async function jget(url){
  const r = await fetch(url,{cache:"no-store"});
  if(!r.ok) throw new Error(url+" "+r.status);
  return r.json();
}

/* ---------- clock ---------- */
setInterval(()=>{
  $("#clock").textContent = new Date().toLocaleTimeString("id-ID",{timeZone:WIB})+" WIB";
},1000);

/* ---------- paper progress + alerts (read-only) ---------- */
async function pollPaper(){
  const el=$("#navPaper"); if(!el) return;
  try{
    const A = await jget("/api/alerts");
    const p = A.progress||{}, c=p.criteria||{};
    const f = x=> (x==null?"—":(+x).toFixed(2));
    const fl = A.flags||{};
    const warn = fl.down||fl.stale;
    const last = (A.alerts||[]).slice(-5).reverse();
    const yh = p.yahudi_scalp||{};
    const mg = yh.macro_gate||{};
    const yhTag = yh.mode ? ` · SCALP S01/S02 (1.7R${mg.macro_bias ? " · "+mg.macro_bias : ""})` : "";
    el.textContent = `PAPER ${p.closed||0}/${p.target_trades||60} trade · E ${f(p.e_r)}R · PF ${f(p.pf)}${yhTag}`
      + (warn ? (fl.down ? " · ⚠ ENGINE DOWN" : " · ⚠ DATA STALE") : "");
    el.style.color = warn ? "var(--warn,#f0b429)" : "";
    const trLines = Object.values(yh.symbol_traces||{}).map(x=>{
      const t = x.trace||{};
      return `${x.symbol}: ${t.blocker||"-"} | ZLEMA=${t.zl_trend||"-"} | Pull=${t.s02_pull_direction||"-"}(${f(t.s02_pull_bias)}) | Zones=${t.usable_zones_count||0}`;
    });
    el.title = (c.verdict?("Status: "+c.verdict+"\n"):"")
      + (trLines.length ? ("=== YAHUDI SCALP PRO (S01_SWEEP & S02_SNIPER) ===\n"+trLines.join("\n")+"\n\n") : "")
      + (last.length
      ? last.map(a=>new Date(a.ts).toLocaleTimeString("id-ID",{timeZone:WIB})+" ["+a.kind+"] "+a.text).join("\n")
      : "Belum ada alert");
  }catch(e){ el.textContent="paper — (alert API tidak terjangkau)"; }
}
setInterval(pollPaper, 15000); setTimeout(pollPaper, 1500);

/* ---------- state poll ---------- */
async function pollState(){
  try{
    S = await jget("/api/state");
    const c=S.cycles&&S.cycles.at(-1);
    const navLive=$("#navLive"), navEng=$("#navEng"), navSub=$("#navSub"), navArm=$("#navArm");
    if(navLive) navLive.className="live "+(S.alive?"on":"off");
    if(navEng) navEng.textContent = S.alive?"PAPER · LIVE":"ENGINE OFF";
    if(navSub) navSub.textContent = S.alive
      ? "apex.exe berjalan (Paper)"
      : "apex.exe tidak berjalan (dashboard tetap jalan)";
    if(navArm){
      const isArmed = S.arm !== false;
      navArm.textContent = isArmed ? "ARM TRUE" : "ARM FALSE";
      navArm.className = "chip " + (isArmed ? "safe" : "warn");
    }
    if(c){
      const trade=!/CHOP|NO_TRADE|OFF/i.test(c.regime);
      $("#regimeChip").textContent="REGIME "+c.regime;
      $("#regimeChip").className="chip "+(trade?"safe":"warn");
    }
    if(page==="control") crRender();
    if(page==="terminal") termOnState();
    if(page==="portfolio") portfolioRender();
  }catch(e){
    if($("#navEng")) $("#navEng").textContent="DASH OFFLINE";
  }
}
async function pollLive(){
  try{
    const L = await jget("/api/live?symbol=ETHUSDT&interval=15m");
    if(L.tickers){
      const b=L.tickers.BTCUSDT,e=L.tickers.ETHUSDT;
      if(b&&b.last!=null){ $("#pBTC").textContent=`BTC ${fP(b.last,"BTCUSDT")}`; $("#pBTC").className="mono "+((b.pct24h||0)>=0?"up":"dn"); }
      if(e&&e.last!=null){ $("#pETH").textContent=`ETH ${fP(e.last,"ETHUSDT")}`; $("#pETH").className="mono "+((e.pct24h||0)>=0?"up":"dn"); }
    }
    if(page==="terminal") termOnLive(L);
  }catch(e){}
}
setInterval(pollState, 3000);
setInterval(pollLive, 2500);

/* ========================================================================
   CONTROL ROOM
======================================================================== */

/* ---------- Control Room (React Flow) ---------- */
let CR = { flow: null, selected: null, events: [], replayIdx: -1, pulse: null, pulseTimer: null };

function wibShort(ts){
  if(ts==null||ts==="") return "—";
  let ms = ts;
  if(typeof ts==="string"){
    const p = Date.parse(ts.endsWith("Z")||ts.includes("+")?ts:ts+"Z");
    ms = isFinite(p)?p:NaN;
  } else {
    ms = Number(ts);
    if(ms < 1e12) ms *= 1000;
  }
  if(!isFinite(ms)) return String(ts).slice(0,19);
  return new Date(ms).toLocaleString("id-ID",{timeZone:WIB,hour:"2-digit",minute:"2-digit",second:"2-digit"});
}

function crSummaryLine(eng){
  const s = (eng && eng.summary) || {};
  const cyc = s.cycle_n != null ? s.cycle_n : ((S&&S.cycles)||[]).length;
  const reg = s.regime || "—";
  const ts = s.last_cycle_ts ? wibShort(s.last_cycle_ts) : new Date().toLocaleTimeString("id-ID",{timeZone:WIB,hour:"2-digit",minute:"2-digit"});
  const longOk = s.long_ok;
  const shortOk = s.short_ok;
  let sig = "tidak ada sinyal";
  if((S&&S.entries&&S.entries.length)) sig = "ENTRY terakhir ada";
  else if((S&&S.skips&&S.skips.length)) sig = "SKIP: "+(S.skips.at(-1).reason||"?");
  else if(shortOk===false && longOk===true) sig = "S1 long_ok · short_ok false";
  else if(longOk===false) sig = "long_ok false";
  const regCls = /CHOP|NO_TRADE|OFF/i.test(reg)?"bad":"ok";
  let html = `<span class="hl">${ts} WIB</span> <span class="dot">·</span> `+
    `siklus <b>#${cyc}</b> <span class="dot">·</span> `+
    `<span class="${regCls}">${reg}</span> <span class="dot">·</span> `+
    `<span>${sig}</span>`;
  if(s.stop_reason) html += ` <span class="dot">·</span> <span class="bad">stop: ${s.stop_reason}</span>`;
  return html;
}

function crMount(){
  const el = $("#page-control");
  el.innerHTML = `<div id="cr-wrap">
    <div id="cr-main">
      <div id="cr-summary">${crSummaryLine((S&&S.engine)||{})}</div>
      <div class="cr-stage"><div class="cr-stage-head"><div><div class="cr-kicker">APEX / EXECUTION MESH</div><h2>Control Room</h2><p>Animated mesh is illustrative · statuses reflect reported data</p></div><div id="cr-hud" class="cr-hud"></div></div>
      <div id="cr-flow"></div></div>
      <div id="cr-replay">
        <span class="micro">REPLAY</span>
        <button class="btn sm" id="cr-prev" title="Event sebelumnya">◀ Prev</button>
        <button class="btn sm" id="cr-play" title="Putar event terakhir">Play last 20</button>
        <button class="btn sm" id="cr-next" title="Event berikutnya">Next ▶</button>
        <span class="mu mono" id="cr-restatus">idle</span>
      </div>
      <div class="cr-log-panel"><div class="cr-log-head"><span>● LIVE STREAM</span><span>ENGINE LOG · LATEST</span></div><div id="cr-log" aria-label="Recent engine log"></div></div>
    </div>
    <div id="cr-side" class="card scroll">
      <h3>ENGINE PIPE</h3>
      <div id="cr-pipe" class="mu" style="font-size:12.5px;margin-bottom:10px"></div>
      <div class="acc"><h4>NODE INSPECT</h4><div id="cr-detail" class="mu">Klik node untuk detail + jejak keputusan</div></div>
      <div class="acc" style="margin-top:10px"><div class="micro">KILL / ARM</div>
        <div class="row"><span>Mode</span><b class="chip safe">PAPER</b></div>
        <div class="row"><span>Engine</span><b id="cr-eng-st" class="chip safe">LIVE</b></div>
        <div class="row"><span>Arm (Paper)</span><b id="cr-arm" class="chip safe">True</b></div>
        <div class="row"><span>Kill</span><b id="cr-kill" class="chip safe">OFF</b></div>
      </div>
    </div>
  </div>`;
  $("#cr-prev").onclick = () => crReplayStep(-1);
  $("#cr-next").onclick = () => crReplayStep(1);
  $("#cr-play").onclick = () => crReplayPlay();
}

function crEnsureFlow(){
  const host = $("#cr-flow");
  if(!host) return;
  if(!window.ApexFlow){
    if(!host.children.length) host.innerHTML = `<div class="empty">Memuat React Flow…</div>`;
    return;
  }
  if(!CR.flow){
    CR.flow = window.ApexFlow.mount(host, {
      graph: (S && S.engine) || {nodes:[],edges:[]},
      checks: (S && S.chain && S.chain.steps) || [], alive: !!(S && S.alive),
      pulse: CR.pulse,
      onSelect: (id, data) => crSelectNode(id, data),
    });
  } else {
    CR.flow.update({
      graph: (S && S.engine) || {nodes:[],edges:[]},
      checks: (S && S.chain && S.chain.steps) || [], alive: !!(S && S.alive),
      pulse: CR.pulse,
      onSelect: (id, data) => crSelectNode(id, data),
    });
  }
}

function crSelectNode(id, data){
  CR.selected = id;
  const n = ((S&&S.engine&&S.engine.nodes)||[]).find(x=>x.id===id) || {label:id, ...(data||{})};
  const chain = (S && S.chain && S.chain.steps) || [];
  const labelLow = String(n.label||"").toLowerCase().slice(0,3);
  const related = chain.filter(st => String(st.key||"").includes(id) || String(st.label||"").toLowerCase().includes(labelLow));
  const evs = (CR.events.length?CR.events:(S&&S.engine_events)||[]).filter(ev => (ev.stop===id) || (ev.path||[]).some(p=>p.startsWith(id+">")||p.endsWith(">"+id))).slice(-8).reverse();
  const evHtml = evs.length
    ? evs.map(ev=>'<div class="row"><span>'+wibShort(ev.ts)+' · '+ev.kind+'</span><b class="mono">'+(ev.reason||ev.summary||"")+'</b></div>').join("")
    : '<div class="mu">Belum ada event di node ini</div>';
  const relHtml = related.length
    ? ('<div class="sep"></div><div class="micro">Decision trace</div>' + related.map(st=>'<div class="row"><span>'+st.label+'</span><b class="'+st.status+'">'+st.value+'</b></div>').join(""))
    : "";
  $("#cr-detail").innerHTML =
    '<b>'+(n.label||id)+'</b>'+
    '<div style="margin-top:6px">'+(n.value||n.detail||"")+'</div>'+
    '<div class="af-ref" style="margin-top:8px">'+(n.ref||"")+'</div>'+
    (n.reason?'<div class="af-reason">'+n.reason+'</div>':'')+
    '<div class="sep"></div><div class="micro">Recent events</div>'+evHtml+relHtml;
}

function crPulse(ev){
  if(!ev) return;
  CR.pulse = { node: ev.stop, path: ev.path||[], kind: ev.kind, reason: ev.reason };
  crEnsureFlow();
  const st = $("#cr-restatus");
  if(st) st.textContent = `${ev.kind||"?"} → ${ev.stop||"?"} ${ev.reason||ev.summary||""}`;
  clearTimeout(CR.pulseTimer);
  CR.pulseTimer = setTimeout(()=>{ CR.pulse=null; crEnsureFlow(); }, 2200);
}

function crReplayStep(dir){
  const evs = CR.events.length?CR.events:((S&&S.engine_events)||[]);
  if(!evs.length){ $("#cr-restatus").textContent="no events"; return; }
  if(CR.replayIdx<0) CR.replayIdx = evs.length-1;
  else CR.replayIdx = Math.max(0, Math.min(evs.length-1, CR.replayIdx+dir));
  crPulse(evs[CR.replayIdx]);
}

async function crReplayPlay(){
  let evs = CR.events.length?CR.events:((S&&S.engine_events)||[]);
  if(!evs.length){
    try{ const r=await jget("/api/engine-events"); evs=r.events||[]; CR.events=evs; }catch(e){}
  }
  const slice = evs.slice(-20);
  for(let i=0;i<slice.length;i++){
    CR.replayIdx = evs.length - slice.length + i;
    crPulse(slice[i]);
    await new Promise(r=>setTimeout(r,650));
  }
}

function crOnEngineEvent(ev){
  if(!ev||!ev.kind) return;
  CR.events.push(ev);
  if(CR.events.length>200) CR.events=CR.events.slice(-200);
  if(page==="control") crPulse(ev);
}

function crRender(){
  if(!$("#cr-flow")) crMount();
  if(!S) return;
  const eng = S.engine || {nodes:[],edges:[],pipeline:[],summary:{}};
  if(S.engine_events && !CR.events.length) CR.events = S.engine_events.slice();
  const sumHtml = crSummaryLine(eng);
  if($("#cr-summary") && $("#cr-summary").innerHTML !== sumHtml) $("#cr-summary").innerHTML = sumHtml;
  const nodes = eng.nodes || [], checks = (S.chain && S.chain.steps) || [];
  const metrics = [
    ["Active modules", nodes.filter(n=>/^(ok|pass|live)$/i.test(n.status||"")).length],
    ["Cycles observed", eng.summary?.cycle_n],
    ["Passed · raw trace", checks.length ? checks.filter(c=>/^(ok|pass)$/i.test(c.status||"")).length : null],
    ["Failed · raw trace", checks.length ? checks.filter(c=>/^(block|fail|stop)$/i.test(c.status||"")).length : null],
  ];
  const hud = $("#cr-hud");
  if(hud && !hud.children.length) metrics.forEach(([name])=>{const item=document.createElement("div"); item.className="cr-metric"; const label=document.createElement("span"); label.textContent=name; const value=document.createElement("strong"); value.textContent="—"; item.append(label,value); hud.append(item);});
  if(hud) metrics.forEach(([,num], i)=>{
    const value=hud.children[i].lastChild;
    if(!Number.isFinite(Number(num)) || num == null){value.textContent="—";return;}
    const target=Number(num), old=Number(value.dataset.value);
    if(value.dataset.value===String(target)) return;
    value.dataset.value=String(target);
    if(!Number.isFinite(old)||matchMedia('(prefers-reduced-motion: reduce)').matches){value.textContent=target;return;}
    const start=performance.now();
    const tick=now=>{if(value.dataset.value!==String(target)) return; const t=Math.min(1,(now-start)/350);value.textContent=Math.round(old+(target-old)*t);if(t<1)requestAnimationFrame(tick);};requestAnimationFrame(tick);
  });
  const pipeHtml = (eng.pipeline||[]).map(x=>`<div>• ${x}</div>`).join("");
  if($("#cr-pipe") && $("#cr-pipe").innerHTML !== pipeHtml) $("#cr-pipe").innerHTML = pipeHtml;
  $("#cr-kill").textContent = S.kill?"ON":"OFF";
  $("#cr-kill").className = "chip "+(S.kill?"bad":"safe");
  if($("#cr-eng-st")){
    $("#cr-eng-st").textContent = S.alive ? "PAPER LIVE" : "ENGINE OFF (Dash ON)";
    $("#cr-eng-st").className = "chip " + (S.alive ? "safe" : "warn");
  }
  if($("#cr-arm")){
    const armed = S.arm !== false;
    $("#cr-arm").textContent = armed ? "True" : "False";
    $("#cr-arm").className = "chip " + (armed ? "safe" : "warn");
  }
  crEnsureFlow();
  const log=$("#cr-log");
  const lines=[...(S.log||[])].slice(-4).reverse();
  if(log && log.dataset.h!==lines.join("|")){
    log.dataset.h=lines.join("|");
    log.replaceChildren(...lines.map(l=>{const line=document.createElement("div");line.className="line "+(/risk|kill|reject|skip/i.test(l)?"cr-log-alert":/reader|regime|cycle/i.test(l)?"cr-log-cycle":/oms|entry|order/i.test(l)?"cr-log-oms":"");line.textContent=l;return line;}));
  }
}
// wait for ApexFlow module then first paint
function crBootWhenReady(){
  if(window.ApexFlow){ if(page==="control") crRender(); return; }
  setTimeout(crBootWhenReady, 120);
}
crBootWhenReady();

/* ========================================================================
   TERMINAL — multi-chart + setups + signals + anatomy
======================================================================== */
const term = {
  layout: Number(localStorage.getItem("apex_layout")||1),
  cells: JSON.parse(localStorage.getItem("apex_cells")||"null") || [{sym:"ETHUSDT",tf:"15m"}],
  showSkip: false,
  overlays: {pivots:false, session:false, chop:false, setup:false, signals:false, liq:false},
  charts: [], // {el, chart, series, vol, overlay, lastBar, closes, wsKey}
  selectedSig: null,
  ws: null,
  kcache: {},
};
function saveTerm(){
  localStorage.setItem("apex_layout", String(term.layout));
  localStorage.setItem("apex_cells", JSON.stringify(term.cells.map(c=>({sym:c.sym,tf:c.tf}))));
}
function termMount(){
  const el=$("#page-terminal");
  el.innerHTML=`<div id="term" class="no-bottom">
    <div id="term-toolbar">
      <div class="group"><span class="glb">Grid</span>
        ${[1,2,4,6,8].map(n=>`<button class="btn sm lay" data-n="${n}" title="${n} chart berdampingan">${n} chart</button>`).join("")}
      </div>
      <div class="group"><span class="glb">Overlay</span>
        <button class="btn sm ov" data-ov="pivots" title="Titik pivot dekat bar — bukan garis penuh">Pivot</button>
        <button class="btn sm ov" data-ov="session" title="Bayangan sesi Asia / New York">Sesi</button>
        <button class="btn sm ov" data-ov="chop" title="Zona CHOP (approx dari kode)">CHOP</button>
        <button class="btn sm ov" data-ov="liq" title="Liquidation heatmap dari Vike (server proxy)">Liq Heatmap (Vike)</button>
        <button class="btn sm ov" data-ov="setup" title="Kotak entry / SL / TP">Setup</button>
        <button class="btn sm ov" data-ov="signals" title="Marker sinyal ENTRY">Sinyal</button>
      </div>
      <div class="sp"></div>
      <span id="liveMode" class="chip info" title="Sumber data chart">FEED REST</span>
      <button class="btn sm" id="togBottom" title="Paksa tampilkan panel posisi">Panel ↓</button>
    </div>
    <div id="chart-grid"></div>
    <div id="trace-panel">
      <h4>STATUS · SIMBOL</h4>
      <div class="status-box" id="term-status">Memuat…</div>
      <h4>JEJAK KEPUTUSAN · S1_TFP V10</h4>
      <div id="trace-steps"></div>
      <div class="mu" style="font-size:12px;margin-top:8px" id="trace-miss"></div>
    </div>
    <div id="term-bottom">
      <div class="tabs" id="btabs" style="padding:6px 8px;border-bottom:1px solid var(--ln)">
        <span class="tab on" data-t="pos" title="Posisi terbuka">Posisi</span>
        <span class="tab" data-t="ord" title="Order pending">Pending</span>
        <span class="tab" data-t="hist" title="Fill / close">Fills</span>
        <span class="tab" data-t="sig" title="Sinyal & skip">Sinyal</span>
        <span class="tab" data-t="log" title="Cuplikan run.log">Log</span>
        <span class="tab" data-t="setup" title="Setup aktif">Setup</span>
      </div>
      <div class="tbl-wrap" id="tbl"></div>
    </div>
  </div>`;
  $$(".lay",el).forEach(b=>b.onclick=()=>{term.layout=Number(b.dataset.n); saveTerm(); termBuildGrid();});
  $$(".ov",el).forEach(b=>b.onclick=()=>{
    const k=b.dataset.ov; term.overlays[k]=!term.overlays[k];
    b.classList.toggle("on",term.overlays[k]);
    if(k==="liq"){ term.charts.forEach(c=>drawLiqHeat(c)); }
    term.charts.forEach(c=>{drawOverlays(c); drawMarkers(c); drawSetup(c);});
  });
  $$("#btabs .tab").forEach(t=>t.onclick=()=>{
    $$("#btabs .tab").forEach(x=>x.classList.toggle("on",x===t));
    term.btab=t.dataset.t; term._forceBottom=true; renderTable();
  });
  const tb=$("#togBottom",el);
  if(tb) tb.onclick=()=>{ term._forceBottom=!term._forceBottom; renderTable(); };
  term.btab="pos";
}
function layoutDims(n){
  if(n===1) return {cols:1,rows:1};
  if(n===2) return {cols:2,rows:1};
  if(n===4) return {cols:2,rows:2};
  if(n===6) return {cols:3,rows:2};
  return {cols:4,rows:2};
}
async function loadPriceMeta(){
  try{
    const d = await jget('/api/symbols');
    (d.items||[]).forEach(x => priceMeta.set(String(x.symbol||'').toUpperCase(), x));
  }catch(e){}
}
function openTrades(){
  if(!S||!S.oms) return [];
  return Object.values(S.oms.trades||{});
}
function focusOrder(cells){
  const open=openTrades();
  if(!open.length) return cells;
  const syms=[...new Set(open.map(t=>t.symbol))];
  const rest=cells.filter(c=>!syms.includes(c.sym));
  const prioritized=syms.map(s=>({sym:s,tf:(cells.find(c=>c.sym===s)||{}).tf||"15m"}));
  return [...prioritized,...rest].slice(0, cells.length);
}
function termEnsure(){
  if(!$("#term")) termMount();
  if(!term.charts.length || term.charts.length !== term.layout || !$("#chart-grid").children.length){
    termBuildGrid();
  } else {
    term.charts.forEach(obj => {
      try{
        const wrap = $(".cwrap", obj.el);
        if(wrap && wrap.clientWidth > 0) obj.chart.applyOptions({width: wrap.clientWidth, height: wrap.clientHeight});
      }catch(e){}
      loadCell(obj);
    });
    connectSSE();
  }
  renderTrace();
  renderTable();
}
function termBuildGrid(){
  const n=term.layout;
  while(term.cells.length<n) term.cells.push({sym:"ETHUSDT",tf:"15m"});
  term.cells=term.cells.slice(0,n);
  term.cells=focusOrder(term.cells);
  saveTerm();
  $$(".lay").forEach(b=>b.classList.toggle("on",Number(b.dataset.n)===n));
  const {cols}=layoutDims(n);
  const grid=$("#chart-grid");
  // destroy old
  term.charts.forEach(c=>{try{c.chart.remove();}catch(e){} });
  term.charts=[];
  grid.style.gridTemplateColumns=`repeat(${cols},1fr)`;
  grid.innerHTML="";
  const openSyms=new Set(openTrades().map(t=>t.symbol));
  term.cells.forEach((cfg,i)=>{
    const cell=document.createElement("div");
    cell.className="cell"+(openSyms.has(cfg.sym)?" has-pos":"");
    cell.dataset.i=i;
    cell.innerHTML=`
      <div class="cell-hd">
        <select class="sym" title="Simbol">${SYM_LIST.map(s=>`<option ${s===cfg.sym?"selected":""}>${s}</option>`).join("")}</select>
        <button class="btn sm pick" title="Cari semua USDT-M">⋯</button>
        <select class="tf" title="Timeframe">${TF_LIST.map(t=>`<option ${t===cfg.tf?"selected":""}>${t}</option>`).join("")}</select>
        <span class="px-live" data-px>—</span>
        <span class="pct24 mono" data-pct>—</span>
        <span class="cd-badge" title="Sisa waktu ke close candle">close —</span>
        <span class="chip info" data-reg title="Regime Market Reader">—</span>
        <span class="chip" data-strat title="Status strategi untuk simbol ini">S1 · idle</span>
        ${openSyms.has(cfg.sym)?'<span class="chip safe">POSISI</span>':''}
        <span class="sig-badge badge">belum ada sinyal</span>
        <span class="sp"></span>
        <button class="btn sm ind" title="Indikator dari kode">Indikator</button>
      </div>
      <div class="cell-chart"><div class="legend mono"></div><div class="cwrap"></div><canvas class="setup"></canvas><div class="last-ax" aria-hidden="true"><div class="last-ax-price">—</div><div class="last-ax-cd">—</div></div><div class="close-vline" aria-hidden="true"></div><canvas class="liq-heat"></canvas></div>
      <div class="closes"></div>`;
    grid.appendChild(cell);
    const wrap=$(".cwrap",cell);
    const chart=LightweightCharts.createChart(wrap,{
      layout:{background:{color:"#0c1016"},textColor:"#b7c2ce",fontFamily:"Manrope",fontSize:12},
      grid:{vertLines:{color:"#1a2029"},horzLines:{color:"#1a2029"}},
      crosshair:{mode:0},
      rightPriceScale:{borderColor:"#232a35"},
      timeScale:{borderColor:"#232a35",timeVisible:true,secondsVisible:cfg.tf==="1m"},
      localization:{
        locale:"id-ID",
        timeFormatter: t => {
          const d=new Date(t*1000);
          return d.toLocaleString("id-ID",{timeZone:WIB,month:"short",day:"2-digit",hour:"2-digit",minute:"2-digit",second:cfg.tf==="1m"?"2-digit":undefined})+" WIB";
        }
      }
    });
    const series=chart.addCandlestickSeries({
      upColor:"#0ecb81",downColor:"#f6465d",borderVisible:false,
      wickUpColor:"#0ecb81",wickDownColor:"#f6465d",
      lastValueVisible:true,
      priceLineVisible:true,
      priceLineWidth:1,
      priceLineStyle:(window.LightweightCharts&&LightweightCharts.LineStyle?LightweightCharts.LineStyle.Dashed:2),
      priceLineColor:"#0ecb81"
    });
    const vol=chart.addHistogramSeries({priceFormat:{type:"volume"},priceScaleId:"v"});
    chart.priceScale("v").applyOptions({scaleMargins:{top:.8,bottom:0}});
    const ro=new ResizeObserver(()=>chart.applyOptions({width:wrap.clientWidth,height:wrap.clientHeight}));
    ro.observe(wrap);
    const obj={el:cell,chart,series,vol,cfg,lastBar:null,closes:[],lines:[],pivotMarks:[]};
    chart.timeScale().subscribeVisibleLogicalRangeChange(()=>{ updateCloseChrome(obj); if(term.overlays.liq) drawLiqHeat(obj); });

    chart.subscribeCrosshairMove(p=>{
      const k=p.seriesData&&p.seriesData.get(series);
      if(k) setLegend(obj,k);
    });
    $(".sym",cell).onchange=e=>{cfg.sym=e.target.value; saveTerm(); loadCell(obj);};
    $(".tf",cell).onchange=e=>{cfg.tf=e.target.value; chart.applyOptions({timeScale:{secondsVisible:cfg.tf==="1m"}}); saveTerm(); loadCell(obj);};
    term.charts.push(obj);
    loadCell(obj);
    requestAnimationFrame(()=>{ try{ obj.chart.resize(obj.el.querySelector(".cell-chart").clientWidth, obj.el.querySelector(".cell-chart").clientHeight); }catch(e){} });
  });
  // scroll to first has-pos
  const hp=$(".cell.has-pos");
  if(hp) hp.scrollIntoView({behavior:"smooth",block:"nearest"});
}
async function loadCell(obj){
  const {sym,tf}=obj.cfg;
  const key=sym+"|"+tf;
  let data=term.kcache[key];
  if(!data || !data.bars || !data.bars.length || Date.now()-data.t>45000){
    try{
      const fetched = await jget(`/api/klines?symbol=${sym}&interval=${tf}`);
      if(Array.isArray(fetched) && fetched.length){
        // Deduplicate & sort ascending by time for LightweightCharts
        const byTime = new Map();
        fetched.forEach(b => { if(b && b.time != null) byTime.set(Number(b.time), b); });
        const sorted = [...byTime.values()].sort((a,b) => a.time - b.time);
        data = {t: Date.now(), bars: sorted};
        term.kcache[key] = data;
      }
    }catch(e){}
  }
  const bars=(data&&data.bars)||[];
  obj._bars=bars;
  if(bars.length){
    obj.series.setData(bars);
    obj.vol.setData(bars.map(k=>({time:k.time,value:k.volume,color:k.close>=k.open?"#0ecb8155":"#f6465d55"})));
    if(obj._loadedKey !== key){
      obj._loadedKey = key;
      try{ obj.chart.timeScale().fitContent(); }catch(e){}
    }
    obj.lastBar=bars.at(-1); setLegend(obj,obj.lastBar);
    obj.closes=bars.slice(-6).map(b=>({t:b.time,c:b.close,o:b.open})); renderCloses(obj);
    const px=$("[data-px]",obj.el); if(px) px.textContent=fP(obj.lastBar.close,sym);
    applyLastPriceStyle(obj, obj.lastBar);
  }
  drawMarkers(obj); drawOverlays(obj); drawSetup(obj); updateSigBadge(obj);
  updateCloseChrome(obj);
  if(term.overlays.liq) drawLiqHeat(obj);
  try{
    const live=await jget(`/api/live?symbol=${sym}&interval=${tf}`);
    const t=(live.tickers||{})[sym];
    if(t){
      if(t.mark||t.last){ term._marks=term._marks||{}; term._marks[sym]=t.mark||t.last; }
      const px=$("[data-px]",obj.el); if(px && (t.last||t.mark)!=null) px.textContent=fP(t.last||t.mark,sym);
      const pctEl=$("[data-pct]",obj.el);
      if(pctEl && t.pct24h!=null){
        const p=Number(t.pct24h);
        pctEl.textContent=(p>=0?"+":"")+p.toFixed(2)+"%";
        pctEl.className="pct24 mono "+(p>=0?"up":"dn");
      }
    }
  }catch(e){}
  updateTermStatus();
}
function setLegend(obj,k){
  const {sym,tf}=obj.cfg;
  const openMs=k.time*1000, closeMs=openMs+(tfMs[tf]||900000)-1;
  $(".legend",obj.el).innerHTML=`<b>${sym}</b> ${tf}
    O <b>${fP(k.open,sym)}</b> H <b>${fP(k.high,sym)}</b> L <b>${fP(k.low,sym)}</b>
    C <b class="${k.close>=k.open?"up":"dn"}">${fP(k.close,sym)}</b>
    <span class="mu">· buka ${wib(openMs)} · tutup ${wib(closeMs)}</span>`;
}
function renderCloses(obj){
  const {sym}=obj.cfg;
  $(".closes",obj.el).innerHTML = obj.closes.map(b=>{
    const d=b.c-b.o; const cls=d>=0?"up":"dn";
    return `<span class="c"><b class="${cls}">${fP(b.c,sym)}</b> ${wib(b.t*1000)} <span class="${cls}">${d>=0?"+":""}${d.toFixed(2)}</span></span>`;
  }).join("");
}
function updateSigBadge(obj){
  const sigs=(S&&S.signals||[]).filter(s=>s.symbol===obj.cfg.sym && s.taken);
  const b=$(".sig-badge",obj.el);
  if(!sigs.length) b.textContent="belum ada entry";
  else b.textContent=sigs.length+" entry";
}
function drawMarkers(obj){
  if(!term.overlays.signals){ const pm=obj._pivotMarks||[]; try{obj.series.setMarkers(pm);}catch(e){}; return; }
  const {sym,tf}=obj.cfg;
  const tfs=tfMs[tf]/1000;
  const m=[];
  const sigs=(S&&S.signals||[]).filter(s=>s.symbol===sym && s.taken);
  sigs.forEach(s=>{
    if(!s.taken) return;
    let ts=s.ts;
    if(typeof ts==="string") ts=Date.parse(ts);
    if(!isFinite(ts)) return;
    // audit ts may be ms
    if(ts>1e12) ts=Math.floor(ts/1000);
    const time=Math.floor(ts/tfs)*tfs;
    const long=/long/i.test(s.side||"");
    m.push({
      time, position: long?"belowBar":"aboveBar",
      color: long?"#0ecb81":"#f6465d",
      shape: long?"arrowUp":"arrowDown",
      text: `${s.strategy||"?"} ✓`.trim()
    });
  });
  // OMS fills
  if(S&&S.oms){
    [...Object.values(S.oms.trades||{}),...(S.oms.closed||[])].filter(t=>t.symbol===sym).forEach(t=>{
      const long=/long/i.test(t.side);
      if(t.filled_ms) m.push({time:Math.floor(t.filled_ms/1000/tfs)*tfs, position:long?"belowBar":"aboveBar", color:long?"#0ecb81":"#f6465d", shape:long?"arrowUp":"arrowDown", text:`${t.strategy} FILL`});
      if(t.closed_ms) m.push({time:Math.floor(t.closed_ms/1000/tfs)*tfs, position:"aboveBar", color:"#f0b90b", shape:"circle", text:t.exit_kind||"exit"});
    });
  }
  m.sort((a,b)=>a.time-b.time);
  obj.series.setMarkers(m);
}
function pivotsApprox(bars, left=2, right=2){
  // fractal pivots approximation (engine uses confirmed pivots; labeled approx)
  const ph=[],pl=[];
  for(let i=left;i<bars.length-right;i++){
    let hi=true,lo=true;
    for(let j=i-left;j<=i+right;j++){ if(j===i) continue; if(bars[j].high>=bars[i].high) hi=false; if(bars[j].low<=bars[i].low) lo=false; }
    if(hi) ph.push({time:bars[i].time, price:bars[i].high});
    if(lo) pl.push({time:bars[i].time, price:bars[i].low});
  }
  return {ph,pl};
}
function drawOverlays(obj){
  // clear old price lines for pivots
  (obj.pivotLines||[]).forEach(l=>{try{obj.series.removePriceLine(l);}catch(e){}});
  obj.pivotLines=[];
  const key=obj.cfg.sym+"|"+obj.cfg.tf;
  const bars=(term.kcache[key]&&term.kcache[key].bars)||[];
    if(term.overlays.pivots && bars.length){
    const {ph,pl}=pivotsApprox(bars);
    // only latest few pivots as markers near the bar — never full-width lines
    const marks=[];
    ph.slice(-4).forEach(p=>marks.push({time:p.time,position:"aboveBar",color:"#f6465d",shape:"arrowDown",text:"",size:0.5}));
    pl.slice(-4).forEach(p=>marks.push({time:p.time,position:"belowBar",color:"#0ecb81",shape:"arrowUp",text:"",size:0.5}));
    obj._pivotMarks = marks;
    if(term.overlays.signals){
      /* merged in drawMarkers */
    } else if(marks.length){
      try{ const pm=obj._pivotMarks||[]; obj.series.setMarkers(marks.concat(pm)); }catch(e){}
    }
  } else {
    obj._pivotMarks = [];
  }
  // session / chop markers via markers strip — use setMarkers merge is hard; draw on canvas overlay instead
  drawSessionChopCanvas(obj, bars);
}
function drawSessionChopCanvas(obj, bars){
  const canvas=$("canvas.setup",obj.el);
  // setup canvas also used for position boxes — redrawn in drawSetup
  // store bars for drawSetup
  obj._bars=bars;
}
function drawSetup(obj){
  // Market-only display: suppress all strategy/position overlays and signal glyphs.
  return;
  const canvas=$("canvas.setup",obj.el);
  const wrap=$(".cwrap",obj.el);
  if(!canvas||!wrap) return;
  canvas.width=wrap.clientWidth; canvas.height=wrap.clientHeight;
  const ctx=canvas.getContext("2d");
  ctx.clearRect(0,0,canvas.width,canvas.height);
  if(!term.overlays.setup && !term.overlays.session) return;
  const chart=obj.chart, series=obj.series;
  const ts=chart.timeScale();
  // session shading Asia/NY (approx)
  if(term.overlays.session && obj._bars){
    obj._bars.forEach(b=>{
      const d=new Date(b.time*1000);
      const h=d.getUTCHours();
      const inSess = h<8 || (h>=13 && h<21);
      if(!inSess) return;
      const x1=ts.timeToCoordinate(b.time);
      const x2=ts.timeToCoordinate(b.time + (tfMs[obj.cfg.tf]/1000));
      if(x1==null||x2==null) return;
      ctx.fillStyle="rgba(240,185,11,0.035)";
      ctx.fillRect(x1,0,Math.max(1,x2-x1),canvas.height);
    });
  }
  if(!term.overlays.setup) return;
  const trades=[];
  if(S&&S.oms){
    Object.values(S.oms.trades||{}).filter(t=>t.symbol===obj.cfg.sym).forEach(t=>trades.push({...t,open:true}));
    (S.oms.closed||[]).filter(t=>t.symbol===obj.cfg.sym).slice(-5).forEach(t=>trades.push({...t,open:false}));
  }
  // latest signal with entry/sl/tp
  const sig=(S&&S.signals||[]).filter(s=>s.symbol===obj.cfg.sym && s.entry!=null).at(-1);
  if(sig) trades.push({
    side:sig.side, entry_price:sig.entry, avg_fill:sig.entry, sl:sig.sl, tp:sig.tp,
    planned_qty:null, filled_qty:null, strategy:sig.strategy, signal:true, open:true,
    created_ms: typeof sig.ts==="number"?sig.ts:Date.parse(sig.ts)
  });
  trades.forEach(t=>{
    const entry=N(t.avg_fill)||N(t.entry_price);
    const sl=N(t.sl), tp=N(t.tp);
    if(!isFinite(entry)||!isFinite(sl)||!isFinite(tp)) return;
    const long=/long/i.test(t.side);
    const yE=series.priceToCoordinate(entry);
    const yS=series.priceToCoordinate(sl);
    const yT=series.priceToCoordinate(tp);
    if(yE==null||yS==null||yT==null) return;
    let x0=0, x1=canvas.width;
    if(t.filled_ms||t.created_ms){
      const t0=Math.floor((t.filled_ms||t.created_ms)/1000);
      const xc=ts.timeToCoordinate(t0);
      if(xc!=null) x0=xc;
    }
    if(t.closed_ms){
      const xc=ts.timeToCoordinate(Math.floor(t.closed_ms/1000));
      if(xc!=null) x1=xc;
    }
    const x=Math.min(x0,x1), w=Math.max(20,Math.abs(x1-x0));
    // profit zone
    ctx.fillStyle=long?"rgba(14,203,129,0.12)":"rgba(14,203,129,0.12)";
    ctx.fillRect(x, Math.min(yE,yT), w, Math.abs(yT-yE));
    // risk zone
    ctx.fillStyle="rgba(246,70,93,0.12)";
    ctx.fillRect(x, Math.min(yE,yS), w, Math.abs(yS-yE));
    // lines
    ctx.strokeStyle="#3d8bfd"; ctx.setLineDash([]); ctx.beginPath(); ctx.moveTo(x,yE); ctx.lineTo(x+w,yE); ctx.stroke();
    ctx.strokeStyle="#f6465d"; ctx.setLineDash([4,3]); ctx.beginPath(); ctx.moveTo(x,yS); ctx.lineTo(x+w,yS); ctx.stroke();
    ctx.strokeStyle="#0ecb81"; ctx.beginPath(); ctx.moveTo(x,yT); ctx.lineTo(x+w,yT); ctx.stroke();
    ctx.setLineDash([]);
    const slPct=Math.abs(entry-sl)/entry*100;
    const rr=Math.abs(tp-entry)/Math.abs(entry-sl);
    const qty=N(t.filled_qty)||N(t.planned_qty);
    const risk=isFinite(qty)?Math.abs(entry-sl)*qty:NaN;
    let upnl="";
    const mark = term._marks && term._marks[obj.cfg.sym];
    if(isFinite(mark)&&isFinite(qty)){
      const u=(long?1:-1)*(mark-entry)*qty;
      upnl=` uPnL $${u.toFixed(2)}`;
    }
    ctx.fillStyle="#eaecef";
    ctx.font="11px IBM Plex Mono,monospace";
    const label=`${long?"LONG":"SHORT"} ${t.strategy||""} E ${fP(entry,obj.cfg.sym)} SL ${fP(sl,obj.cfg.sym)} (${slPct.toFixed(2)}%) TP ${fP(tp,obj.cfg.sym)} RR ${rr.toFixed(2)}${isFinite(qty)?" qty "+qty:""}${isFinite(risk)?" risk $"+risk.toFixed(2):""}${upnl}`;
    ctx.fillText(label, x+4, Math.min(yE,yT,yS)-6);
  });
}
function termOnState(){
  if(!$("#term")) return;
  // refresh markers/setups
  const openSyms=new Set(openTrades().map(t=>t.symbol));
  term.charts.forEach(c=>{
    c.el.classList.toggle("has-pos", openSyms.has(c.cfg.sym));
    const badge=$(".cell-hd .chip.safe",c.el);
    // rebuild badge roughly
    drawMarkers(c); drawSetup(c); updateSigBadge(c);
  });
  renderTable(); renderTrace();
}
function termOnLive(L){
  if(!$("#term")) return;
  term._marks={};
  if(L.tickers){
    for(const [s,t] of Object.entries(L.tickers)){
      if(t.mark||t.last) term._marks[s]=t.mark||t.last;
      term.charts.forEach(obj=>{
        if(obj.cfg.sym!==s) return;
        const px=$("[data-px]",obj.el); if(px && (t.last||t.mark)!=null) px.textContent=fP(t.last||t.mark,s);
        const pctEl=$("[data-pct]",obj.el);
        if(pctEl && t.pct24h!=null){
          const p=Number(t.pct24h);
          pctEl.textContent=(p>=0?"+":"")+p.toFixed(2)+"%";
          pctEl.className="pct24 mono "+(p>=0?"up":"dn");
        }
      });
    }
  }
  // update matching cells from live bars
  term.charts.forEach(async c=>{
    // countdown + WIB close stamp + last-price axis label + close vline
    updateCloseChrome(c);
    // poll live bar for this cell (shared server cache 2s)
    try{
      const live=await jget(`/api/live?symbol=${c.cfg.sym}&interval=${c.cfg.tf}`);
      const bar=live.bars&&live.bars.at(-1);
      if(bar){
        const prev=c.lastBar&&c.lastBar.time;
        c.series.update(bar);
        c.vol.update({time:bar.time,value:bar.volume||0,color:bar.close>=bar.open?"#0ecb8155":"#f6465d55"});
        applyLastPriceStyle(c, bar);
        // candle close flash
        if(c.lastBar && bar.time>c.lastBar.time){
          c.el.classList.remove("flash-close"); void c.el.offsetWidth; c.el.classList.add("flash-close");
          c.closes.push({t:c.lastBar.time,c:c.lastBar.close,o:c.lastBar.open});
          if(c.closes.length>6) c.closes.shift();
          renderCloses(c);
        }
        c.lastBar=bar; setLegend(c,bar);
      }
      if(live.tickers&&live.tickers[c.cfg.sym]){
        const t=live.tickers[c.cfg.sym];
        if(t.mark||t.last) term._marks[c.cfg.sym]=t.mark||t.last;
      }
      drawSetup(c);
    }catch(e){}
  });
  // WS primary attempt (may be blocked)
  openTermWS();
}
function openTermWS(){
  // Live updates stream via /api/stream SSE; avoid direct browser WS to fstream when ISP drops frames
  const el=$("#liveMode");
  if(el && (!el.textContent || el.textContent==="FEED REST")){
    el.textContent="LIVE: SSE+REST";
    el.className="chip safe";
  }
}

function renderTrace(){
  updateTermStatus();
  if(!$("#trace-steps")||!S) return;
  const steps=(S.chain&&S.chain.steps)||[];
  const box=$("#trace-steps");
  const isUnlogged = s => s.status==="unlogged" || s.status==="unknown" || /belum di-log|tidak di-log/i.test(String(s.value||""));
  const logged = steps.filter(s => !isUnlogged(s));
  const unlogged = steps.filter(isUnlogged);
  const stepHtml = (s,i) => `
    <div class="trace-step ${s.status}" data-i="${i}">
      <div class="trace-dot"></div>
      <div><b>${s.label}</b><div class="mono" style="font-size:12.5px;color:#c5d0db;margin-top:2px">${s.value||"—"}</div>
      <div style="font-size:11px;color:#8b98a8;margin-top:2px">${s.source||""}</div></div>
    </div>`;
  let html = logged.map(stepHtml).join("");
  if(unlogged.length){
    html += `<div class="trace-group"><div class="tg-label">Belum di-log engine</div>${unlogged.map(stepHtml).join("")}</div>`;
  }
  box.innerHTML = html;
  $$(".trace-step",box).forEach((el,i)=>{
    setTimeout(()=>el.classList.add("on"), i*70);
  });
  const miss=(S.chain&&S.chain.missing_logs)||[];
  $("#trace-miss").innerHTML = miss.length
    ? `<b style="color:#9aa7b5">Field log yang masih kurang:</b><ul style="margin:4px 0 0 16px;color:#8b98a8">${miss.map(m=>`<li>${m}</li>`).join("")}</ul>`
    : "";
}

function renderTable(){
  if(!$("#tbl")||!S) return;
  const oms=S.oms||{trades:{},closed:[],risk:{}};
  const open=Object.values(oms.trades||{});
  const tab=term.btab||"pos";
  const empty={pos:"NO OPEN RISK",ord:"NO PENDING ORDERS",hist:"NO FILLS YET",sig:"Menunggu sinyal pertama…",log:"LOG SILENT",setup:"NO ACTIVE SETUP"};
  const sd=s=>/long/i.test(s)?`<b class="up">LONG</b>`:`<b class="dn">SHORT</b>`;
  let html="";
  const table=(head,rows)=>{
    if(!rows.length) return `<div class="empty">${empty[tab]}</div>`;
    return `<table class="tbl"><tr>${head.map(h=>`<th>${h}</th>`).join("")}</tr>${rows.map(r=>`<tr>${r.map(c=>`<td>${c}</td>`).join("")}</tr>`).join("")}</table>`;
  };
  if(tab==="pos"){
    html=table(["Simbol","Sisi","Strategi","Qty","Entry","Mark","SL","TP","uPnL","Dibuka"],
      open.filter(t=>N(t.filled_qty)>0).map(t=>{
        const m=term._marks&&term._marks[t.symbol]; const sg=/long/i.test(t.side)?1:-1;
        const u=isFinite(m)?sg*(m-N(t.avg_fill))*N(t.filled_qty):NaN;
        return [`<b>${t.symbol}</b>`,sd(t.side),t.strategy,`<span class="mono">${t.filled_qty}</span>`,
          `<span class="mono">${fP(N(t.avg_fill),t.symbol)}</span>`,`<span class="mono">${fP(m,t.symbol)}</span>`,
          `<span class="mono dn">${fP(N(t.sl),t.symbol)}</span>`,`<span class="mono up">${fP(N(t.tp),t.symbol)}</span>`,
          `<b class="mono ${u>=0?"up":"dn"}">${isFinite(u)?(u>=0?"+":"")+u.toFixed(2):"—"}</b>`, wib(t.filled_ms||t.created_ms)];
      }));
  } else if(tab==="ord"){
    html=table(["Simbol","Sisi","Strategi","Harga","Qty","SL","TP","State","Expire"],
      open.filter(t=>!(N(t.filled_qty)>0)).map(t=>[`<b>${t.symbol}</b>`,sd(t.side),t.strategy,
        `<span class="mono">${fP(N(t.entry_price),t.symbol)}</span>`,`<span class="mono">${t.planned_qty}</span>`,
        `<span class="mono dn">${fP(N(t.sl),t.symbol)}</span>`,`<span class="mono up">${fP(N(t.tp),t.symbol)}</span>`,
        `<span class="chip info">${t.state}</span>`,wib(t.expires_ms)]));
  } else if(tab==="hist"){
    html=table(["Simbol","Sisi","Strategi","Entry","Exit","Jenis","PnL","Tutup"],
      [...(oms.closed||[])].reverse().map(t=>{const p=N(t.pnl_net);
        return [`<b>${t.symbol}</b>`,sd(t.side),t.strategy,`<span class="mono">${fP(N(t.avg_fill),t.symbol)}</span>`,
          `<span class="mono">${fP(N(t.exit_price),t.symbol)}</span>`,t.exit_kind||"—",
          `<b class="mono ${p>=0?"up":"dn"}">${isFinite(p)?(p>=0?"+$":"-$")+Math.abs(p).toFixed(2):"—"}</b>`, t.closed_ms?wib(t.closed_ms):"—"];}));
  } else if(tab==="sig"){
    const rows=(S.signals||[]).slice().reverse().slice(0,100).map(s=>{
      const ts=typeof s.ts==="number"?s.ts:Date.parse(s.ts);
      return [wib(ts), `<span class="chip ${s.taken?"safe":"warn"}">${s.taken?"TAKEN":"SKIP"}</span>`,
        s.symbol||"—", s.strategy||"—", s.side||"—", `<span class="mu">${s.reason||s.kind||""}</span>`];
    });
    html=table(["Waktu","Status","Simbol","Strategi","Sisi","Alasan"], rows);
  } else if(tab==="setup"){
    const rows=open.map(t=>{
      const entry=N(t.avg_fill)||N(t.entry_price); const sl=N(t.sl),tp=N(t.tp);
      const slPct=Math.abs(entry-sl)/entry*100; const rr=Math.abs(tp-entry)/Math.abs(entry-sl);
      return [`<b>${t.symbol}</b>`,sd(t.side),t.strategy, `<span class="mono">${fP(entry,t.symbol)}</span>`,
        `<span class="mono dn">${fP(sl,t.symbol)} (${slPct.toFixed(2)}%)</span>`,
        `<span class="mono up">${fP(tp,t.symbol)}</span>`, `<span class="mono">${rr.toFixed(2)}</span>`,
        `<span class="mono">${t.filled_qty||t.planned_qty}</span>`];
    });
    html=table(["Simbol","Sisi","Strategi","Entry","SL","TP","RR","Qty"], rows);
  } else {
    html=table(["Log"], [...(S.log||[]),...(S.errors||[]).map(x=>"ERR "+x)].reverse().map(l=>[
      `<span class="mono ${/ERR|Error/.test(l)?"dn":/ENTRY/.test(l)?"up":/SKIP/.test(l)?"mu":""}">${l.replace(/[<>&]/g,"")}</span>`]));
  }
  $("#tbl").innerHTML=html;
  const root=$("#term");
  if(root){
    const openN=(openTrades()||[]).length;
    const pending=open.filter(t=>!(N(t.filled_qty)>0)).length;
    const hasRisk = openN>0 || pending>0 || term._forceBottom;
    root.classList.toggle("no-bottom", !hasRisk);
    const tb=$("#togBottom");
    if(tb) tb.textContent = hasRisk && term._forceBottom ? "Panel ↑" : "Panel ↓";
  }
  updateTermStatus();
}

function updateTermStatus(){
  const box=$("#term-status");
  if(!box) return;
  const eng=(S&&S.engine&&S.engine.summary)||{};
  const focus=(term.cells&&term.cells[0])||{sym:"—",tf:"—"};
  const open=(openTrades()||[]).filter(t=>t.symbol===focus.sym);
  const mark=term._marks&&term._marks[focus.sym];
  const longOk=eng.long_ok, shortOk=eng.short_ok;
  let strat="S1 idle";
  if(open.length) strat="S1 · POSISI TERBUKA";
  else if(longOk && shortOk===false) strat="S1 · long_ok · short off";
  else if(longOk===false) strat="S1 · long_ok false";
  const eq=eng.equity!=null?eng.equity:(S&&S.oms&&S.oms.risk&&S.oms.risk.equity);
  const risk=S&&S.oms&&S.oms.risk||{};
  box.innerHTML =
    `<div><b>${focus.sym}</b> · ${focus.tf}</div>`+
    `<div>Harga <b>${mark!=null?fP(mark,focus.sym):"—"}</b></div>`+
    `<div>Regime <b class="${/CHOP|NO_TRADE/i.test(eng.regime||"")?"dn":"up"}">${eng.regime||"—"}</b></div>`+
    `<div>${strat}</div>`+
    `<div style="margin-top:6px;padding-top:6px;border-top:1px solid #1c2430">Equity <b>$${Number(eq||0).toFixed(2)}</b></div>`+
    `<div>Daily SL <b>${risk.day_sl_count??0}/${risk.day_sl_limit??6}</b> · PnL <b>$${Number(risk.day_pnl||0).toFixed(2)}</b></div>`+
    `<div class="mu" style="margin-top:4px">Open risk: ${open.length} · siklus #${eng.cycle_n??"—"}</div>`;
  // also sync per-cell header chips
  term.charts.forEach(c=>{
    const reg=$("[data-reg]",c.el);
    if(reg){ reg.textContent=eng.regime||"—"; reg.className="chip info"; }
    const st=$("[data-strat]",c.el);
    if(st){
      const has=openTrades().some(t=>t.symbol===c.cfg.sym);
      st.textContent = has ? "S1 · POS" : (longOk && shortOk===false ? "S1 · long_ok" : "S1 · idle");
      st.className = "chip "+(has?"safe":"info");
    }
  });
}

/* ========================================================================
   RESEARCH
======================================================================== */
async function researchRender(){
  const el=$("#page-research");
  if(!RESEARCH){ try{ RESEARCH=await jget("/api/research"); }catch(e){ el.innerHTML=`<div class="empty">RESEARCH OFFLINE ·  ${e}</div>`; return; } }
  const R=RESEARCH;
  const seg=(R.segments||[]).map(s=>`<tr>
    <td><b>${s.seg}</b></td><td class="mu">${s.range||""}</td>
    <td class="mono">${s.trades??"—"}</td>
    <td class="mono">${s.e!=null?Number(s.e).toFixed(3)+"R":"—"}</td>
    <td class="mono">${s.pf!=null?Number(s.pf).toFixed(2):"—"}</td>
    <td class="mono">${s.wr!=null?(Number(s.wr)*100).toFixed(1)+"%":"—"}</td>
    <td class="mono">${s.net_usd!=null?"$"+Number(s.net_usd).toFixed(2):"—"}</td>
  </tr>`).join("");
  const gates=(R.gates||[]).map(g=>`<div class="row"><span>${g.id}</span><b class="${g.pass?"pass":"fail"}">${g.pass?"PASS":"FAIL/INCONCLUSIVE"}</b></div>
    <div class="mu" style="font-size:11px;margin:-4px 0 8px">${g.detail||""}</div>`).join("");
  const folds=(R.folds||[]).map(f=>`<tr>
    <td>${f.fold}</td><td class="mu">${(f.test_from||"").slice(0,10)} → ${(f.test_to||"").slice(0,10)}</td>
    <td class="mono">${f.trades}</td><td class="mono ${Number(f.e)>=0?"up":"dn"}">${Number(f.e).toFixed(3)}R</td>
    <td class="mono">${Number(f.pf).toFixed(2)}</td></tr>`).join("");
  el.innerHTML=`<div class="grid" style="grid-template-columns:1.2fr .8fr">
    <div class="card">
      <h3>V10 CANDIDATE · <span class="mono">${R.v10_hash}</span></h3>
      <div class="kpi ${/NO-GO|INCONCLUSIVE/i.test(R.verdict)?"dn":"up"}">${R.verdict}</div>
      <div class="mu" style="margin:6px 0 12px">Angka diparse dari reports/ (bukan hardcode).</div>
      <table class="tbl"><tr><th>Segmen</th><th>Rentang</th><th>Trades</th><th>E[R]</th><th>PF</th><th>WR</th><th>Net</th></tr>${seg}</table>
      <h4 style="margin-top:14px">WF FOLDS</h4>
      <table class="tbl"><tr><th>Fold</th><th>Test</th><th>Trades</th><th>E</th><th>PF</th></tr>${folds||'<tr><td colspan="5" class="mu">tidak ada</td></tr>'}</table>
    </div>
    <div class="card">
      <h3>GATE MATRIX</h3>${gates}
      <h4 style="margin-top:12px">MC · DSR</h4>
      <div class="row"><span>p95 DD</span><b class="mono">${R.mc&&R.mc.p95_dd_r!=null?Number(R.mc.p95_dd_r).toFixed(2)+"R":"—"}</b></div>
      <div class="row"><span>P(&lt;$40)</span><b class="mono">${R.mc&&R.mc.p_below_floor!=null?(Number(R.mc.p_below_floor)*100).toFixed(2)+"%":"—"}</b></div>
      <div class="row"><span>DSR</span><b class="mono">${R.dsr!=null?Number(R.dsr).toFixed(3):"—"}</b></div>
      <div class="row"><span>Max DD</span><b class="mono">${R.metrics&&R.metrics.max_dd_r!=null?Number(R.metrics.max_dd_r).toFixed(2)+"R / $"+Number(R.metrics.max_dd_usd).toFixed(2):"—"}</b></div>
    </div>
  </div>
  <div class="card" style="margin-top:10px">
    <h3>EQ CURVE · V10 BT</h3>
    <canvas id="eqBT" height="80" style="width:100%;height:160px"></canvas>
  </div>
  <div class="card" style="margin-top:10px">
    <h3>TRIALS · TAIL</h3>
    <div class="scroll" style="max-height:240px">
      <table class="tbl"><tr><th>Hash</th><th>Kind</th><th>Trades</th><th>E</th><th>PF</th></tr>
      ${(R.trials||[]).slice(-40).reverse().map(t=>`<tr>
        <td class="mono">${(t.param_hash||"").slice(0,12)}</td><td>${t.kind||"—"}</td>
        <td class="mono">${t.trades??"—"}</td>
        <td class="mono">${t.expectancy_r!=null?Number(t.expectancy_r).toFixed(3):"—"}</td>
        <td class="mono">${t.pf!=null?Number(t.pf).toFixed(2):"—"}</td></tr>`).join("")}
      </table>
    </div>
  </div>`;
  // draw equity
  const cv=$("#eqBT"); if(cv && R.equity&&R.equity.length){
    const g=cv.getContext("2d"); const W=cv.width=cv.clientWidth*2; const H=cv.height=320;
    const data=R.equity.filter(r=>r.balance_usd).map(r=>Number(r.balance_usd));
    if(data.length>1){
      const mn=Math.min(...data), mx=Math.max(...data);
      const X=i=>i/(data.length-1)*W, Y=v=>H-(v-mn)/(mx-mn||1)*(H-16)-8;
      g.beginPath(); data.forEach((v,i)=>i?g.lineTo(X(i),Y(v)):g.moveTo(0,Y(v)));
      g.strokeStyle="#0ecb81"; g.lineWidth=2; g.stroke();
    }
  }
}

/* ========================================================================
   PORTFOLIO
======================================================================== */
function portfolioRender(){
  const el=$("#page-portfolio");
  if(!S){ if(!el.children.length) el.innerHTML=`<div class="empty">SYNC STATE…</div>`; return; }
  const oms=S.oms||{trades:{},closed:[],risk:{}};
  const closed=oms.closed||[];
  const open=Object.values(oms.trades||{});
  const c=S.cycles&&S.cycles.at(-1);
  const eq=c?c.equity:60;
  const realized=closed.reduce((a,t)=>a+(N(t.pnl_net)||0),0);
  if(!$("#eqPaper")){
    el.innerHTML=`<div class="statgrid">
      <div class="stat"><div class="l">Equity paper</div><div class="v mono" id="pf-eq"></div></div>
      <div class="stat"><div class="l">Start</div><div class="v mono">$60.00</div></div>
      <div class="stat"><div class="l">Realized PnL</div><div class="v mono" id="pf-real"></div></div>
      <div class="stat"><div class="l">Open / Closed</div><div class="v mono" id="pf-oc"></div></div>
      <div class="stat"><div class="l">Daily SL</div><div class="v mono" id="pf-dsl"></div></div>
      <div class="stat"><div class="l">Daily PnL</div><div class="v mono" id="pf-dpnl"></div></div>
    </div>
    <div class="card" style="margin-top:10px">
      <h3>EQ · CYCLE MARK</h3>
      <canvas id="eqPaper" style="width:100%;height:120px"></canvas>
      <div id="pf-flat"></div>
    </div>
    <div class="card" style="margin-top:10px">
      <h3>CLOSED BOOK</h3>
      <div id="pf-closed"></div>
    </div>`;
  }
  if($("#pf-eq")) $("#pf-eq").textContent = `$${f2(eq)}`;
  if($("#pf-real")){ $("#pf-real").textContent = `$${f2(realized)}`; $("#pf-real").className = `v mono ${realized>=0?"up":"dn"}`; }
  if($("#pf-oc")) $("#pf-oc").textContent = `${open.length} / ${closed.length}`;
  if($("#pf-dsl")) $("#pf-dsl").textContent = `${(oms.risk||{}).daily_sl_count||0} / 6`;
  if($("#pf-dpnl")) $("#pf-dpnl").textContent = `$${f2((oms.risk||{}).daily_realized_net||0)}`;
  const flatHtml = !(S.cycles&&S.cycles.length)?`<div class="empty">FLAT · $${f2(eq)}</div>`:"";
  if($("#pf-flat") && $("#pf-flat").innerHTML !== flatHtml) $("#pf-flat").innerHTML = flatHtml;
  const closedHtml = closed.length?`<table class="tbl"><tr><th>Simbol</th><th>Sisi</th><th>PnL</th><th>Exit</th></tr>
      ${closed.map(t=>`<tr><td>${t.symbol}</td><td>${t.side}</td><td class="mono ${N(t.pnl_net)>=0?"up":"dn"}">${f2(N(t.pnl_net))}</td><td>${t.exit_kind||"—"}</td></tr>`).join("")}
    </table>`:`<div class="empty">NO FILLS YET</div>`;
  if($("#pf-closed") && $("#pf-closed").innerHTML !== closedHtml) $("#pf-closed").innerHTML = closedHtml;
  const cv=$("#eqPaper");
  if(cv && S.cycles&&S.cycles.length){
    const g=cv.getContext("2d"); const W=cv.width=cv.clientWidth*2; const H=cv.height=240;
    const data=S.cycles.map(x=>x.equity); const mn=Math.min(...data,59.5), mx=Math.max(...data,60.5);
    const X=i=>i/(data.length-1||1)*W, Y=v=>H-(v-mn)/(mx-mn)*(H-10)-5;
    const col=data.at(-1)>=60?"#0ecb81":"#f6465d";
    g.beginPath(); data.forEach((v,i)=>i?g.lineTo(X(i),Y(v)):g.moveTo(0,Y(v))); g.strokeStyle=col; g.lineWidth=2; g.stroke();
  }
}


function flashLastAx(obj, up){
  const ax=$(".last-ax",obj.el); if(!ax) return;
  ax.classList.remove("flash-up","flash-dn"); void ax.offsetWidth;
  ax.classList.add(up?"flash-up":"flash-dn");
}
function applyLastPriceStyle(obj, bar){
  if(!obj || !obj.series || !bar) return;
  const up = Number(bar.close) >= Number(bar.open);
  const col = up ? "#0ecb81" : "#f6465d";
  try{
    obj.series.applyOptions({
      lastValueVisible:true,
      priceLineVisible:true,
      priceLineWidth:1,
      priceLineStyle:(window.LightweightCharts&&LightweightCharts.LineStyle?LightweightCharts.LineStyle.Dashed:2),
      priceLineColor:col
    });
  }catch(e){}
  const ax=$(".last-ax",obj.el);
  if(ax){
    ax.classList.toggle("up", up);
    ax.classList.toggle("dn", !up);
    const p=$(".last-ax-price",ax);
    if(p){
      const prev=p.textContent;
      const next=fP(bar.close, obj.cfg.sym);
      if(prev!==next){ p.textContent=next; flashLastAx(obj, up); }
      else p.textContent=next;
    }
  }
}
function candleCloseMs(obj){
  const ms=tfMs[obj.cfg.tf]||900000;
  const openSec = obj.lastBar && obj.lastBar.time;
  if(openSec) return openSec*1000 + ms;
  const now=Date.now();
  return Math.ceil(now/ms)*ms;
}
function updateCloseChrome(obj){
  if(!obj||!obj.el) return;
  const ms=tfMs[obj.cfg.tf]||900000;
  const now=Date.now();
  const closeAt = candleCloseMs(obj);
  const rem = Math.max(0, closeAt - now);
  const mm=String(Math.floor(rem/60000)).padStart(2,"0");
  const ss=String(Math.floor((rem/1000)%60)).padStart(2,"0");
  const closeWib = new Date(closeAt).toLocaleTimeString("id-ID",{timeZone:WIB,hour:"2-digit",minute:"2-digit",second:"2-digit"})+" WIB";
  const badge=$(".cd-badge",obj.el);
  if(badge) badge.textContent=`${mm}:${ss} · close ${closeWib}`;
  const axCd=$(".last-ax-cd",obj.el);
  if(axCd) axCd.textContent=mm+":"+ss;
  // position last-ax next to last price on right scale
  try{
    const y = obj.series.priceToCoordinate(obj.lastBar ? obj.lastBar.close : null);
    const ax=$(".last-ax",obj.el);
    if(ax && y!=null){ ax.style.display="block"; ax.style.top = (y-18)+"px"; }
    else if(ax) ax.style.display="none";
  }catch(e){}
  // vertical dashed marker at current candle close time
  try{
    const closeSec = Math.floor(closeAt/1000);
    const x = obj.chart.timeScale().timeToCoordinate(closeSec);
    const v=$(".close-vline",obj.el);
    if(v && x!=null){ v.style.display="block"; v.style.left = x+"px"; }
    else if(v) v.style.display="none";
  }catch(e){ const v=$(".close-vline",obj.el); if(v) v.style.display="none"; }
}
async function ensureVikeLiq(sym){
  term._vikeLiq = term._vikeLiq || {};
  const key = (sym||"ETHUSDT").toUpperCase();
  const hit = term._vikeLiq[key];
  if(hit && Date.now()-hit.t < 60000) return hit.data;
  try{
    const data = await jget(`/api/vike/liquidation-heatmap?symbol=${encodeURIComponent(key)}&bins=100`);
    term._vikeLiq[key]={t:Date.now(), data};
    return data;
  }catch(e){
    const data={ok:false, message:String(e), configured:false};
    term._vikeLiq[key]={t:Date.now(), data};
    return data;
  }
}
async function drawLiqHeat(obj){
  const cv=$(".liq-heat",obj.el);
  if(!cv) return;
  const host=obj.el.querySelector(".cell-chart");
  const W=cv.width=host.clientWidth*2;
  const H=cv.height=host.clientHeight*2;
  cv.style.width=host.clientWidth+"px";
  cv.style.height=host.clientHeight+"px";
  const g=cv.getContext("2d");
  g.clearRect(0,0,W,H);
  if(!term.overlays.liq){ cv.style.display="none"; return; }
  cv.style.display="block";
  const data = await ensureVikeLiq(obj.cfg.sym);
  const st=$(".sig-badge",obj.el);
  if(data && data.message && !data.ok){
    // soft status chip via legend suffix
    const leg=$(".legend",obj.el);
    if(leg && !leg.dataset.vikeMsg){ leg.dataset.vikeMsg="1"; /* keep legend clean */ }
  }
  if(!data || !data.ok || !(data.bins||[]).length) return;
  const maxV = Math.max(...data.bins.map(b=>b.liq_usd||0), 1);
  for(const b of data.bins){
    let y1,y2;
    try{
      y1=obj.series.priceToCoordinate(b.price_end);
      y2=obj.series.priceToCoordinate(b.price_start);
    }catch(e){ continue; }
    if(y1==null||y2==null) continue;
    const top=Math.min(y1,y2)*2, bot=Math.max(y1,y2)*2;
    const a=Math.min(0.55, 0.08 + (b.liq_usd/maxV)*0.5);
    // long liq below mark tends red-ish, short above cyan — use long/short split
    const longR = (b.long_usd||0) / Math.max(1, b.liq_usd||1);
    g.fillStyle = longR>=0.5 ? `rgba(246,70,93,${a})` : `rgba(14,203,129,${a})`;
    g.fillRect(0, top, W, Math.max(2, bot-top));
  }
  // mark price dashed
  if(data.mark_price!=null){
    try{
      const ym=obj.series.priceToCoordinate(data.mark_price);
      if(ym!=null){
        g.strokeStyle="rgba(240,185,11,.7)";
        g.setLineDash([6,6]); g.lineWidth=2;
        g.beginPath(); g.moveTo(0, ym*2); g.lineTo(W, ym*2); g.stroke();
        g.setLineDash([]);
      }
    }catch(e){}
  }
}


/* ========================================================================
   HEATMAP
======================================================================== */
async function heatmapRender(){
  if(typeof window.microHeatmapRender==="function"){
    return window.microHeatmapRender();
  }
  const el=$("#page-heatmap");
  el.innerHTML=`<div class="empty">micro.js belum termuat</div>`;
}

/* ========================================================================
   MARKET
======================================================================== */
async function marketRender(){
  const el=$("#page-market");
  if(!el) return;
  if(!MARKET){
    if(!el.children.length) el.innerHTML=`<div class="card"><div class="empty">Memuat data Market…</div></div>`;
    try{ MARKET=await jget("/api/market"); }catch(e){ el.innerHTML=`<div class="card"><div class="empty">Gagal memuat Market: ${e.message||e}</div></div>`; return; }
  } else {
    jget("/api/market").then(m=>{ MARKET=m; }).catch(()=>{});
  }
  const fmtRatio = (obj, k) => {
    if(!obj) return "—";
    if(typeof obj === "object"){
      const v = obj[k] ?? obj.longShortRatio ?? obj.buySellRatio;
      return v != null ? Number(v).toFixed(4) : "—";
    }
    return String(obj);
  };
  const block=(name,m)=>`<div class="card"><h3>${name} PERPETUAL</h3>
    <div class="row"><span>Mark Price</span><b class="mono">${m&&m.mark?fP(Number(m.mark),name+"USDT"):"—"}</b></div>
    <div class="row"><span>Index Price</span><b class="mono">${m&&m.index?fP(Number(m.index),name+"USDT"):"—"}</b></div>
    <div class="row"><span>Funding Rate</span><b class="mono">${m&&m.funding!=null?(Number(m.funding)*100).toFixed(4)+"%":"—"}</b></div>
    <div class="row"><span>Open Interest</span><b class="mono">${m&&m.oi!=null?Number(m.oi).toLocaleString():"—"}</b></div>
    <div class="row"><span>Global Long/Short</span><b class="mono">${fmtRatio(m&&m.long_short,"longShortRatio")}</b></div>
    <div class="row"><span>Taker Buy/Sell</span><b class="mono">${fmtRatio(m&&m.taker,"buySellRatio")}</b></div>
  </div>`;
  const regs = (MARKET&&MARKET.regimes) || (S&&S.cycles) || [];
  const mr09 = (MARKET&&MARKET.mr09) || (S&&S.mr09) || [];
  el.innerHTML=`<div class="grid" style="grid-template-columns:1fr 1fr;gap:10px">
    ${block("BTC",(MARKET&&MARKET.btc)||{})}${block("ETH",(MARKET&&MARKET.eth)||{})}
  </div>
  <div class="card" style="margin-top:10px"><h3>REGIME TAPE · run.log (${regs.length} siklus)</h3>
    <div class="scroll" style="max-height:220px"><table class="tbl"><tr><th>Waktu</th><th>Regime</th></tr>
      ${regs.slice().reverse().slice(0,100).map(r=>`<tr><td class="mono">${r.ts||"—"}</td><td><span class="chip ${/CHOP|NO_TRADE|OFF/i.test(r.regime||"")?"warn":"safe"}">${r.regime||"—"}</span></td></tr>`).join("")||"<tr><td colspan=2 class='mu'>Belum ada siklus regime tercatat di run.log</td></tr>"}
    </table></div>
  </div>
  <div class="card" style="margin-top:10px"><h3>MR-09 SHADOW (Multi-TF Reversal Monitor)</h3>
    ${mr09.length?`<table class="tbl"><tr><th>Waktu</th><th>rev_up</th><th>rev_down</th></tr>
      ${mr09.slice().reverse().map(m=>`<tr><td class="mono">${wib(m.ts)}</td><td class="mono up">${f2(m.rev_up)}</td><td class="mono dn">${f2(m.rev_down)}</td></tr>`).join("")}
    </table>`:`<div class="empty">Belum ada event MR-09 Shadow tercatat di audit.jsonl (hanya aktif saat sinyal reversal dievaluasi)</div>`}
  </div>`;
}

/* ========================================================================
   LEARNING
======================================================================== */
let _learnCharts = {};
function fillLearnHeats(){
  const closed = (S&&S.oms&&S.oms.closed) || (LEARN&&LEARN.journal) || [];
  const pe = $("#learn-paper-empty"); const ph = $("#learn-paper-heat");
  if(pe && ph){
    if(!closed.length){
      pe.textContent = "Belum ada trade paper tertutup — grafik di atas menampilkan distribusi jam sinyal/siklus";
      if(window.echarts){
        const hours = Array.from({length:24},(_,i)=>i);
        const cycByH = hours.map(()=>0);
        ((S&&S.cycles)||[]).forEach(c=>{
          const t = Date.parse(c.ts);
          if(isFinite(t)) cycByH[new Date(t).getUTCHours()]++;
        });
        if(_learnCharts.paper){ try{ _learnCharts.paper.dispose(); }catch(e){} }
        const chP = echarts.init(ph, null, {height:140});
        _learnCharts.paper = chP;
        chP.setOption({
          animation:false, backgroundColor:"transparent",
          grid:{left:36,right:12,top:24,bottom:24},
          tooltip:{trigger:"axis"},
          xAxis:{type:"category",data:hours.map(h=>h+":00"),axisLabel:{color:"#8b98a8",fontSize:9}},
          yAxis:{type:"value",axisLabel:{color:"#8b98a8",fontSize:9},splitLine:{lineStyle:{color:"#232a35"}}},
          series:[{name:"Siklus Engine (UTC)",type:"bar",data:cycByH,itemStyle:{color:"#3d8bfd"}}]
        });
      }
    } else {
      pe.textContent = "";
    }
  }
  const se = $("#learn-sig-empty"); const sh = $("#learn-sig-heat");
  const sigs = (S&&S.signals&&S.signals.length ? S.signals : (LEARN&&LEARN.signals)) || [];
  if(window.echarts && sh){
    const hours = Array.from({length:24},(_,i)=>i);
    const taken = hours.map(()=>0), skipped = hours.map(()=>0);
    sigs.forEach(s=>{
      let ts = s.ts; if(typeof ts==="string") ts = Date.parse(ts); if(ts && !(ts>1e12)) ts = ts*1000;
      if(!isFinite(ts)) return;
      const hh = new Date(ts).getUTCHours();
      if(s.taken || s.kind==="ENTRY") taken[hh]++; else skipped[hh]++;
    });
    if(se) se.textContent = sigs.length ? "" : "Menunggu sinyal ENTRY/SKIP baru dari engine paper";
    if(_learnCharts.sig){ try{ _learnCharts.sig.dispose(); }catch(e){} }
    const ch = echarts.init(sh, null, {height:140});
    _learnCharts.sig = ch;
    ch.setOption({
      animation:false, backgroundColor:"transparent",
      grid:{left:36,right:12,top:28,bottom:24},
      tooltip:{trigger:"axis"},
      legend:{data:["Taken","Skip"],textStyle:{color:"#c5d0db",fontSize:10}},
      xAxis:{type:"category",data:hours.map(h=>h+":00"),axisLabel:{color:"#8b98a8",fontSize:9}},
      yAxis:{type:"value",axisLabel:{color:"#8b98a8",fontSize:9},splitLine:{lineStyle:{color:"#232a35"}}},
      series:[
        {name:"Taken",type:"bar",data:taken,itemStyle:{color:"#0ecb81"}},
        {name:"Skip",type:"bar",data:skipped,itemStyle:{color:"#f0b90b"}}
      ]
    });
  }
}

async function learningRender(){
  const el=$("#page-learning");
  if(!el) return;
  if(!LEARN){
    if(!el.children.length) el.innerHTML=`<div class="card"><div class="empty">Memuat Learning & Evaluasi…</div></div>`;
    try{ LEARN=await jget("/api/learning"); }catch(e){ el.innerHTML=`<div class="card"><div class="empty">Gagal memuat Learning: ${e.message||e}</div></div>`; return; }
  }
  const L = LEARN || {};
  const daysElapsed = Number(L.days_elapsed || 0);
  const daysTarget = Number(L.days_target || 28) || 28;
  const pct = Math.min(100, (daysElapsed / daysTarget) * 100);
  const tvs = (L.pattern && L.pattern.taken_vs_skip) || {taken:0, skipped:0};
  el.innerHTML=`<div class="statgrid">
    <div class="stat"><div class="l">Paper gate 4 minggu</div><div class="v mono">${daysElapsed.toFixed(2)} / ${daysTarget} hari</div>
      <div style="height:6px;background:#222;border-radius:3px;margin-top:6px"><i style="display:block;height:100%;width:${pct}%;background:var(--yl)"></i></div></div>
    <div class="stat"><div class="l">Trade paper</div><div class="v mono">${L.paper_trades ?? 0}</div></div>
    <div class="stat"><div class="l">Taken / Skip</div><div class="v mono">${tvs.taken ?? 0} / ${tvs.skipped ?? 0}</div></div>
  </div>
  <div class="card" style="margin-top:10px"><h3>AUTO EVAL · LIVE DATA</h3>
    <ul>${(L.notes||[]).map(n=>`<li>${n}</li>`).join("") || "<li>Paper mode aktif dan memantau siklus 15m.</li>"}</ul>
  </div>
  <div class="grid" style="grid-template-columns:1fr 1fr;margin-top:10px">
    <div class="card"><h3>SKIP CODES</h3>
      ${Object.keys(L.skip_reasons||{}).length?`<table class="tbl">${Object.entries(L.skip_reasons).map(([k,v])=>`<tr><td>${k}</td><td class="mono">${v}</td></tr>`).join("")}</table>`
      :`<div class="empty">Belum ada kode SKIP tercatat di sesi ini</div>`}
    </div>
    <div class="card"><h3>PAPER × V10 BT</h3>
      <div class="mu">Paper baru jalan — bandingkan setelah ada sampel. Verdict backtest: <b>${(L.vs_backtest||{}).verdict||"—"}</b></div>
      <table class="tbl" style="margin-top:8px"><tr><th>Seg</th><th>Trades</th><th>E</th><th>PF</th></tr>
        ${((L.vs_backtest||{}).v10_segments||[]).map(s=>`<tr><td>${s.seg}</td><td class="mono">${s.trades??"—"}</td>
          <td class="mono">${s.e!=null?Number(s.e).toFixed(3):"—"}</td><td class="mono">${s.pf!=null?Number(s.pf).toFixed(2):"—"}</td></tr>`).join("")}
      </table>
    </div>
  </div>
  <div class="grid" style="grid-template-columns:1fr 1fr;margin-top:10px">
    <div class="card"><h3>PAPER PnL / SIKLUS · JAM UTC <span class="mu">(compact)</span></h3>
      <div id="learn-paper-heat" class="compact-heat" style="height:145px"></div>
      <div class="mu" style="font-size:11px;margin-top:4px" id="learn-paper-empty"></div>
    </div>
    <div class="card"><h3>SIG/SKIP · JAM UTC <span class="mu">(compact)</span></h3>
      <div id="learn-sig-heat" class="compact-heat" style="height:145px"></div>
      <div class="mu" style="font-size:11px;margin-top:4px" id="learn-sig-empty"></div>
    </div>
  </div>
  <div class="card" style="margin-top:10px"><h3>GLOSSARY · FROM CODE</h3>
    ${(L.glossary||[]).map(g=>`<div style="margin-bottom:10px"><b>${g.title}</b><div class="mu">${g.body}</div></div>`).join("")}
  </div>
  <div class="card" style="margin-top:10px"><h3>UNLOGGED FIELDS</h3>
    <ul class="mu">${(L.chain_missing||[]).map(x=>`<li>${x}</li>`).join("")}</ul>
  </div>`;
  fillLearnHeats();
}

/* ========================================================================
   LIBRARY
======================================================================== */
let libTab="data";

async function libraryRender(){
  const el=$("#page-library");
  if(!el) return;
  if(!LIB){
    if(!el.children.length) el.innerHTML=`<div class="card"><div class="empty">Memuat Perpustakaan…</div></div>`;
    try{ LIB=await jget("/api/library"); }catch(e){ el.innerHTML=`<div class="card"><div class="empty">Gagal memuat Perpustakaan: ${e.message||e}</div></div>`; return; }
  }
  el.innerHTML=`<div class="tabs" id="libtabs">
      <span class="tab on" data-lt="data">Data Trading</span>
      <span class="tab" data-lt="learn">Pembelajaran</span>
      <span class="tab" data-lt="eval">Evaluasi</span>
      <span class="tab" data-lt="feedback">Feedback</span>
      <span class="tab" data-lt="misc">Lain-lain</span>
      <div class="sp"></div>
      <input class="search" id="libSearch" placeholder="Cari di perpustakaan…">
    </div>
    <div id="libBody" style="margin-top:10px"></div>`;
  $$("#libtabs .tab").forEach(t=>t.onclick=()=>{libTab=t.dataset.lt; $$("#libtabs .tab").forEach(x=>x.classList.toggle("on",x===t)); libBody();});
  $("#libSearch").oninput=()=>libBody();
  libBody();
}
async function libBody(){
  const q=($("#libSearch")&&$("#libSearch").value||"").toLowerCase();
  const body=$("#libBody"); if(!body||!LIB) return;
  if(libTab==="data"){
    const rows=[
      ...(LIB.cycles||[]).map(c=>({kind:"cycle",ts:c.ts,symbol:"—",strategy:"—",side:"—",extra:c.regime,raw:JSON.stringify(c)})),
      ...(LIB.entries||[]).map(e=>({kind:"entry",...e,extra:"",raw:e.line||""})),
      ...(LIB.skips||[]).map(e=>({kind:"skip",...e,extra:e.reason,raw:e.line||""})),
      ...((LIB.oms&&LIB.oms.closed)||[]).map(t=>({kind:"trade",ts:t.closed_ms,symbol:t.symbol,strategy:t.strategy,side:t.side,extra:t.pnl_net,raw:JSON.stringify(t)})),
      ...(LIB.equity_bt||[]).slice(0,500).map(r=>({kind:"bt",ts:r.signal_utc,symbol:r.symbol,strategy:r.strategy,side:r.side,extra:r.r_mult,raw:JSON.stringify(r)})),
    ].filter(r=>!q || JSON.stringify(r).toLowerCase().includes(q));
    body.innerHTML=`<div class="card"><div class="row"><span>${rows.length} baris</span>
      <button class="btn sm" id="dlCsv">Download CSV</button>
      <button class="btn sm" id="dlJson">Download JSON</button></div>
      <div class="scroll" style="max-height:60vh"><table class="tbl"><tr><th>Jenis</th><th>Waktu</th><th>Simbol</th><th>Strategi</th><th>Sisi</th><th>Extra</th></tr>
      ${rows.slice(0,500).map(r=>`<tr><td>${r.kind}</td><td class="mono">${r.ts||"—"}</td><td>${r.symbol||"—"}</td><td>${r.strategy||"—"}</td><td>${r.side||"—"}</td><td class="mu">${r.extra||""}</td></tr>`).join("")}
      </table></div></div>`;
    $("#dlCsv").onclick=()=>{
      const csv="kind,ts,symbol,strategy,side,extra\n"+rows.map(r=>[r.kind,r.ts,r.symbol,r.strategy,r.side,JSON.stringify(r.extra||"")].join(",")).join("\n");
      const a=document.createElement("a"); a.href=URL.createObjectURL(new Blob([csv],{type:"text/csv"})); a.download="apex-library.csv"; a.click();
    };
    $("#dlJson").onclick=()=>{
      const a=document.createElement("a"); a.href=URL.createObjectURL(new Blob([JSON.stringify(rows,null,2)],{type:"application/json"})); a.download="apex-library.json"; a.click();
    };
  } else if(libTab==="learn"){
    body.innerHTML=`<div class="grid" style="grid-template-columns:280px 1fr;gap:10px">
      <div class="card scroll" style="max-height:70vh">
        <h4>ALPHA NOTES</h4>${(LIB.articles||[]).map(a=>`<div class="nav-item" data-art="${a.id}"><span class="lbl">${a.title}</span></div>`).join("")}
        <h4 style="margin-top:12px">REPO DOCS</h4>${(LIB.docs||[]).map(d=>`<div class="nav-item" data-doc="${d.name}"><span class="lbl">${d.name}</span></div>`).join("")}
      </div>
      <div class="card md scroll" id="libDoc" style="max-height:70vh">Pilih artikel atau dokumen.</div>
    </div>`;
    $$("[data-art]").forEach(x=>x.onclick=()=>{
      const a=LIB.articles.find(i=>i.id===x.dataset.art);
      $("#libDoc").innerHTML=`<h2>${a.title}</h2><p>${a.body}</p>`;
    });
    $$("[data-doc]").forEach(x=>x.onclick=async()=>{
      $("#libDoc").innerHTML="SYNC…";
      try{
        const d=await jget("/api/doc?name="+encodeURIComponent(x.dataset.doc));
        $("#libDoc").innerHTML = window.marked ? marked.parse(d.text) : `<pre>${d.text.replace(/[<>]/g,"")}</pre>`;
      }catch(e){ $("#libDoc").textContent=String(e); }
    });
  } else if(libTab==="eval"){
    body.innerHTML=`<div class="card"><h3>VERDICT</h3>
      <div class="kpi dn">${LIB.verdict||"—"}</div>
      <table class="tbl" style="margin-top:8px"><tr><th>Seg</th><th>Trades</th><th>E</th><th>PF</th></tr>
        ${(LIB.segments||[]).map(s=>`<tr><td>${s.seg}</td><td class="mono">${s.trades??"—"}</td>
          <td class="mono">${s.e!=null?Number(s.e).toFixed(3):"—"}</td><td class="mono">${s.pf!=null?Number(s.pf).toFixed(2):"—"}</td></tr>`).join("")}
      </table>
      <h4 style="margin-top:12px">GATES</h4>
      ${(LIB.gates||[]).map(g=>`<div class="row"><span>${g.id}</span><b class="${g.pass?"pass":"fail"}">${g.pass?"PASS":"FAIL"}</b></div>`).join("")}
    </div>`;
  } else if(libTab==="feedback"){
    let notes={notes:[]}; try{ notes=await jget("/api/notes"); }catch(e){}
    body.innerHTML=`<div class="card">
      <h3>DESK NOTES</h3>
      <div class="mu" style="margin-bottom:8px">Satu-satunya write: append ke dashboard/library/notes.jsonl (localhost only).</div>
      <textarea id="noteText" rows="3" style="width:100%;background:var(--p2);border:1px solid var(--ln);border-radius:8px;padding:8px" placeholder="Catatan…"></textarea>
      <div style="display:flex;gap:8px;margin-top:8px">
        <input id="noteTag" class="search" placeholder="tag (mis. trade, sinyal, umum)" style="max-width:200px">
        <button class="btn" id="noteSave">Simpan</button>
      </div>
      <div id="noteList" style="margin-top:12px">${(notes.notes||[]).map(n=>`
        <div class="note-item" data-id="${n.id}"><div class="mu">${wib(n.ts)} · ${n.tag||"umum"} ${n.edited?"(diedit)":""}</div>
        <div>${(n.text||"").replace(/[<>]/g,"")}</div>
        <div style="margin-top:6px"><button class="btn sm note-del">Hapus</button></div></div>`).join("")||`<div class="empty">NO NOTES</div>`}</div>
    </div>`;
    $("#noteSave").onclick=async()=>{
      const text=$("#noteText").value.trim(); if(!text) return;
      await fetch("/api/notes",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({action:"create",text,tag:$("#noteTag").value||"umum"})});
      libraryRender();
    };
    $$(".note-del").forEach(b=>b.onclick=async()=>{
      const id=b.closest(".note-item").dataset.id;
      await fetch("/api/notes",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({action:"delete",id})});
      libraryRender();
    });
  } else {
    let miles={commits:[],configs:[]}; try{ miles=await jget("/api/milestones"); }catch(e){}
    body.innerHTML=`<div class="grid" style="grid-template-columns:1fr 1fr;gap:10px">
      <div class="card"><h3>GIT TIMELINE · RO</h3>
        <div class="scroll" style="max-height:60vh">${(miles.commits||[]).map(c=>`
          <div style="padding:8px 0;border-bottom:1px solid var(--ln)"><b class="mono">${c.hash}</b>
          <span class="mu">${c.date}</span><div>${c.subject}</div></div>`).join("")}</div>
      </div>
      <div class="card"><h3>CFG SNAPSHOT</h3>
        <div class="scroll" style="max-height:60vh">${(miles.configs||[]).map(c=>`
          <details style="margin-bottom:8px"><summary>${c.name}</summary><pre style="white-space:pre-wrap;font-size:11px">${c.text.replace(/[<>]/g,"")}</pre></details>`).join("")}</div>
      </div>
    </div>`;
  }
}



/* ===== SSE live + RAF tween + symbol picker + indicators ===== */
term.favs = JSON.parse(localStorage.getItem("apex_favs")||'["BTCUSDT","ETHUSDT"]');
term.tweens = new WeakMap();
term.es = null;

function saveFavs(){ localStorage.setItem("apex_favs", JSON.stringify(term.favs)); }

function connectSSE(){
  if(term.es) try{ term.es.close(); }catch(e){}
  const vis = term.cells.map(c=>c.sym+"|"+c.tf).concat(term.cells.map(c=>c.sym));
  const uniq=[...new Set(vis)];
  fetch("/api/visible?set="+encodeURIComponent(uniq.join(","))).catch(()=>{});
  const url="/api/stream?symbols="+encodeURIComponent([...new Set(term.cells.map(c=>c.sym))].join(","))
    +"&intervals="+encodeURIComponent(term.cells.map(c=>c.sym+"|"+c.tf).join(","));
  try{
    term.es = new EventSource(url);
    term.es.onmessage = (ev)=>{
      try{
        const msg=JSON.parse(ev.data);
        if(msg.type==="tick") applyTick(msg);
        if(msg.type==="engine_event"){ crOnEngineEvent(msg); }
        if(msg.type==="hello"){ /* ok */ }
        if(msg.via){ const el=$("#liveMode"); if(el){ el.textContent="FEED "+String(msg.via).toUpperCase(); el.className="chip "+(msg.via==="rest"?"warn":"safe"); } }
      }catch(e){}
    };
    term.es.onerror=()=>{ const el=$("#liveMode"); if(el){ el.textContent="FEED RETRY"; el.className="chip bad"; } };
  }catch(e){}
}

function applyTick(msg){
  if(msg.tickers){
    term._marks = term._marks||{};
    for(const [s,t] of Object.entries(msg.tickers)){
      if(t.mark||t.last) term._marks[s]=t.mark||t.last;
      if(s==="BTCUSDT"&&t.last!=null){ $("#pBTC").textContent="BTC "+fP(t.last,"BTCUSDT"); }
      if(s==="ETHUSDT"&&t.last!=null){ $("#pETH").textContent="ETH "+fP(t.last,"ETHUSDT"); }
      term.charts.forEach(obj=>{
        if(obj.cfg.sym!==s) return;
        const px=$("[data-px]",obj.el); if(px && (t.last||t.mark)!=null) px.textContent=fP(t.last||t.mark,s);
        const pctEl=$("[data-pct]",obj.el);
        if(pctEl && t.pct24h!=null){
          const p=Number(t.pct24h);
          pctEl.textContent=(p>=0?"+":"")+p.toFixed(2)+"%";
          pctEl.className="pct24 mono "+(p>=0?"up":"dn");
        }
      });
    }
  }
  if(!msg.bars) return;
  term.charts.forEach(c=>{
    const key=c.cfg.sym+"|"+c.cfg.tf;
    const bars=msg.bars[key];
    if(!bars||!bars.length) return;
    const bar=bars.at(-1);
    tweenBar(c, bar);
    const px=$(".px-live", c.el); if(px) px.textContent=fP(bar.close, c.cfg.sym);
  });
}

function tweenBar(obj, target){
  const cur = obj.lastBar ? {...obj.lastBar} : {...target};
  const start = performance.now();
  const dur = 180;
  const from = {c:cur.close,h:cur.high,l:cur.low};
  const to = {c:target.close,h:Math.max(cur.high,target.high,target.close),l:Math.min(cur.low,target.low,target.close)};
  if(obj._raf) cancelAnimationFrame(obj._raf);
  const step=(now)=>{
    const k=Math.min(1,(now-start)/dur);
    const e=1-Math.pow(1-k,3);
    const bar={
      time:target.time,
      open:target.open,
      close:from.c+(to.c-from.c)*e,
      high:from.h+(to.h-from.h)*e,
      low:from.l+(to.l-from.l)*e,
      volume:target.volume
    };
    try{
      obj.series.update(bar);
      obj.vol.update({time:bar.time,value:bar.volume||0,color:bar.close>=bar.open?"#0ecb8155":"#f6465d55"});
    }catch(e){}
    if(k<1) obj._raf=requestAnimationFrame(step);
    else {
      obj.lastBar=target;
      setLegend(obj,target);
      drawSetup(obj);
    }
  };
  obj._raf=requestAnimationFrame(step);
}

function openSymbolPicker(cellIdx){
  const fav=new Set(term.favs);
  const overlay=document.createElement("div");
  overlay.className="picker-overlay";
  overlay.innerHTML=`<div class="picker"><div class="ph"><input id="pickQ" placeholder="SYMBOL · BTC ETH …" autofocus>
    <div class="micro" style="margin-top:8px">USDT-M PERP · LOCAL FAVS</div></div><div class="list" id="pickList"><div class="empty">SYNC…</div></div></div>`;
  document.body.appendChild(overlay);
  overlay.onclick=e=>{ if(e.target===overlay) overlay.remove(); };
  const render=(items)=>{
    $("#pickList",overlay).innerHTML = items.slice(0,200).map(it=>`
      <div class="rowi" data-s="${it.symbol}">
        <span><b>${it.symbol}</b> ${fav.has(it.symbol)?'★':''}</span>
        <span class="${(it.pct||0)>=0?'up':'dn'}">${it.pct!=null?((it.pct>=0?'+':'')+it.pct.toFixed(2)+'%'):'—'}</span>
        <span class="mu">${it.qv?('$'+ (it.qv/1e6).toFixed(1)+'M'):''}</span>
      </div>`).join("") || `<div class="empty">NO MATCH</div>`;
    $$(".rowi",overlay).forEach(r=>{
      r.onclick=()=>{
        const sym = r.dataset.s;
        term.cells[cellIdx].sym = sym;
        if(!term.favs.includes(sym)){ term.favs.unshift(sym); term.favs=term.favs.slice(0,24); saveFavs(); }
        saveTerm(); overlay.remove();
        const obj = term.charts[cellIdx];
        if(obj){
          obj.cfg.sym = sym;
          const btn = $(".sym-btn", obj.el); if(btn) btn.textContent = sym;
          const sel = $("select.sym", obj.el); if(sel) sel.value = sym;
          loadCell(obj);
          connectSSE();
        } else {
          termBuildGrid();
        }
      };
      r.oncontextmenu=ev=>{
        ev.preventDefault();
        const s=r.dataset.s;
        if(fav.has(s)) term.favs=term.favs.filter(x=>x!==s); else term.favs.unshift(s);
        saveFavs(); openSymbolPicker(cellIdx); overlay.remove();
      };
    });
  };
  const load=async(q)=>{
    try{
      const d=await jget("/api/symbols?q="+encodeURIComponent(q||""));
      let items=d.items||[];
      // favorites first
      items.sort((a,b)=>(fav.has(b.symbol)?1:0)-(fav.has(a.symbol)?1:0) || (b.qv||0)-(a.qv||0));
      render(items);
    }catch(e){ $("#pickList",overlay).innerHTML=`<div class="empty">${e}</div>`; }
  };
  $("#pickQ",overlay).oninput=e=>load(e.target.value);
  load("");
}

function openIndicatorDialog(cellIdx){
  const obj=term.charts[cellIdx]; if(!obj) return;
  const existing=document.querySelector(".ind-dlg"); if(existing) existing.remove();
  const dlg=document.createElement("div"); dlg.className="ind-dlg card";
  const cat=(window.ApexIndicators&&ApexIndicators.CATALOG)||[];
  const active=obj.indicators||[];
  dlg.innerHTML=`<div class="micro">INDICATORS</div>
    <button class="btn sm" id="presetV10">Preset Strategi V10</button>
    <div style="margin-top:8px">${cat.map(c=>`
      <label style="display:flex;gap:8px;align-items:flex-start;padding:6px 0;border-bottom:1px solid var(--ln);cursor:pointer">
        <input type="checkbox" data-id="${c.id}" ${active.some(a=>a.id===c.id)?"checked":""}>
        <span><b>${c.name}</b><div class="mu" style="font-size:10px" title="${c.src}">${c.src}${c.approx?" · APPROX":""}</div></span>
      </label>`).join("")}</div>
    <div style="margin-top:8px;display:flex;gap:6px"><button class="btn sm" id="indApply">Apply</button><button class="btn sm" id="indClose">Close</button></div>`;
  document.body.appendChild(dlg);
  $("#presetV10",dlg).onclick=()=>{
    obj.indicators=(ApexIndicators.PRESET_V10||[]).map(x=>({...x}));
    persistInd(obj); applyIndicators(obj); dlg.remove();
  };
  $("#indClose",dlg).onclick=()=>dlg.remove();
  $("#indApply",dlg).onclick=()=>{
    obj.indicators=$$("input[data-id]:checked",dlg).map(i=>{
      const c=cat.find(x=>x.id===i.dataset.id)||{};
      return {id:i.dataset.id, ...(c.defaults||{})};
    });
    persistInd(obj); applyIndicators(obj); dlg.remove();
  };
}
function persistInd(obj){
  const key="apex_ind_"+obj.cfg.sym+"_"+obj.cfg.tf;
  localStorage.setItem(key, JSON.stringify(obj.indicators||[]));
  // also by cell index
  const all=JSON.parse(localStorage.getItem("apex_cell_ind")||"{}");
  all[term.charts.indexOf(obj)]=obj.indicators||[];
  localStorage.setItem("apex_cell_ind", JSON.stringify(all));
}
function loadInd(obj, idx){
  const all=JSON.parse(localStorage.getItem("apex_cell_ind")||"{}");
  obj.indicators = all[idx] || JSON.parse(localStorage.getItem("apex_ind_"+obj.cfg.sym+"_"+obj.cfg.tf)||"[]");
}
function applyIndicators(obj){
  (obj.indSeries||[]).forEach(s=>{ try{ obj.chart.removeSeries(s); }catch(e){} });
  obj.indSeries=[];
  if(!window.ApexIndicators||!obj._bars) return;
  (obj.indicators||[]).forEach(ind=>{
    const cat=(ApexIndicators.CATALOG||[]).find(c=>c.id===ind.id)||{};
    const res=ApexIndicators.compute(ind.id, obj._bars, ind);
    if(res.type==="line" && res.values.length){
      const pane=cat.pane==="pane"?"ind_"+ind.id:undefined;
      const series = pane
        ? obj.chart.addLineSeries({color:ind.color||cat.defaults?.color||"#f0b90b", lineWidth:1, priceScaleId:pane})
        : obj.chart.addLineSeries({color:ind.color||cat.defaults?.color||"#3d8bfd", lineWidth:1});
      if(pane) obj.chart.priceScale(pane).applyOptions({scaleMargins:{top:.72,bottom:0}});
      series.setData(res.values);
      obj.indSeries.push(series);
    } else if(res.type==="markers"){
      // merge with signal markers later — draw as tiny circles via setMarkers append is complex; use price lines for last few
      res.values.slice(-30).forEach(pv=>{
        try{
          obj.pivotLines=obj.pivotLines||[];
          /* pivot full-width lines removed */
        }catch(e){}
      });
    }
  });
}

// Hook termBuildGrid: symbol picker + indikator wiring (no duplicate buttons)
const _termBuildGrid = termBuildGrid;
termBuildGrid = function(){
  _termBuildGrid();
  term.charts.forEach((obj,i)=>{
    loadInd(obj,i);
    const hd=$(".cell-hd", obj.el);
    const sel=$("select.sym", hd);
    if(sel && !hd.querySelector(".sym-btn")){
      const btn=document.createElement("button");
      btn.className="sym-btn"; btn.textContent=obj.cfg.sym; btn.title="Pilih simbol USDT-M";
      btn.onclick=()=>openSymbolPicker(i);
      sel.replaceWith(btn);
    } else if(hd.querySelector(".sym-btn")){
      hd.querySelector(".sym-btn").textContent=obj.cfg.sym;
      hd.querySelector(".sym-btn").onclick=()=>openSymbolPicker(i);
    }
    const pick=$(".pick", hd); if(pick) pick.onclick=()=>openSymbolPicker(i);
    const ind=$(".ind", hd); if(ind){ ind.onclick=()=>openIndicatorDialog(i); ind.textContent="Indikator"; }
    // remove any legacy duplicate INDICATORS button
    $$(".ind-btn", hd).forEach(b=>b.remove());
    setTimeout(()=>{ if(obj._bars) applyIndicators(obj); }, 800);
  });
  connectSSE();
  if(window.lucide) try{ lucide.createIcons(); }catch(e){}
};

// Patch loadCell to set _bars and apply indicators
const _loadCell = loadCell;
loadCell = async function(obj){
  await _loadCell(obj);
  applyIndicators(obj);
};

// Patch drawOverlays to keep _bars
const _drawOverlays = drawOverlays;
drawOverlays = function(obj){
  _drawOverlays(obj);
};


// ---------- boot ----------
loadPriceMeta();
pollState().then(()=>{
  crRender();
  if(openTrades().length) go("terminal");
  if(window.lucide) try{ lucide.createIcons(); }catch(e){}
});
pollLive();
// Prefetch pages & terminal timeframes in background so first paint of any page/TF is <100ms
setTimeout(async ()=>{
  try{ if(!MARKET) MARKET = await jget("/api/market"); }catch(e){}
  try{ if(!LEARN) LEARN = await jget("/api/learning"); }catch(e){}
  try{ if(!LIB) LIB = await jget("/api/library"); }catch(e){}
  try{ if(!RESEARCH) RESEARCH = await jget("/api/research"); }catch(e){}
  for(const tf of ["15m","4h","1h","5m"]){
    try{
      const key = "ETHUSDT|" + tf;
      if(!term.kcache[key]){
        const b = await jget(`/api/klines?symbol=ETHUSDT&interval=${tf}`);
        if(Array.isArray(b) && b.length) term.kcache[key] = {t: Date.now(), bars: b};
      }
    }catch(e){}
  }
}, 250);

function ensureGlobalSSE(){
  if(window._apexES) return;
  try{
    const es=new EventSource("/api/stream?symbols=BTCUSDT,ETHUSDT");
    window._apexES=es;
    es.onmessage=(ev)=>{
      try{
        const msg=JSON.parse(ev.data);
        if(msg.type==="engine_event") crOnEngineEvent(msg);
        if(msg.type==="tick" && msg.tickers){
          const b=msg.tickers.BTCUSDT, e=msg.tickers.ETHUSDT;
          if(b&&b.last!=null){ const el=$("#pBTC"); if(el){ el.textContent="BTC "+fP(b.last,"BTCUSDT"); el.className="mono up"; }}
          if(e&&e.last!=null){ const el=$("#pETH"); if(el){ el.textContent="ETH "+fP(e.last,"ETHUSDT"); el.className="mono up"; }}
        }
      }catch(err){}
    };
  }catch(err){}
}
ensureGlobalSSE();

})();
