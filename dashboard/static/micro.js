/* Microstructure Heatmap — zero-flicker incremental ECharts updates, stale-while-revalidate,
   background tab prefetch, full-window WIB alignment for Wall Lifelines, Liq Map, Large Trades + VP, and Tier & Flow */
(() => {
"use strict";
const $ = (s, el=document) => el.querySelector(s);
const $$ = (s, el=document) => [...el.querySelectorAll(s)];
const WIB = "Asia/Jakarta";
const wibNow = (ms=Date.now()) => new Date(ms).toLocaleString("id-ID",{timeZone:WIB,hour:"2-digit",minute:"2-digit",second:"2-digit"})+" WIB";
const fmtWibAxis = (ms, win) => {
  const opts = (win==="1h"||win==="4h")
    ? {timeZone:WIB,hour:"2-digit",minute:"2-digit"}
    : {timeZone:WIB,hour:"2-digit",minute:"2-digit",second:"2-digit"};
  return new Date(ms).toLocaleTimeString("id-ID", opts);
};
const ageStr = (s) => (s==null?"—":(s<1?`${Math.round(s*1000)}ms`:`${s.toFixed(1)}s`));
const fmtUsdCompact = (v) => {
  const n = Math.abs(Number(v) || 0);
  const sign = Number(v) < 0 ? "-" : "";
  if(n >= 1e6) return `${sign}$${(n/1e6).toFixed(2)}M`;
  if(n >= 1e3) return `${sign}$${(n/1e3).toFixed(1)}K`;
  return `${sign}$${Math.round(n)}`;
};

const TABS = [
  {id:"command", label:"0 · COMMAND CENTER", src:"MIXED"},
  {id:"orderflow", label:"1 · ORDER FLOW", src:"BINANCE"},
  {id:"walls", label:"2 · WALL LIFELINES", src:"BINANCE"},
  {id:"liqmap", label:"3 · LIQ MAP", src:"VIKE"},
  {id:"largevp", label:"4 · LARGE TRADES + VP", src:"BINANCE"},
  {id:"tierflow", label:"5 · TIER & FLOW", src:"BINANCE"},
  {id:"cohorts", label:"6 · COHORTS (Vike)", src:"VIKE"},
  {id:"funding_oi", label:"7 · FUNDING & OI", src:"VIKE"},
  {id:"liq_risk", label:"8 · LIQ RISK + CLUSTERS", src:"VIKE"},
  {id:"market", label:"9 · USDT-M HEAT", src:"BINANCE"},
];
const WINDOWS = ["5m","15m","1h","4h"];

const MICRO = {
  tab:"orderflow", sym:"ETHUSDT", window:"15m",
  mountedTab:null,
  charts:{}, chartSig:{},
  es:null, lastSnap:{}, updateCounts:{},
  raf:0, pending:null, alive:false, visible:true,
  fps:0, frames:0, fpsT:0,
  tape:[], lastTapeSig:"", prevTreemap:{},
  animLoop:0,
  inFlight:false,
  prefetchTimer:0,
  lastHealthPaint:0,
};

function snapKey(tab=MICRO.tab, sym=MICRO.sym, win=MICRO.window){
  return tab==="market" ? "market:ALL" : `${sym}:${tab}:${win}`;
}

function jget(url, timeoutMs=6000){
  const ctrl = new AbortController();
  const tid = setTimeout(() => ctrl.abort(), timeoutMs);
  return fetch(url, {cache:"no-store", signal:ctrl.signal}).then(async r => {
    clearTimeout(tid);
    if(!r.ok) throw new Error(`HTTP ${r.status} (${url.split("?")[0]})`);
    return r.json();
  }).catch(err => {
    clearTimeout(tid);
    if(err && err.name === "AbortError"){
      throw new Error(`Timeout ${Math.round(timeoutMs/1000)}s pada ${url.split("?")[0]}`);
    }
    throw err;
  });
}

function ensureChart(host, key){
  if(!window.echarts || !host) return null;
  let ch = MICRO.charts[key];
  if(ch && !ch.isDisposed() && ch.getDom()===host) return ch;
  if(ch){ try{ ch.dispose(); }catch(e){} }
  const dpr = Math.min(2, window.devicePixelRatio||1);
  ch = echarts.init(host, null, {renderer:"canvas", devicePixelRatio:dpr});
  MICRO.charts[key] = ch;
  delete MICRO.chartSig[key];
  return ch;
}

/* Skip redundant ECharts setOption when data fingerprint is unchanged — eliminates canvas pulse/flicker */
function shouldPaint(key, sig){
  if(!sig) return true;
  if(MICRO.chartSig[key] === sig) return false;
  MICRO.chartSig[key] = sig;
  return true;
}

function disposeCharts(){
  Object.values(MICRO.charts).forEach(ch => { try{ ch.dispose(); }catch(e){} });
  MICRO.charts = {};
  MICRO.chartSig = {};
  MICRO.mountedTab = null;
}

function setText(el, txt){
  if(!el) return;
  const s = String(txt ?? "");
  if(el.textContent !== s) el.textContent = s;
}

function setClass(el, cls){
  if(!el) return;
  if(el.className !== cls) el.className = cls;
}

function setSourceBanner(source, errMsg){
  const el = $("#micro-err-banner");
  if(!el) return;
  if(!errMsg){
    if(el.style.display !== "none") el.style.display = "none";
    setText(el, "");
    return;
  }
  if(el.style.display !== "inline-flex") el.style.display = "inline-flex";
  setText(el, `⚠ [${source||"UPSTREAM"}] ${errMsg} (menampilkan cache terakhir)`);
}

function updateHealthDOM(h, force=false){
  if(!h) return;
  const now = Date.now();
  if(!force && (now - MICRO.lastHealthPaint < 1500)) return;
  MICRO.lastHealthPaint = now;
  const hh = $("#micro-health");
  if(!hh) return;
  const feeds = h.feeds || (h.health && h.health.feeds) || [];
  const w = h.weight || (h.health && h.health.weight) || {};
  if(!feeds.length) return;

  // Build skeleton chips once, then update textContent in-place to prevent DOM layout reflow/flicker
  if(hh.children.length !== feeds.length + 1){
    hh.innerHTML = feeds.map(f => {
      const cls = f.status==="ok" ? "safe" : (f.status==="STALE" ? "bad" : "warn");
      return `<span class="chip ${cls}" data-feed="${f.name}">${f.name} ${f.status==="STALE"?"STALE":ageStr(f.age_s)} · ${f.via||"—"}</span>`;
    }).join(" ") + ` <span class="chip info" data-feed="__wt" title="Binance REST weight/min">wt ${w.used_this_min??"—"}/${w.budget??1200}</span>`;
    return;
  }
  feeds.forEach(f => {
    const chip = hh.querySelector(`[data-feed="${f.name}"]`);
    if(chip){
      const cls = "chip " + (f.status==="ok" ? "safe" : (f.status==="STALE" ? "bad" : "warn"));
      setClass(chip, cls);
      setText(chip, `${f.name} ${f.status==="STALE"?"STALE":ageStr(f.age_s)} · ${f.via||"—"}`);
    }
  });
  const wchip = hh.querySelector(`[data-feed="__wt"]`);
  if(wchip){
    setText(wchip, `wt ${w.used_this_min??"—"}/${w.budget??1200}`);
  }
}

function toolbarHTML(){
  return `
  <div class="micro-toolbar row" style="gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:8px">
    <select id="micro-sym" class="sym-dark">
      <option>ETHUSDT</option><option>BTCUSDT</option><option>SOLUSDT</option>
    </select>
    <div class="row" style="gap:4px" id="micro-wins">
      ${WINDOWS.map(w=>`<button type="button" class="btn sm win ${w===MICRO.window?"on":""}" data-win="${w}">${w}</button>`).join("")}
    </div>
    <span id="micro-src" class="chip info">SRC —</span>
    <span id="micro-upd" class="chip">upd —</span>
    <span id="micro-fps" class="chip">fps —</span>
    <span id="micro-age" class="mu mono"></span>
    <span id="micro-err-banner" class="chip warn" style="display:none"></span>
    <div class="sp"></div>
    <div id="micro-health" class="row" style="gap:4px;flex-wrap:wrap"></div>
  </div>
  <div class="tabs" id="micro-tabs" style="flex-wrap:wrap">
    ${TABS.map(t=>`<span class="tab ${t.id===MICRO.tab?"on":""}" data-mt="${t.id}">${t.label}</span>`).join("")}
  </div>
  <div id="micro-tape" class="micro-tape mono" aria-hidden="true"></div>`;
}

/* Build tab DOM skeleton ONCE per tab switch — never wipe innerHTML on data refresh */
function ensureTabDOM(body, tab){
  if(!body) return;
  if(MICRO.mountedTab === tab && body.children.length > 0) return;
  Object.values(MICRO.charts).forEach(ch => { try{ ch.dispose(); }catch(e){} });
  MICRO.charts = {};
  MICRO.chartSig = {};
  MICRO.mountedTab = tab;

  if(tab === "command"){
    body.innerHTML = `<div id="command-grid" class="command-grid">
      <div class="command-kpis" id="command-kpis"></div>
      <div class="command-main card"><div class="command-title"><span>SCALPING MARKET READ</span><span id="command-state" class="chip info">WAITING</span></div><div id="command-read" class="command-read">Menunggu data Binance + Vike…</div><div id="command-evidence" class="command-evidence"></div></div>
      <div class="command-flow card"><div class="command-title"><span>FLOW TOPOLOGY</span><span id="command-score" class="mono">score —</span></div><div class="command-topology"><span class="topo-node" data-node="binance">BINANCE FEED</span><span class="topo-arrow">→</span><span class="topo-node" data-node="vike">VIKE CONTEXT</span><span class="topo-arrow">→</span><span class="topo-node" data-node="regime">REGIME</span><span class="topo-arrow">→</span><span class="topo-node" data-node="read">SCALP READ</span><span class="topo-arrow">→</span><span class="topo-node" data-node="paper">PAPER WATCH</span></div></div>
      <div class="command-terminal card">
        <div class="command-visual command-profile"><div class="command-panel-title">VOLUME PROFILE <span id="command-poc" class="chip info">POC —</span></div><div id="command-profile-chart" style="height:100%;min-height:0"></div></div>
        <div class="command-visual command-book"><div class="command-panel-title">ORDER BOOK / DEPTH <span id="command-spread" class="chip info">spread —</span></div><div id="command-book-grid"></div></div>
        <div class="command-visual command-metrics"><div class="command-panel-title">MARKET METRICS <span class="chip info">BINANCE</span></div><div id="command-metrics-grid"></div></div>
        <div class="command-visual command-price-chart"><div class="command-panel-title">PRICE / OHLC <span id="command-price" class="hero sm">—</span></div><div id="command-candle" class="command-candle"></div></div>
        <div class="command-visual command-liq"><div class="command-panel-title">LIQUIDITY / CONCENTRATION MAP <span class="chip safe">VIKE CONTEXT</span></div><div id="command-liq-chart" style="height:100%;min-height:0"></div></div>
      </div>
      <div class="command-side card"><div class="command-title"><span>CONTEXT COMPONENTS</span><span class="mu">READ ONLY</span></div><div id="command-components"></div><div class="command-source-note">Vike panels are Hyperliquid context. Binance depth/price remains the execution-venue reference.</div></div>
      <div class="command-footer" id="command-footer">BINANCE FEED → VIKE CONTEXT → NORMALIZE → REGIME → SCALP READ · EXECUTION DISABLED</div>
    </div>`;
  } else if(tab === "orderflow"){
    body.innerHTML = `<div class="grid" style="grid-template-columns:1.2fr 1fr;gap:10px;height:100%">
      <div style="height:100%"><div class="mu" id="of-hdr">Footprint delta · aggressor · window ${MICRO.window}</div>
        <div id="of-bars" style="height:calc(100% - 22px)"></div></div>
      <div style="height:100%"><div class="mu">CVD <span id="cvd-head" class="chip safe">head</span></div>
        <div id="of-cvd" style="height:calc(100% - 22px)"></div></div>
    </div>`;
  } else if(tab === "walls"){
    body.innerHTML = `<div class="row" style="gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:6px">
        <span class="mu" id="wall-hdr">Orderbook Wall Lifelines (Support Bid vs Resistance Ask) + Candlestick (${MICRO.window} WIB)</span>
        <span id="wall-book" class="chip info mono">mid —</span>
        <span id="wall-bid-top" class="chip safe mono">Bid Wall —</span>
        <span id="wall-ask-top" class="chip bad mono">Ask Wall —</span>
        <span id="wall-imb" class="chip mono" style="margin-left:auto">Imbalance —</span>
      </div>
      <div class="grid" style="grid-template-columns:1.55fr 0.75fr;gap:10px;height:calc(100% - 34px)">
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Wall Lifelines Trajectory + Harga OHLC (${MICRO.window} WIB)</div>
          <div id="wall-chart" style="height:calc(100% - 18px)"></div>
        </div>
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Live Depth20 Orderbook Ladder (Bid vs Ask Notional USD)</div>
          <div id="wall-book-chart" style="height:calc(100% - 18px)"></div>
        </div>
      </div>`;
  } else if(tab === "liqmap"){
    body.innerHTML = `<div class="row" style="gap:10px;flex-wrap:wrap;align-items:center;margin-bottom:6px">
        <span id="liq-status" class="chip safe">VIKE: OK</span>
        <span class="mu mono" id="liq-mark">mark=— · bins=—</span>
        <div class="row" style="gap:6px;align-items:center;margin-left:auto">
          <span class="mu" style="font-size:11px">Intensitas Likuidasi:</span>
          <span style="display:inline-block;width:90px;height:10px;border-radius:3px;background:linear-gradient(90deg,rgba(59,130,246,0.25),rgba(6,182,212,0.5),rgba(245,158,11,0.75),rgba(246,70,93,0.95));border:1px solid #2b3139"></span>
          <span class="mu mono" style="font-size:10px">Low → Extreme</span>
          <span class="chip safe" style="font-size:10px">Long Liq</span>
          <span class="chip bad" style="font-size:10px">Short Liq</span>
        </div>
      </div>
      <div class="grid" style="grid-template-columns:1.55fr 0.85fr;gap:10px;height:calc(100% - 36px)">
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Candlestick (${MICRO.window} WIB) + Vike Liquidation Heatmap Layer + Realized Liq</div>
          <div id="liq-main" style="height:calc(100% - 18px)"></div>
        </div>
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Liquidation Ladder (Long vs Short Value USD)</div>
          <div id="liq-ladder" style="height:calc(100% - 18px)"></div>
        </div>
      </div>`;
  } else if(tab === "largevp"){
    body.innerHTML = `<div class="row" style="gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:6px">
        <span class="mu" id="lt-hdr">Candlestick (${MICRO.window} WIB) + Large Trades Bubbles (Notional USD) + Volume Profile</span>
        <span id="lt-poc-chip" class="chip warn mono">POC —</span>
        <span id="lt-vah-chip" class="chip info mono">VAH —</span>
        <span id="lt-val-chip" class="chip info mono">VAL —</span>
        <span class="chip safe" style="font-size:10px;margin-left:auto">● Buy Aggressor</span>
        <span class="chip bad" style="font-size:10px">● Sell Aggressor</span>
      </div>
      <div class="grid" style="grid-template-columns:1.58fr 0.72fr;gap:10px;height:calc(100% - 34px)">
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Harga OHLC + Large Trade Bubbles (Ukuran = Notional USD)</div>
          <div id="lt-bubbles" style="height:calc(100% - 18px)"></div>
        </div>
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Volume Profile (Buy vs Sell Volume per Level Harga)</div>
          <div id="lt-vp" style="height:calc(100% - 18px)"></div>
        </div>
      </div>`;
  } else if(tab === "tierflow"){
    body.innerHTML = `<div class="row" style="gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:6px">
        <span class="mu" id="tier-hdr">Order Flow Berdasarkan Ukuran Trade (Tier Breakdown · Window ${MICRO.window} WIB)</span>
        <div class="row" style="gap:5px;margin-left:auto;flex-wrap:wrap">
          <span class="chip" style="border-color:#8b98a8;color:#c5d0db;font-size:10px">Retail &lt;$1K</span>
          <span class="chip" style="border-color:#3d8bfd;color:#3d8bfd;font-size:10px">Mid $1K–$10K</span>
          <span class="chip" style="border-color:#f0b90b;color:#f0b90b;font-size:10px">Large $10K–$50K</span>
          <span class="chip" style="border-color:#0ecb81;color:#0ecb81;font-size:10px">Whale $50K–$250K</span>
          <span class="chip" style="border-color:#f6465d;color:#f6465d;font-size:10px">Mega ≥$250K</span>
        </div>
      </div>
      <div class="grid" style="grid-template-columns:1.45fr 0.85fr;gap:12px;height:calc(100% - 34px)">
        <div style="height:100%">
          <div class="mu" style="font-size:11px;margin-bottom:2px">Net Order Flow per Tier Sepanjang Window (${MICRO.window} WIB)</div>
          <div id="tier-bars" style="height:calc(100% - 18px)"></div>
        </div>
        <div style="height:100%;display:grid;grid-template-rows:1.05fr 0.95fr;gap:8px">
          <div style="height:100%;min-height:0">
            <div class="mu" style="font-size:11px;margin-bottom:2px">Proporsi Volume Notional per Tier (%)</div>
            <div id="tier-pie" style="height:calc(100% - 18px)"></div>
          </div>
          <div style="height:100%;min-height:0">
            <div class="mu" style="font-size:11px;margin-bottom:2px">Buy vs Sell Volume per Tier (USD)</div>
            <div id="tier-net" style="height:calc(100% - 18px)"></div>
          </div>
        </div>
      </div>`;
  } else if(tab === "cohorts"){
    body.innerHTML = `<div class="row" style="gap:8px;margin-bottom:8px">
        <span id="coh-status" class="chip safe">VIKE cohorts</span>
        <span id="coh-msg" class="mu mono"></span>
      </div>
      <div class="grid" style="grid-template-columns:1fr 1fr;gap:10px;height:calc(100% - 40px)">
        <div id="coh-metrics" style="height:100%"></div><div id="coh-size" style="height:100%"></div>
      </div>`;
  } else if(tab === "funding_oi"){
    body.innerHTML = `<div class="statgrid">
        <div class="stat"><div class="l" id="fund-lbl">Funding</div><div class="v mono" id="fund-v">—</div></div>
        <div class="stat"><div class="l">Symbols Tracked</div><div class="v mono" id="fund-n">—</div></div>
      </div>
      <div id="fund-chart" style="height:calc(100% - 90px);margin-top:10px"></div>`;
  } else if(tab === "liq_risk"){
    body.innerHTML = `<div class="grid" style="grid-template-columns:1fr 1fr;gap:10px;height:100%">
        <div id="risk-chart" style="height:100%"></div><div id="cluster-chart" style="height:100%"></div>
      </div>`;
  } else if(tab === "market"){
    body.innerHTML = `<div class="row" style="gap:8px;align-items:center;margin-bottom:4px">
        <span class="mu">USDT-M HEAT · 24H % / Quote Volume</span>
        <span id="hm-market-err" class="mu mono"></span>
      </div>
      <div id="hm-market" style="height:calc(100% - 28px);width:100%"></div>`;
  }
}

window.microHeatmapRender = async function microHeatmapRender(){
  const el = $("#page-heatmap"); if(!el) return;
  stopMicro(false);
  MICRO.alive = true; MICRO.visible = true;
  if(!$("#micro-body", el)){
    disposeCharts();
    el.innerHTML = `
      <div class="card micro-card" style="min-height:calc(100vh - 120px)">
        ${toolbarHTML()}
        <div id="micro-body" style="margin-top:8px;height:calc(100vh - 220px);min-height:600px"></div>
      </div>`;
    $("#micro-sym").value = MICRO.sym;
    $("#micro-sym").onchange = () => {
      MICRO.sym = $("#micro-sym").value;
      MICRO.chartSig = {};
      softReload(true);
      restartSSE();
      schedulePrefetch();
    };
    $$("#micro-wins .win").forEach(b => b.onclick = () => {
      MICRO.window = b.dataset.win;
      $$("#micro-wins .win").forEach(x => x.classList.toggle("on", x===b));
      MICRO.chartSig = {};
      softReload(true);
      restartSSE();
      schedulePrefetch();
    });
    $$("#micro-tabs .tab").forEach(t => t.onclick = () => {
      const nextTab = t.dataset.mt;
      if(nextTab === MICRO.tab) return;
      MICRO.tab = nextTab;
      $$("#micro-tabs .tab").forEach(x => x.classList.toggle("on", x===t));
      softReload(true);
    });
  }
  document.addEventListener("visibilitychange", onVis);
  window.addEventListener("resize", onResize);
  startAnimLoop();
  await softReload(true);
  startSSE();
  schedulePrefetch();
};

function onVis(){ MICRO.visible = document.visibilityState === "visible"; }
function onResize(){
  Object.values(MICRO.charts).forEach(ch => {
    try{ if(ch && !ch.isDisposed()) ch.resize(); }catch(e){}
  });
}

function snapUrl(tab, sym=MICRO.sym, win=MICRO.window){
  return `/api/micro/snapshot?symbol=${encodeURIComponent(sym)}&tab=${encodeURIComponent(tab)}&window=${encodeURIComponent(win)}`;
}

function schedulePrefetch(){
  if(MICRO.prefetchTimer) clearTimeout(MICRO.prefetchTimer);
  MICRO.prefetchTimer = setTimeout(prefetchOtherTabs, 150);
}

async function prefetchOtherTabs(){
  const sym = MICRO.sym, win = MICRO.window;
  for(const t of TABS){
    if(!MICRO.alive) return;
    if(t.id === MICRO.tab) continue;
    const key = snapKey(t.id, sym, win);
    const existing = MICRO.lastSnap[key];
    if(existing && (Date.now() - (existing._fetchedAt||0)) < 10000) continue;
    try{
      if(t.id === "market"){
        const heat = await jget("/api/heatmap", 5000);
        heat._fetchedAt = Date.now();
        MICRO.lastSnap[key] = heat;
      } else {
        const snap = await jget(snapUrl(t.id, sym, win), 5000);
        snap._fetchedAt = Date.now();
        MICRO.lastSnap[key] = snap;
      }
    }catch(e){}
  }
}

async function softReload(isUserSwitch){
  const body = $("#micro-body"); if(!body) return;
  const tab = MICRO.tab;
  const sym = MICRO.sym;
  const win = MICRO.window;
  const key = snapKey(tab, sym, win);

  // 1. Ensure tab skeleton is mounted (never wipes innerHTML if already on this tab!)
  ensureTabDOM(body, tab);

  // 2. Stale-while-revalidate: if we already have cached data for this tab, paint it immediately (<5ms)
  const cached = MICRO.lastSnap[key] || MICRO.lastSnap[tab];
  if(cached && isUserSwitch){
    if(tab === "market"){
      paintMarketData(cached);
    } else {
      paintTab(cached);
      setMeta(cached.source || "CACHE", cached.ts || Date.now(), cached.health || cached, true);
    }
  }

  if(MICRO.inFlight && !isUserSwitch) return;
  MICRO.inFlight = true;
  try{
    if(tab === "command"){
      const snap = await jget(snapUrl("command", sym, win), 6000);
      snap._fetchedAt = Date.now();
      MICRO.lastSnap[key] = snap;
      MICRO.lastSnap.command = snap;
      MICRO.updateCounts.command = (MICRO.updateCounts.command || 0) + 1;
      setSourceBanner("MIXED", snap.stale ? (snap.message || snap.error) : null);
      if(MICRO.tab === tab){ paintCommand(body, snap); setMeta("MIXED", snap.ts, snap.health, isUserSwitch); }
      return;
    }
    if(tab === "market"){
      const heat = await jget("/api/heatmap", 6000);
      heat._fetchedAt = Date.now();
      MICRO.lastSnap[key] = heat;
      MICRO.lastSnap.market = heat;
      MICRO.updateCounts.market = (MICRO.updateCounts.market || 0) + 1;
      setSourceBanner("BINANCE", heat.stale ? (heat.message || heat.error) : null);
      if(MICRO.tab === "market"){
        body.querySelectorAll(".empty-err").forEach(x => x.remove());
        paintMarketData(heat);
      }
      return;
    }
    const snap = await jget(snapUrl(tab, sym, win), 6000);
    snap._fetchedAt = Date.now();
    MICRO.lastSnap[key] = snap;
    MICRO.lastSnap[tab] = snap;
    MICRO.updateCounts[tab] = (MICRO.updateCounts[tab] || 0) + 1;
    setSourceBanner(snap.source || "UPSTREAM", null);
    if(MICRO.tab === tab){
      body.querySelectorAll(".empty-err").forEach(x => x.remove());
      paintTab(snap);
      setMeta(snap.source || "—", snap.ts, snap.health || snap, isUserSwitch);
    }
    if(snap.large){ snap.large.slice(-15).forEach(pushTape); }
  }catch(e){
    const src = (TABS.find(x => x.id === tab) || {}).src || "SERVER";
    setSourceBanner(src, e.message || String(e));
    if(!cached){
      const emptyHost = body.querySelector(".empty-err");
      if(!emptyHost){
        body.insertAdjacentHTML("afterbegin", `<div class="empty empty-err" style="color:#f0b90b">Gagal memuat dari [${src}]: ${e.message||e}</div>`);
      }
    }
  }finally{
    MICRO.inFlight = false;
  }
}

function markFeedFreshness(snap){
  if(!snap || !snap.health) return;
  const feeds = snap.health.feeds || [];
  const stale = feeds.some(f => String(f.status||"").toUpperCase()==="STALE");
  const age = Math.max(...feeds.map(f => Number(f.age_s)||0), 0);
  snap.stale = stale || age > 45;
  snap.feed_age_s = age;
  snap.feed_status = stale ? "STALE" : "LIVE";
}

function setMeta(src, ts, health, forceHealth=false){
  if(health && health.feeds) markFeedFreshness({health});
  if(health && (health.feeds || (health.health && health.health.feeds))){
    const hs = health.feeds ? health : health.health;
    const feeds = hs.feeds || [];
    const stale = feeds.some(f => String(f.status||"").toUpperCase()==="STALE");
    const age = Math.max(...feeds.map(f => Number(f.age_s)||0), 0);
    const srcEl0 = $("#micro-src");
    if(srcEl0 && stale) setClass(srcEl0, "chip bad");
    if(srcEl0 && !stale && src !== "VIKE") setClass(srcEl0, "chip " + (src === "MIXED" ? "info" : "info"));
    const ageEl0 = $("#micro-age");
    if(ageEl0 && age > 45) ageEl0.style.color = "#f04f68";
    else if(ageEl0) ageEl0.style.color = "";
  }
  const srcEl = $("#micro-src");
  if(srcEl){
    setText(srcEl, "SRC " + src);
    setClass(srcEl, "chip " + (src==="VIKE" ? "safe" : "info"));
  }
  const upd = $("#micro-upd");
  if(upd) setText(upd, `upd ${MICRO.updateCounts[MICRO.tab]||1}`);
  const age = $("#micro-age");
  if(age) setText(age, `updated ${wibNow(ts)} · ${MICRO.window}`);
  if(health && (health.feeds || (health.health && health.health.feeds))){
    updateHealthDOM(health.feeds ? health : health.health, forceHealth);
  }
}

function paintTab(snap){
  const body = $("#micro-body"); if(!body || !snap) return;
  const tab = snap.tab || MICRO.tab;
  ensureTabDOM(body, tab);
  const errEl = body.querySelector(".empty-err");
  if(errEl) errEl.remove();
  if(tab === "command") return paintCommand(body, snap);
  if(tab === "orderflow") return paintOrderFlow(body, snap);
  if(tab === "walls") return paintWalls(body, snap);
  if(tab === "liqmap") return paintLiqMap(body, snap);
  if(tab === "largevp") return paintLargeVP(body, snap);
  if(tab === "tierflow") return paintTier(body, snap);
  if(tab === "cohorts") return paintCohorts(body, snap);
  if(tab === "funding_oi") return paintFunding(body, snap);
  if(tab === "liq_risk") return paintLiqRisk(body, snap);
}

/* Disable ECharts entry/component pulse animations so updates are 100% rock-steady without flicker */
const animOpt = {
  animation: false,
  animationDuration: 0,
  animationDurationUpdate: 0,
};

function nearestCandleIdx(candles, tMs){
  if(!candles || !candles.length) return 0;
  let best = candles.length - 1, bestDist = Infinity;
  for(let i = 0; i < candles.length; i++){
    const d = Math.abs(candles[i].t - tMs);
    if(d < bestDist){ bestDist = d; best = i; }
  }
  return best;
}

function numVal(v, fallback=0){ const n=Number(v); return Number.isFinite(n)?n:fallback; }
function signedScore(v, fallback=0){ const n=numVal(v,fallback); return Math.max(-1,Math.min(1,n)); }
function firstArray(raw, keys=[]){
  if(Array.isArray(raw)) return raw;
  if(!raw || typeof raw !== "object") return [];
  for(const k of keys){ if(Array.isArray(raw[k])) return raw[k]; }
  return [];
}
function vikeContextFromSnap(snap){
  const v = snap.vike || {};
  const cohorts = v.cohorts || v.cohort_metrics || {};
  const liq = v.liqmap || v.liquidation_heatmap || {};
  const flow = v.large_fills || v.large || {};
  const risk = v.liq_risk || v.risk || {};
  const bins = firstArray(liq, ["bins","levels"]);
  const cohortRows = firstArray(cohorts, ["cohorts","items","rows","data"]);
  const fills = firstArray(flow, ["fills","items","rows","large_fills"]);
  const longLiq = bins.reduce((a,b)=>a+numVal(b.long_usd ?? b.long_value_usd ?? b.long,0),0);
  const shortLiq = bins.reduce((a,b)=>a+numVal(b.short_usd ?? b.short_value_usd ?? b.short,0),0);
  let cohortBias = numVal(cohorts.bias ?? cohorts.net_bias ?? cohorts.directional_bias, NaN);
  if(!Number.isFinite(cohortBias)){
    const long = cohortRows.reduce((a,r)=>a+numVal(r.long_usd ?? r.long_value_usd ?? r.long,0),0);
    const sh = cohortRows.reduce((a,r)=>a+numVal(r.short_usd ?? r.short_value_usd ?? r.short,0),0);
    cohortBias = (long+sh) ? (long-sh)/(long+sh) : 0;
  }
  const fillBuy = fills.reduce((a,r)=>a+((String(r.side||r.direction||"").toLowerCase().includes("buy")||String(r.side||"").toLowerCase().includes("long"))?numVal(r.usd ?? r.notional_usd ?? r.size_usd,0):0),0);
  const fillSell = fills.reduce((a,r)=>a+((String(r.side||r.direction||"").toLowerCase().includes("sell")||String(r.side||"").toLowerCase().includes("short"))?numVal(r.usd ?? r.notional_usd ?? r.size_usd,0):0),0);
  const flowBias = (fillBuy+fillSell) ? (fillBuy-fillSell)/(fillBuy+fillSell) : signedScore((snap.lastCvd ?? snap.cvd_bias),0);
  const liqBias = (longLiq+shortLiq) ? (shortLiq-longLiq)/(longLiq+shortLiq) : 0;
  const depth = snap.book || {};
  const bid = numVal(depth.bid_usd ?? depth.bid_notional ?? depth.bid,0), ask = numVal(depth.ask_usd ?? depth.ask_notional ?? depth.ask,0);
  const wallBias = (bid+ask) ? (bid-ask)/(bid+ask) : numVal(snap.imbalance,0);
  const oi = numVal(snap.binance?.oi ?? snap.oi,0), funding = numVal(snap.binance?.funding ?? snap.funding,0);
  const crowdBias = Math.max(-1,Math.min(1, funding===0 ? 0 : -Math.sign(funding)*Math.min(1,Math.abs(funding)*10000)));
  const candles = snap.candles || [];
  const c0=candles[0], c1=candles[candles.length-1];
  const microBias = c0&&c1 ? signedScore((numVal(c1.c)-numVal(c0.o))/Math.max(1e-9,numVal(c1.o)*0.01),0) : 0;
  const score = 100*(.25*cohortBias+.20*liqBias+.20*flowBias+.15*wallBias+.10*crowdBias+.10*microBias);
  const ages = Object.values(snap.health?.feeds||{}).map(x=>numVal(x.age_s,0)).filter(x=>x>0);
  const stale = Boolean(snap.stale || (snap.health?.feeds||[]).some(x=>String(x.status||"").toUpperCase()==="STALE"));
  const state = score>=45?"BULLISH CONTEXT":score>=20?"BULLISH LEAN":score<=-45?"BEARISH CONTEXT":score<=-20?"BEARISH LEAN":"MIXED / NO EDGE";
  const read = state.startsWith("BULL")?"LONG-BIAS WATCH":state.startsWith("BEAR")?"SHORT-BIAS WATCH":"NO-EDGE WATCH";
  const confidence = Math.round(Math.max(0,Math.min(100,100*(.45*(stale?.35:1)+.25*(bins.length||cohortRows.length?1:.35)+.20*(fills.length||barsLength(snap)?1:.35)+.10*(state.includes("MIXED")?.55:1)))));
  return {score,state,read,confidence,stale,cohortBias,liqBias,flowBias,wallBias,crowdBias,microBias,bins,cohortRows,fills,liq,ages};
}
function barsLength(snap){ return (snap.bars||[]).length; }
function drawCommandTerminal(body, snap){
  const profile = snap.volume_profile || {};
  const levels = profile.levels || [];
  const order = snap.orderbook || {};
  const stats = snap.market_stats || {};
  const liq = snap.liquidation_map || {};
  const candles = snap.candles || [];
  const profileHost = $("#command-profile-chart");
  if(profileHost && window.echarts){
    const ch = ensureChart(profileHost,"command-profile");
    const rows = levels.filter(x=>Number(x.price||x.p||0)>0).slice(-36);
    const sig = `vp:${rows.length}:${profile.poc}:${rows.slice(-4).map(x=>x.vol||x.total||0).join(',')}`;
    if(ch && shouldPaint("command-profile",sig)) ch.setOption({...animOpt,grid:{left:50,right:12,top:12,bottom:24},xAxis:{type:"value",axisLabel:{color:"#82909e",fontSize:9},splitLine:{lineStyle:{color:"#222d39"}}},yAxis:{type:"category",data:rows.map(x=>Number(x.price||x.p).toFixed(2)),axisLabel:{color:"#aebbc7",fontSize:9}},series:[{type:"bar",data:rows.map(x=>({value:Number(x.vol||x.total||x.buy||0),itemStyle:{color:Number(x.buy||0)>=Number(x.sell||0)?"#238f72":"#a04a5a"}})),barWidth:"72%"}]},{notMerge:false,lazyUpdate:true});
  }
  const bookHost=$("#command-book-grid");
  if(bookHost){
    const bids=(order.bids||[]).slice(0,10), asks=(order.asks||[]).slice(0,10);
    const max=Math.max(1,...bids.map(x=>Number(x[0])*Number(x[1])),...asks.map(x=>Number(x[0])*Number(x[1])));
    const row=(x,side)=>{const px=Number(x[0]),qty=Number(x[1]),usd=px*qty;return `<div class="book-row ${side}"><span>${px.toFixed(2)}</span><span class="book-bar"><i style="width:${Math.min(100,usd/max*100)}%"></i></span><b>${qty.toFixed(3)}</b></div>`};
    bookHost.innerHTML=`<div class="book-head"><span>PRICE</span><span>DEPTH</span><span>QTY</span></div>${asks.slice().reverse().map(x=>row(x,"ask")).join("")}<div class="book-mid">MID ${Number(order.mid||snap.mark||0).toFixed(2)}</div>${bids.map(x=>row(x,"bid")).join("")}`;
  }
  const metrics=$("#command-metrics-grid");
  if(metrics){const lsr=stats.long_short||{},tak=stats.taker||{};metrics.innerHTML=[["MARK",stats.mark||snap.mark],["INDEX",stats.index],["FUNDING",stats.funding],["OPEN INT",stats.open_interest],["L/S",lsr.longShortRatio||lsr.longAccount||lsr.ratio],["TAKER",tak.buySellRatio||tak.buyVol||tak.ratio]].map(([k,v])=>`<div class="metric-cell"><small>${k}</small><b>${v==null?"—":(typeof v==='number'?Number(v).toLocaleString('en-US',{maximumFractionDigits:6}):String(v))}</b></div>`).join("");}
  const liqHost=$("#command-liq-chart");
  if(liqHost && window.echarts){const ch=ensureChart(liqHost,"command-liq");const bins=(liq.bins||[]).filter(x=>Number(x.price_bin_start||x.lo||0)>0).slice(-40);const sig=`liq:${bins.length}:${liq.mark_price}:${bins.slice(-3).map(x=>x.liquidation_value_usd||x.total||0).join(',')}`;if(ch&&shouldPaint("command-liq",sig))ch.setOption({...animOpt,grid:{left:48,right:12,top:12,bottom:24},xAxis:{type:"value",axisLabel:{color:"#82909e",fontSize:9}},yAxis:{type:"category",data:bins.map(x=>Number(x.price_bin_start||x.lo).toFixed(2)),axisLabel:{color:"#aebbc7",fontSize:8}},series:[{name:"Long liq",type:"bar",stack:"liq",data:bins.map(x=>Number(x.long_value_usd||x.long||0)),itemStyle:{color:"#b44859"}},{name:"Short liq",type:"bar",stack:"liq",data:bins.map(x=>Number(x.short_value_usd||x.short||0)),itemStyle:{color:"#278f73"}}]},{notMerge:false,lazyUpdate:true});}
}
function paintCommand(body, snap){
  const ctx = vikeContextFromSnap(snap);
  const price=numVal(snap.mark ?? snap.last ?? snap.candles?.at(-1)?.c,NaN);
  setText($("#command-price"), Number.isFinite(price)?price.toLocaleString("en-US",{maximumFractionDigits:8}):"—");
  const stateEl=$("#command-state"); setText(stateEl, ctx.state); setClass(stateEl,"chip "+(ctx.score>=20?"safe":ctx.score<=-20?"bad":"warn"));
  setText($("#command-score"),`score ${ctx.score>=0?"+":""}${ctx.score.toFixed(1)} · conf ${ctx.confidence}%`);
  setClass($("#command-score"),"mono "+(ctx.score>=20?"up":ctx.score<=-20?"dn":"yl"));
  const read = `${ctx.read}. 15m context → 5m structure → 1m tape confirmation. Execution disabled.`;
  setText($("#command-read"),read);
  const componentRows=[
    ["B · Cohort bias",ctx.cohortBias,"VIKE"],["L · Liq asymmetry",ctx.liqBias,"VIKE"],["F · Large flow",ctx.flowBias,"VIKE + BINANCE"],["O · Wall/depth",ctx.wallBias,"BINANCE"],["C · Funding/OI",ctx.crowdBias,"BINANCE"],["M · Microstructure",ctx.microBias,"BINANCE"]
  ];
  const comp=$("#command-components"); if(comp) comp.innerHTML=componentRows.map(([n,v,s])=>`<div class="command-component"><span>${n}<small>${s}</small></span><b class="${v>=.12?"up":v<=-.12?"dn":"mu"}">${v>=0?"+":""}${Number(v).toFixed(2)}</b></div>`).join("");
  const evidence=$("#command-evidence"); if(evidence) evidence.innerHTML=`<span class="chip ${ctx.stale?"bad":"safe"}">${ctx.stale?"STALE / LOWER CONFIDENCE":"FRESHNESS OK"}</span> <span class="chip info">Vike bins ${ctx.bins.length}</span> <span class="chip info">large fills ${ctx.fills.length}</span> <span class="chip">execution DISABLED</span>`;
  ["binance","vike","regime","read","paper"].forEach(n=>{ const el=body.querySelector(`[data-node="${n}"]`); if(el) el.className="topo-node "+((n==="vike"&&ctx.bins.length===0)?"warn":"ok"); });
  const k=$("#command-kpis"); if(k) k.innerHTML=`<div class="command-kpi"><small>PAIR</small><b>${snap.symbol||MICRO.sym}</b></div><div class="command-kpi"><small>MARK</small><b>${Number.isFinite(price)?price.toLocaleString("en-US",{maximumFractionDigits:8}):"—"}</b></div><div class="command-kpi"><small>STATE</small><b class="${ctx.score>=20?"up":ctx.score<=-20?"dn":"yl"}">${ctx.state}</b></div><div class="command-kpi"><small>Vike age</small><b>${ctx.ages.length?Math.max(...ctx.ages).toFixed(1)+"s":"—"}</b></div>`;
  const candle=$("#command-candle"); if(candle && window.echarts){ const cs=snap.candles||[]; const ch=ensureChart(candle,"command-candle"); const sig=`${cs.length}:${cs.at(-1)?.t}:${cs.at(-1)?.c}`; if(ch&&shouldPaint("command-candle",sig)) ch.setOption({...animOpt,grid:{left:48,right:12,top:18,bottom:28},xAxis:{type:"category",data:cs.map(c=>fmtWibAxis(c.t,snap.window||MICRO.window)),axisLabel:{color:"#82909e",fontSize:9}},yAxis:{scale:true,axisLabel:{color:"#82909e"},splitLine:{lineStyle:{color:"#222d39"}}},series:[{type:"candlestick",data:cs.map(c=>[c.o,c.c,c.l,c.h]),itemStyle:{color:"#17c784",color0:"#f04f68",borderColor:"#17c784",borderColor0:"#f04f68"}}]},{notMerge:false,lazyUpdate:true}); }
  drawCommandTerminal(body,snap);
}

function paintOrderFlow(body, snap){
  const win = snap.window || MICRO.window;
  setText($("#of-hdr"), `Footprint delta · aggressor · window ${win} · backfill=${snap.backfilled?"ready":"syncing"}`);
  const bars = snap.bars || [], cvd = snap.cvd || [];
  const lastB = bars[bars.length - 1] || {};
  const lastC = cvd[cvd.length - 1] || {};
  const sig = `${win}:${bars.length}:${lastB.t||0}:${(lastB.buy||0).toFixed(3)}:${(lastB.sell||0).toFixed(3)}:${(lastC.cvd||0).toFixed(3)}`;

  const ch1 = ensureChart($("#of-bars"), "of-bars");
  if(ch1 && shouldPaint("of-bars", sig)){
    ch1.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:54,right:18,top:24,bottom:36},
      tooltip:{trigger:"axis"},
      xAxis:{
        type:"category",
        data:bars.map(b => fmtWibAxis(b.t, win)),
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      series:[
        {name:"Buy Vol",type:"bar",stack:"v",data:bars.map(b=>b.buy),itemStyle:{color:"#0ecb81"},barMaxWidth:18},
        {name:"Sell Vol",type:"bar",stack:"v",data:bars.map(b=>-b.sell),itemStyle:{color:"#f6465d"},barMaxWidth:18},
        {
          name:"Delta",type:"line",data:bars.map(b=>b.delta),showSymbol:false,
          lineStyle:{width:2,color:"#f0b90b"},
          markPoint:{
            animation:false,
            data: bars.length ? [{coord:[bars.length-1, bars[bars.length-1].delta], symbolSize:10, itemStyle:{color:"#f0b90b"}}] : []
          }
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
  const ch2 = ensureChart($("#of-cvd"), "of-cvd");
  if(ch2 && shouldPaint("of-cvd", sig)){
    ch2.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:54,right:18,top:24,bottom:36},
      tooltip:{trigger:"axis"},
      xAxis:{
        type:"category",
        data:cvd.map(c => fmtWibAxis(c.t, win)),
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      series:[{
        name:"CVD",type:"line",data:cvd.map(c=>c.cvd),showSymbol:false,
        areaStyle:{color:"rgba(61,139,253,.18)"},
        lineStyle:{color:"#3d8bfd",width:2},
        markPoint:{
          animation:false,
          symbol:"circle",symbolSize:11,
          data: cvd.length ? [{coord:[cvd.length-1, lastC.cvd], itemStyle:{color:"#3d8bfd",borderColor:"#fff",borderWidth:2}}] : []
        }
      }]
    }, {notMerge:false, lazyUpdate:true});
    if(cvd.length){
      setText($("#cvd-head"), `CVD ${Number(lastC.cvd||0).toFixed(2)}`);
    }
  }
}

function paintWalls(body, snap){
  const series = snap.series || [];
  const candles = snap.candles || [];
  const lifelines = snap.lifelines || [];
  const win = snap.window || MICRO.window;
  const book = snap.book || {};
  const midVal = Number(book.mid || snap.mark || (candles.length ? candles[candles.length-1].c : 0)) || 0;

  setText($("#wall-hdr"), `Orderbook Wall Lifelines (Support Bid vs Resistance Ask) + Candlestick (${win} WIB)`);
  setText($("#wall-book"), `Mid ${midVal ? midVal.toFixed(2) : "—"} · via ${snap.via||"ws"}`);

  const timeline = candles.length ? candles : series.map(s => ({t:s.t, o:s.mid||midVal, h:s.mid||midVal, l:s.mid||midVal, c:s.mid||midVal}));
  const xLabels = timeline.map(c => fmtWibAxis(c.t, win));
  const ohlcData = timeline.map(c => [c.o, c.c, c.l, c.h]);

  const bidWallPts = [];
  const askWallPts = [];
  const midLine = timeline.map((c, idx) => {
    const s = series[idx] || {};
    return Number((s.mid || c.c || midVal).toFixed(2));
  });

  let maxBidWall = null, maxAskWall = null;
  series.forEach((s, sIdx) => {
    const cIdx = (series.length === timeline.length) ? sIdx : nearestCandleIdx(timeline, s.t);
    (s.walls || []).forEach(w => {
      const item = {
        value: [cIdx, Number(w.price), Number(w.usd || 0), w.side, s.t],
        itemStyle: {
          color: w.side === "bid" ? "rgba(14, 203, 129, 0.78)" : "rgba(246, 70, 93, 0.78)",
          borderColor: w.side === "bid" ? "#0ecb81" : "#f6465d",
          borderWidth: 1,
        }
      };
      if(w.side === "bid"){
        bidWallPts.push(item);
        if(!maxBidWall || w.usd > maxBidWall.usd) maxBidWall = w;
      } else {
        askWallPts.push(item);
        if(!maxAskWall || w.usd > maxAskWall.usd) maxAskWall = w;
      }
    });
  });

  // Build horizontal lifeline segments for ECharts markLine so each wall renders as a crisp horizontal support/resistance bar
  const lifelineSegments = lifelines.map(ll => {
    const isBid = ll.side === "bid";
    const col = isBid ? "rgba(14, 203, 129, 0.88)" : "rgba(246, 70, 93, 0.88)";
    const wPx = Math.max(3, Math.min(7, Math.round(Math.sqrt(Number(ll.usd || 100000)) / 180)));
    return [
      {
        coord: [ll.start_idx, Number(ll.price)],
        lineStyle: { color: col, width: wPx, type: "solid", cap: "round" },
        label: {
          show: Boolean(ll.active),
          position: "insideEndTop",
          distance: 4,
          color: isBid ? "#0ecb81" : "#f6465d",
          fontSize: 9,
          fontWeight: 700,
          formatter: `${fmtUsdCompact(ll.usd)} @ ${Number(ll.price).toFixed(1)}`
        }
      },
      {
        coord: [ll.end_idx, Number(ll.price)]
      }
    ];
  });

  if(maxBidWall) setText($("#wall-bid-top"), `Max Bid Wall ${fmtUsdCompact(maxBidWall.usd)} @ ${Number(maxBidWall.price).toFixed(2)}`);
  if(maxAskWall) setText($("#wall-ask-top"), `Max Ask Wall ${fmtUsdCompact(maxAskWall.usd)} @ ${Number(maxAskWall.price).toFixed(2)}`);

  const ladderRows = Array.isArray(snap.ladder) && snap.ladder.length
    ? [...snap.ladder].sort((a,b) => a.price - b.price)
    : [];
  const sumBidUsd = ladderRows.filter(r => r.side === "bid").reduce((a, r) => a + Number(r.usd || 0), 0);
  const sumAskUsd = ladderRows.filter(r => r.side === "ask").reduce((a, r) => a + Number(r.usd || 0), 0);
  const totBookUsd = sumBidUsd + sumAskUsd;
  const imbPct = totBookUsd > 0 ? ((sumBidUsd - sumAskUsd) / totBookUsd) * 100 : 0;
  const imbEl = $("#wall-imb");
  if(imbEl){
    setText(imbEl, `Book Imbalance ${imbPct>=0?"+":""}${imbPct.toFixed(1)}% (${imbPct>=0?"BID DOM":"ASK DOM"})`);
    setClass(imbEl, "chip mono " + (imbPct >= 5 ? "safe" : (imbPct <= -5 ? "bad" : "info")));
  }

  const allPrices = [
    ...timeline.flatMap(c => [c.l, c.h]),
    ...bidWallPts.map(p => p.value[1]),
    ...askWallPts.map(p => p.value[1]),
  ].filter(v => Number.isFinite(v) && v > 0);
  const minP = allPrices.length ? Math.min(...allPrices) : (midVal * 0.995);
  const maxP = allPrices.length ? Math.max(...allPrices) : (midVal * 1.005);
  const pad = Math.max((maxP - minP) * 0.06, midVal * 0.0005, 0.5);

  const wallSig = `${win}:${timeline.length}:${midVal.toFixed(2)}:${bidWallPts.length}:${askWallPts.length}:${lifelines.length}:${Math.round(sumBidUsd)}`;
  const ch = ensureChart($("#wall-chart"), "walls");
  if(ch && shouldPaint("walls", wallSig)){
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:68,right:22,top:30,bottom:36},
      legend:{
        data:["Harga (OHLC)","Bid Walls (Support)","Ask Walls (Resistance)","Mid Price"],
        textStyle:{color:"#c5d0db",fontSize:11},
        top:2
      },
      tooltip:{
        trigger:"item",
        formatter: p => {
          if(p.seriesType === "candlestick"){
            const c = timeline[p.dataIndex];
            if(!c) return "";
            return `<b>${wibNow(c.t)}</b><br>O: ${c.o.toFixed(2)} · H: ${c.h.toFixed(2)} · L: ${c.l.toFixed(2)} · C: <b>${c.c.toFixed(2)}</b>`;
          }
          if(p.seriesName === "Mid Price"){
            const c = timeline[p.dataIndex];
            return `${c ? wibNow(c.t) : ""}<br>Mid Price: <b>${Number(p.value).toFixed(2)}</b>`;
          }
          const v = p.value || [];
          const isBid = v[3] === "bid";
          const col = isBid ? "#0ecb81" : "#f6465d";
          return `<b style="color:${col}">${isBid ? "BID SUPPORT LIFELINE" : "ASK RESISTANCE LIFELINE"}</b><br>` +
                 `Level Harga: <b>$${Number(v[1]||0).toFixed(2)}</b><br>` +
                 `Resting Notional: <b>$${Math.round(Number(v[2]||0)).toLocaleString()}</b><br>` +
                 `Waktu: ${wibNow(v[4])}`;
        }
      },
      xAxis:{
        type:"category",
        data:xLabels,
        boundaryGap:true,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{
        type:"value",
        scale:true,
        min: Number((minP - pad).toFixed(2)),
        max: Number((maxP + pad).toFixed(2)),
        axisLabel:{color:"#8b98a8",formatter:v=>Number(v).toFixed(1)},
        splitLine:{lineStyle:{color:"#1e2530"}}
      },
      series:[
        {
          name:"Harga (OHLC)",
          type:"candlestick",
          z:4,
          data:ohlcData,
          itemStyle:{
            color:"#0ecb81",
            color0:"#f6465d",
            borderColor:"#0ecb81",
            borderColor0:"#f6465d"
          },
          markLine:{
            silent:true,
            animation:false,
            symbol:["none","none"],
            z:2,
            data: lifelineSegments
          }
        },
        {
          name:"Bid Walls (Support)",
          type:"scatter",
          color:"#0ecb81",
          z:3,
          symbol:"roundRect",
          symbolSize:[14, 5],
          data: bidWallPts
        },
        {
          name:"Ask Walls (Resistance)",
          type:"scatter",
          color:"#f6465d",
          z:3,
          symbol:"roundRect",
          symbolSize:[14, 5],
          data: askWallPts
        },
        {
          name:"Mid Price",
          type:"line",
          color:"#f0b90b",
          z:6,
          showSymbol:false,
          lineStyle:{color:"#f0b90b",width:1.6,type:"dashed"},
          data: midLine
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }

  // Right panel: 20-Level Orderbook Depth & Wall Ladder (10 Bid Support levels below MID, 10 Ask Resistance levels above MID)
  const bookCh = ensureChart($("#wall-book-chart"), "wall-book-chart");
  if(bookCh && ladderRows.length && shouldPaint("wall-book-chart", wallSig)){
    const yLabels = ladderRows.map(r => Number(r.price).toFixed(2));
    let midIdx = Math.floor(yLabels.length / 2) - 1;
    let bestD = Infinity;
    ladderRows.forEach((r, i) => {
      const d = Math.abs(r.price - midVal);
      if(d < bestD){ bestD = d; midIdx = i; }
    });

    bookCh.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:66,right:18,top:32,bottom:36},
      legend:{
        data:["Bid Depth","Ask Depth"],
        textStyle:{color:"#c5d0db",fontSize:11},
        top:2
      },
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"shadow"},
        formatter: params => {
          const idx = params[0]?.dataIndex ?? 0;
          const r = ladderRows[idx];
          if(!r) return "";
          const col = r.side === "bid" ? "#0ecb81" : "#f6465d";
          return `<b style="color:${col}">${r.side === "bid" ? "BID SUPPORT LEVEL" : "ASK RESISTANCE LEVEL"} @ $${Number(r.price).toFixed(2)}</b><br>` +
                 `Orderbook Depth: <b>$${Math.round(r.usd).toLocaleString()}</b>`;
        }
      },
      xAxis:{
        type:"value",
        axisLabel:{color:"#8b98a8",fontSize:10,formatter:v=>fmtUsdCompact(v)},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      yAxis:{
        type:"category",
        data:yLabels,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      series:[
        {
          name:"Bid Depth",
          type:"bar",
          stack:"ob",
          color:"#0ecb81",
          data: ladderRows.map(r => r.side === "bid" ? Math.round(r.usd) : 0),
          itemStyle:{color:"rgba(14,203,129,0.82)"}
        },
        {
          name:"Ask Depth",
          type:"bar",
          stack:"ob",
          color:"#f6465d",
          data: ladderRows.map(r => r.side === "ask" ? Math.round(r.usd) : 0),
          itemStyle:{color:"rgba(246,70,93,0.82)"},
          markLine:{
            silent:true,
            animation:false,
            symbol:"none",
            data: yLabels[midIdx] ? [{
              yAxis: yLabels[midIdx],
              label:{formatter:`MID ${midVal.toFixed(2)}`,color:"#f0b90b",position:"insideEndTop",fontSize:10,fontWeight:700},
              lineStyle:{type:"dashed",color:"#f0b90b",width:1.8}
            }] : []
          }
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function heatBandColor(ratio){
  if(ratio >= 0.80) return "rgba(246, 70, 93, 0.42)";
  if(ratio >= 0.50) return "rgba(245, 158, 11, 0.34)";
  if(ratio >= 0.22) return "rgba(6, 182, 212, 0.24)";
  return "rgba(59, 130, 246, 0.14)";
}

function paintLiqMap(body, snap){
  const bins = snap.bins || [];
  const candles = snap.candles || [];
  const win = snap.window || MICRO.window;
  const mark = Number(snap.mark || (candles.length ? candles[candles.length-1].c : 0)) || 0;

  const stChip = $("#liq-status");
  if(stChip){
    setText(stChip, snap.vike_ok ? "VIKE: OK" : (snap.vike_message || "VIKE + BINANCE"));
    setClass(stChip, "chip " + (snap.vike_ok ? "safe" : "warn"));
  }
  setText($("#liq-mark"), `mark=${mark ? mark.toFixed(2) : "—"} · window=${win} · bins=${bins.length} · candles=${candles.length}`);

  let cLo = candles.length ? Math.min(...candles.map(c=>c.l)) : (mark * 0.98);
  let cHi = candles.length ? Math.max(...candles.map(c=>c.h)) : (mark * 1.02);
  const span = Math.max(cHi - cLo, mark * 0.015, 1);
  const viewLo = Math.min(cLo, mark || cLo) - span * 0.65;
  const viewHi = Math.max(cHi, mark || cHi) + span * 0.65;

  let nearBins = bins.filter(b => {
    const m = b.mid || ((b.lo + b.hi) / 2);
    return m >= viewLo && m <= viewHi;
  });
  if(nearBins.length < 8 && bins.length){
    nearBins = bins.filter(b => {
      const m = b.mid || ((b.lo + b.hi) / 2);
      return !mark || Math.abs(m - mark) / mark < 0.08;
    });
  }
  nearBins.sort((a,b) => (a.mid||0) - (b.mid||0));
  const maxBinUsd = Math.max(1, ...nearBins.map(b => b.total || (b.long + b.short) || 0));

  const heatAreas = nearBins
    .filter(b => (b.total || (b.long + b.short) || 0) > maxBinUsd * 0.04)
    .map(b => {
      const tot = b.total || (b.long + b.short) || 0;
      const ratio = Math.min(1, tot / maxBinUsd);
      const halfStep = Math.max((b.hi - b.lo) / 2, mark * 0.0006);
      const mid = b.mid || ((b.lo + b.hi) / 2);
      return [
        {yAxis: mid - halfStep, itemStyle:{color: heatBandColor(ratio)}},
        {yAxis: mid + halfStep}
      ];
    });

  const rows = (snap.realized || []).slice(-120);
  const liqScatter = rows.map(r => {
    const tMs = typeof r.t === "number" ? r.t : (Date.parse(r.t) || Date.now());
    const idx = nearestCandleIdx(candles, tMs);
    const side = String(r.side || "").toLowerCase();
    const isLongLiq = side.includes("long") || side.includes("sell");
    return {
      value: [idx, r.price, r.usd || 0, isLongLiq ? "LONG LIQ" : "SHORT LIQ", tMs],
      itemStyle: {
        color: isLongLiq ? "#f6465d" : "#0ecb81",
        borderColor: "#fff",
        borderWidth: 1,
      }
    };
  }).filter(d => d.value[1] >= viewLo * 0.97 && d.value[1] <= viewHi * 1.03);

  const xLabels = candles.map(c => fmtWibAxis(c.t, win));
  const ohlcData = candles.map(c => [c.o, c.c, c.l, c.h]);
  const lastC = candles[candles.length - 1] || {};
  const liqSig = `${win}:${candles.length}:${lastC.t||0}:${(lastC.c||0).toFixed(2)}:${mark.toFixed(2)}:${nearBins.length}:${liqScatter.length}`;

  const mainChart = ensureChart($("#liq-main"), "liq-main");
  if(mainChart && shouldPaint("liq-main", liqSig)){
    mainChart.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:68,right:22,top:24,bottom:36},
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"cross"},
        formatter: params => {
          if(!Array.isArray(params)) params = [params];
          const p0 = params[0];
          const idx = p0 ? p0.dataIndex : 0;
          const c = candles[idx];
          let html = c ? `<b>${wibNow(c.t)}</b><br>O: ${c.o.toFixed(2)} · H: ${c.h.toFixed(2)} · L: ${c.l.toFixed(2)} · C: <b>${c.c.toFixed(2)}</b>` : "";
          if(c){
            const hitBin = nearBins.find(b => c.c >= b.lo && c.c <= b.hi);
            if(hitBin){
              html += `<br><span style="color:#f0b90b">Liq Cluster @ ${hitBin.lo.toFixed(1)}–${hitBin.hi.toFixed(1)}: $${((hitBin.total||0)/1e6).toFixed(2)}M</span>`;
            }
          }
          return html;
        }
      },
      xAxis:{
        type:"category",
        data:xLabels,
        boundaryGap:true,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{
        type:"value",
        scale:true,
        min: v => Math.min(v.min, viewLo),
        max: v => Math.max(v.max, viewHi),
        axisLabel:{color:"#8b98a8",formatter:v=>Number(v).toFixed(1)},
        splitLine:{lineStyle:{color:"#1e2530"}}
      },
      series:[
        {
          name:"Harga (OHLC)",
          type:"candlestick",
          z:5,
          data:ohlcData,
          itemStyle:{
            color:"#0ecb81",
            color0:"#f6465d",
            borderColor:"#0ecb81",
            borderColor0:"#f6465d"
          },
          markArea:{
            silent:true,
            animation:false,
            z:1,
            data:heatAreas
          },
          markLine:{
            silent:true,
            animation:false,
            symbol:"none",
            z:7,
            data: mark ? [{
              yAxis: mark,
              label:{formatter:`MARK ${mark.toFixed(2)}`,color:"#f0b90b",position:"insideEndTop",fontSize:10},
              lineStyle:{type:"dashed",color:"#f0b90b",width:1.8}
            }] : []
          }
        },
        {
          name:"Realized Liq",
          type:"scatter",
          z:8,
          symbolSize: v => Math.max(6, Math.min(24, Math.sqrt(v[2]||1)/30)),
          data: liqScatter
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }

  const ladder = ensureChart($("#liq-ladder"), "liq-ladder");
  if(ladder && nearBins.length && shouldPaint("liq-ladder", liqSig)){
    const yLabels = nearBins.map(b => (b.mid || ((b.lo+b.hi)/2)).toFixed(2));
    let closest = 0, best = Infinity;
    nearBins.forEach((b, i) => {
      const d = Math.abs((b.mid || ((b.lo+b.hi)/2)) - mark);
      if(d < best){ best = d; closest = i; }
    });
    ladder.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:72,right:22,top:24,bottom:36},
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"shadow"},
        formatter: params => {
          const idx = params[0]?.dataIndex ?? 0;
          const b = nearBins[idx];
          if(!b) return "";
          return `<b>Level $${b.lo.toFixed(2)} – $${b.hi.toFixed(2)}</b><br>` +
                 `<span style="color:#0ecb81">Long Liq: $${((b.long||0)/1e6).toFixed(2)}M</span><br>` +
                 `<span style="color:#f6465d">Short Liq: $${((b.short||0)/1e6).toFixed(2)}M</span><br>` +
                 `Total: <b>$${((b.total||0)/1e6).toFixed(2)}M</b>`;
        }
      },
      xAxis:{
        type:"value",
        axisLabel:{color:"#8b98a8",fontSize:10,formatter:v=>"$"+(Math.abs(v)/1e6).toFixed(1)+"M"},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      yAxis:{
        type:"category",
        data:yLabels,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      series:[
        {name:"Long Liq",type:"bar",stack:"l",data:nearBins.map(b=>-(b.long||0)),itemStyle:{color:"#0ecb81"}},
        {
          name:"Short Liq",type:"bar",stack:"l",data:nearBins.map(b=>b.short||0),itemStyle:{color:"#f6465d"},
          markLine:{
            silent:true, animation:false, symbol:"none",
            label:{formatter:`MARK ${mark.toFixed(2)}`,color:"#f0b90b",position:"insideEndTop",fontSize:10},
            lineStyle:{type:"dashed",color:"#f0b90b",width:1.8},
            data: yLabels[closest] ? [{yAxis:yLabels[closest]}] : []
          }
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintLargeVP(body, snap){
  const candles = snap.candles || [];
  const large = snap.large || [];
  const profile = snap.profile || [];
  const win = snap.window || MICRO.window;

  setText($("#lt-hdr"), `Candlestick (${win} WIB) + Large Trades Bubbles (${large.length} block trades) + Volume Profile`);
  setText($("#lt-poc-chip"), `POC ${snap.poc!=null ? Number(snap.poc).toFixed(2) : "—"}`);
  setText($("#lt-vah-chip"), `VAH ${snap.vah!=null ? Number(snap.vah).toFixed(2) : "—"}`);
  setText($("#lt-val-chip"), `VAL ${snap.val!=null ? Number(snap.val).toFixed(2) : "—"}`);

  large.slice(-6).forEach(pushTape);

  const xLabels = candles.map(c => fmtWibAxis(c.t, win));
  const ohlcData = candles.map(c => [c.o, c.c, c.l, c.h]);

  // Map large trade bubbles onto their corresponding candle index across the entire window
  const buyBubbles = [];
  const sellBubbles = [];
  large.forEach(t => {
    const idx = (typeof t.candle_idx === "number" && t.candle_idx >= 0 && t.candle_idx < candles.length)
      ? t.candle_idx
      : nearestCandleIdx(candles, t.t);
    const pt = {
      value: [idx, Number(t.p), Number(t.usd || 0), t.side, Number(t.q || 0), t.t, t.tier || "large"],
      itemStyle: {
        color: t.side === "buy" ? "rgba(14, 203, 129, 0.80)" : "rgba(246, 70, 93, 0.80)",
        borderColor: t.side === "buy" ? "#0ecb81" : "#f6465d",
        borderWidth: 1.5,
      }
    };
    if(t.side === "buy") buyBubbles.push(pt);
    else sellBubbles.push(pt);
  });

  // Inside-end labels ensure POC / VAH / VAL are never clipped off the right canvas edge
  const vpLines = [];
  if(snap.poc != null){
    vpLines.push({
      yAxis:Number(snap.poc),
      label:{formatter:`POC ${Number(snap.poc).toFixed(2)}`,color:"#f0b90b",position:"insideEndTop",distance:6,fontSize:10,fontWeight:700},
      lineStyle:{color:"#f0b90b",width:1.8,type:"dashed"}
    });
  }
  if(snap.vah != null){
    vpLines.push({
      yAxis:Number(snap.vah),
      label:{formatter:`VAH ${Number(snap.vah).toFixed(2)}`,color:"#3d8bfd",position:"insideEndTop",distance:6,fontSize:10},
      lineStyle:{color:"#3d8bfd",width:1.4,type:"dotted"}
    });
  }
  if(snap.val != null){
    vpLines.push({
      yAxis:Number(snap.val),
      label:{formatter:`VAL ${Number(snap.val).toFixed(2)}`,color:"#3d8bfd",position:"insideEndBottom",distance:6,fontSize:10},
      lineStyle:{color:"#3d8bfd",width:1.4,type:"dotted"}
    });
  }

  // Exact shared price bounds so Left Candlestick Y-axis and Right VP Y-axis align horizontally
  const allPrices = [
    ...candles.flatMap(c => [c.l, c.h]),
    ...profile.map(p => p.price),
  ].filter(v => Number.isFinite(v) && v > 0);
  const minP = allPrices.length ? Math.min(...allPrices) : null;
  const maxP = allPrices.length ? Math.max(...allPrices) : null;
  const pad = (minP != null && maxP != null) ? Math.max((maxP - minP) * 0.04, 0.3) : 0;
  const yMin = minP != null ? Number((minP - pad).toFixed(2)) : undefined;
  const yMax = maxP != null ? Number((maxP + pad).toFixed(2)) : undefined;

  const lastC = candles[candles.length - 1] || {};
  const ltSig = `${win}:${candles.length}:${lastC.t||0}:${(lastC.c||0).toFixed(2)}:${large.length}:${snap.poc||0}:${snap.vah||0}:${snap.val||0}`;

  const ch = ensureChart($("#lt-bubbles"), "lt-bub");
  if(ch && shouldPaint("lt-bub", ltSig)){
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:68,right:18,top:24,bottom:36},
      tooltip:{
        trigger:"item",
        formatter: p => {
          if(p.seriesType === "candlestick"){
            const c = candles[p.dataIndex];
            if(!c) return "";
            return `<b>${wibNow(c.t)}</b><br>Open: ${c.o.toFixed(2)}<br>High: ${c.h.toFixed(2)}<br>Low: ${c.l.toFixed(2)}<br>Close: <b>${c.c.toFixed(2)}</b>`;
          }
          const v = p.value || [];
          const col = v[3] === "buy" ? "#0ecb81" : "#f6465d";
          return `<b style="color:${col}">${String(v[3]||"").toUpperCase()} AGGRESSOR (${String(v[6]||"large").toUpperCase()})</b><br>` +
                 `Notional: <b>$${Math.round(Number(v[2]||0)).toLocaleString()}</b><br>` +
                 `Harga: <b>$${Number(v[1]||0).toFixed(2)}</b> · Qty: ${Number(v[4]||0).toFixed(4)}<br>` +
                 `Waktu: ${wibNow(v[5])}`;
        }
      },
      xAxis:{
        type:"category",
        data:xLabels,
        boundaryGap:true,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{
        type:"value",
        scale:true,
        min: yMin,
        max: yMax,
        axisLabel:{color:"#8b98a8",formatter:v=>Number(v).toFixed(1)},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      series:[
        {
          name:"Price",
          type:"candlestick",
          z:2,
          data:ohlcData,
          itemStyle:{
            color:"#0ecb81",
            color0:"#f6465d",
            borderColor:"#0ecb81",
            borderColor0:"#f6465d"
          },
          markLine:{
            silent:true,
            animation:false,
            symbol:"none",
            z:4,
            data:vpLines
          }
        },
        {
          name:"Buy Large Trades",
          type:"scatter",
          color:"#0ecb81",
          z:6,
          symbolSize: v => Math.max(9, Math.min(30, Math.sqrt(v[2]||1)/15)),
          data: buyBubbles
        },
        {
          name:"Sell Large Trades",
          type:"scatter",
          color:"#f6465d",
          z:6,
          symbolSize: v => Math.max(9, Math.min(30, Math.sqrt(v[2]||1)/15)),
          data: sellBubbles
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }

  const vp = ensureChart($("#lt-vp"), "lt-vp");
  if(vp && profile.length && shouldPaint("lt-vp", ltSig)){
    const yLabels = profile.map(p => Number(p.price).toFixed(2));
    const nearestLabel = target => {
      if(target == null || !profile.length) return null;
      let best = 0, bestD = Infinity;
      profile.forEach((p, i) => {
        const d = Math.abs(p.price - target);
        if(d < bestD){ bestD = d; best = i; }
      });
      return yLabels[best];
    };
    const pocLbl = nearestLabel(snap.poc);
    const vahLbl = nearestLabel(snap.vah);
    const valLbl = nearestLabel(snap.val);
    vp.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:62,right:18,top:24,bottom:36},
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"shadow"},
        formatter: params => {
          const idx = params[0]?.dataIndex ?? 0;
          const row = profile[idx];
          if(!row) return "";
          return `<b>Level Harga ~$${Number(row.price).toFixed(2)}</b><br>` +
                 `<span style="color:#0ecb81">Buy Vol: ${Number(row.buy||0).toFixed(3)}</span><br>` +
                 `<span style="color:#f6465d">Sell Vol: ${Number(row.sell||0).toFixed(3)}</span><br>` +
                 `Total Vol: <b>${Number(row.vol||0).toFixed(3)}</b>`;
        }
      },
      xAxis:{
        type:"value",
        axisLabel:{color:"#8b98a8",fontSize:10},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      yAxis:{
        type:"category",
        data:yLabels,
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      series:[
        {
          name:"Buy Vol", type:"bar", stack:"vp",
          data: profile.map((p, i) => ({
            value: Number(p.buy || 0),
            itemStyle:{color: yLabels[i]===pocLbl ? "#f0b90b" : "rgba(14,203,129,0.78)"}
          }))
        },
        {
          name:"Sell Vol", type:"bar", stack:"vp",
          data: profile.map((p, i) => ({
            value: Number(p.sell || 0),
            itemStyle:{color: yLabels[i]===pocLbl ? "#d99b00" : "rgba(246,70,93,0.78)"}
          })),
          markLine:{
            silent:true, animation:false, symbol:"none",
            data:[
              ...(pocLbl ? [{yAxis:pocLbl, label:{formatter:`POC ${pocLbl}`,color:"#f0b90b",position:"insideEndTop",distance:4,fontSize:10,fontWeight:700}, lineStyle:{type:"dashed",color:"#f0b90b",width:1.8}}] : []),
              ...(vahLbl ? [{yAxis:vahLbl, label:{formatter:`VAH ${vahLbl}`,color:"#3d8bfd",position:"insideEndTop",distance:4,fontSize:10}, lineStyle:{type:"dotted",color:"#3d8bfd",width:1.4}}] : []),
              ...(valLbl ? [{yAxis:valLbl, label:{formatter:`VAL ${valLbl}`,color:"#3d8bfd",position:"insideEndBottom",distance:4,fontSize:10}, lineStyle:{type:"dotted",color:"#3d8bfd",width:1.4}}] : []),
            ]
          }
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintTier(body, snap){
  const bars = snap.bars || [];
  const win = snap.window || MICRO.window;
  const tiers = snap.tiers || ["retail","mid","large","whale","mega"];
  const tierLabels = {retail:"Retail (<$1K)", mid:"Mid ($1K–10K)", large:"Large ($10K–50K)", whale:"Whale ($50K–250K)", mega:"Mega (≥$250K)"};
  const colors = {retail:"#8b98a8",mid:"#3d8bfd",large:"#f0b90b",whale:"#0ecb81",mega:"#f6465d"};
  setText($("#tier-hdr"), `Order Flow Berdasarkan Ukuran Trade (Tier Breakdown · Window ${win} WIB · ${bars.length} bar)`);

  const share = snap.share || {};
  const shareUsd = snap.share_usd || {};
  const bMap = snap.tier_breakdown_map || {};
  if(Array.isArray(snap.tier_breakdown)){
    snap.tier_breakdown.forEach(r => { if(r && r.tier) bMap[r.tier] = r; });
  }
  const lastB = bars[bars.length - 1] || {};
  const tierSig = `${win}:${bars.length}:${lastB.t||0}:${tiers.map(t=>Math.round((bMap[t]&&bMap[t].total_usd)||shareUsd[t]||(share[t]||0)*10000)).join(",")}`;

  const ch = ensureChart($("#tier-bars"), "tier-bars");
  if(ch && shouldPaint("tier-bars", tierSig)){
    const netLine = bars.map(b => tiers.reduce((acc, t) => acc + Number(b[t] || 0), 0));
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      legend:{
        data:[...tiers.map(t => tierLabels[t] || t), "Total Net Flow"],
        textStyle:{color:"#c5d0db",fontSize:10},
        top:0
      },
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"shadow"},
        formatter: params => {
          if(!Array.isArray(params) || !params.length) return "";
          const idx = params[0].dataIndex ?? 0;
          const b = bars[idx];
          let s = `<b>${b ? wibNow(b.t) : params[0].axisValue}</b>`;
          params.forEach(p => {
            s += `<br><span style="color:${p.color}">● ${p.seriesName}: <b>${fmtUsdCompact(p.value)}</b></span>`;
          });
          return s;
        }
      },
      grid:{left:64,right:18,top:34,bottom:36},
      xAxis:{
        type:"category",
        data:bars.map(b => fmtWibAxis(b.t, win)),
        axisLabel:{color:"#8b98a8",fontSize:10}
      },
      yAxis:{
        type:"value",
        axisLabel:{color:"#8b98a8",fontSize:10,formatter:v=>fmtUsdCompact(v)},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      series:[
        ...tiers.map(t => ({
          name: tierLabels[t] || t,
          type:"bar",
          stack:"t",
          barMaxWidth:20,
          data:bars.map(b => Math.round(Number(b[t] || 0))),
          itemStyle:{color:colors[t]||"#fff"}
        })),
        {
          name:"Total Net Flow",
          type:"line",
          color:"#ffffff",
          showSymbol:false,
          lineStyle:{color:"#ffffff",width:1.6,type:"dashed"},
          data:netLine.map(v => Math.round(v))
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }

  // Right Top: Donut Chart of Volume Share per Tier (animation:false avoids ECharts 5 pie sector sweep clipping bug!)
  const pie = ensureChart($("#tier-pie"), "tier-pie");
  if(pie && shouldPaint("tier-pie", tierSig)){
    const pieData = tiers.map(t => {
      const row = bMap[t] || {};
      const usdVal = Number(row.total_usd ?? shareUsd[t] ?? ((Number(row.buy_usd||0) + Number(row.sell_usd||0)) || (Number(share[t]||0) * 1e6)));
      return {
        name: (t.charAt(0).toUpperCase() + t.slice(1)),
        value: Math.max(1, Math.round(usdVal)),
        itemStyle:{color:colors[t]}
      };
    });
    const totUsd = pieData.reduce((a, d) => a + d.value, 0) || 1;
    pie.setOption({
      ...animOpt, backgroundColor:"transparent",
      tooltip:{
        trigger:"item",
        formatter: p => `<b>${p.name} Tier</b><br>Total Volume: <b>${fmtUsdCompact(p.value)}</b> (${p.percent}%)`
      },
      legend:{
        orient:"vertical",
        right:6,
        top:"middle",
        itemWidth:12,
        itemHeight:10,
        textStyle:{color:"#c5d0db",fontSize:10.5,fontFamily:"IBM Plex Mono"},
        formatter: name => {
          const item = pieData.find(d => d.name === name);
          if(!item) return name;
          const pct = ((item.value / totUsd) * 100).toFixed(1);
          return `${name.padEnd(6," ")} ${fmtUsdCompact(item.value)} (${pct}%)`;
        }
      },
      series:[{
        type:"pie",
        animation:false,
        center:["25%","52%"],
        radius:["34%","64%"],
        avoidLabelOverlap:true,
        itemStyle:{borderColor:"#0b0e11",borderWidth:2},
        label:{
          show:true,
          position:"inside",
          color:"#0b0e11",
          fontWeight:700,
          fontSize:10,
          formatter: p => p.percent >= 7 ? `${p.percent.toFixed(0)}%` : ""
        },
        labelLine:{show:false},
        data: pieData
      }]
    }, {notMerge:false, lazyUpdate:true});
  }

  // Right Bottom: Buy vs Sell USD per Tier (symmetric horizontal bar chart)
  const netCh = ensureChart($("#tier-net"), "tier-net");
  if(netCh && shouldPaint("tier-net", tierSig)){
    const yNames = tiers.map(t => t.charAt(0).toUpperCase() + t.slice(1));
    const buyArr = tiers.map(t => {
      const row = bMap[t];
      if(row && row.buy_usd != null) return Math.round(Number(row.buy_usd));
      const posSum = bars.reduce((a, b) => a + Math.max(0, Number(b[t] || 0)), 0);
      return Math.round(posSum);
    });
    const sellArr = tiers.map(t => {
      const row = bMap[t];
      if(row && row.sell_usd != null) return Math.round(Number(row.sell_usd));
      const negSum = bars.reduce((a, b) => a + Math.max(0, -Number(b[t] || 0)), 0);
      return Math.round(negSum);
    });

    netCh.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:58,right:18,top:26,bottom:28},
      legend:{
        data:["Buy Vol (Taker Buy)","Sell Vol (Taker Sell)"],
        textStyle:{color:"#c5d0db",fontSize:10},
        top:0
      },
      tooltip:{
        trigger:"axis",
        axisPointer:{type:"shadow"},
        formatter: params => {
          const idx = params[0]?.dataIndex ?? 0;
          const tName = yNames[idx] || "";
          const bV = buyArr[idx] || 0;
          const sV = sellArr[idx] || 0;
          const nV = bV - sV;
          return `<b>${tName} Tier</b><br>` +
                 `<span style="color:#0ecb81">Buy Vol: ${fmtUsdCompact(bV)}</span><br>` +
                 `<span style="color:#f6465d">Sell Vol: ${fmtUsdCompact(sV)}</span><br>` +
                 `Net Delta: <b style="color:${nV>=0?'#0ecb81':'#f6465d'}">${nV>=0?'+':''}${fmtUsdCompact(nV)}</b>`;
        }
      },
      xAxis:{
        type:"value",
        axisLabel:{color:"#8b98a8",fontSize:9,formatter:v=>fmtUsdCompact(Math.abs(v))},
        splitLine:{lineStyle:{color:"#232a35"}}
      },
      yAxis:{
        type:"category",
        data:yNames,
        axisLabel:{color:"#c5d0db",fontSize:10}
      },
      series:[
        {
          name:"Buy Vol (Taker Buy)",
          type:"bar",
          stack:"bs",
          color:"#0ecb81",
          barMaxWidth:18,
          data:buyArr,
          itemStyle:{color:"rgba(14,203,129,0.85)"}
        },
        {
          name:"Sell Vol (Taker Sell)",
          type:"bar",
          stack:"bs",
          color:"#f6465d",
          barMaxWidth:18,
          data:sellArr.map(v => -v),
          itemStyle:{color:"rgba(246,70,93,0.85)"}
        }
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintCohorts(body, snap){
  const cm = snap.cohort_metrics || {}, sb = snap.size_breakdown || {};
  const ok = (cm && cm.ok) || (sb && sb.ok);
  const st = $("#coh-status");
  if(st){
    setText(st, ok ? "VIKE cohorts: OK" : "VIKE cohorts (cached/sync)");
    setClass(st, "chip " + (ok ? "safe" : "warn"));
  }
  const metrics = ((cm.data||cm).metrics) || [];
  const latest = {};
  metrics.forEach(m => {
    const c = m.cohort || "?";
    if(!latest[c] || String(m.ts) > String(latest[c].ts)) latest[c] = m;
  });
  const rows = Object.values(latest);
  const cohSig = `coh:${rows.length}:${rows.map(r=>Math.round(Number(r.total_position_value||0))).join(",")}`;
  const ch = ensureChart($("#coh-metrics"), "coh-m");
  if(ch && rows.length && shouldPaint("coh-m", cohSig)){
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:100,right:20,top:20,bottom:40},
      tooltip:{trigger:"axis"},
      xAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      yAxis:{type:"category",data:rows.map(r=>r.cohort),axisLabel:{color:"#c5d0db"}},
      series:[{type:"bar",data:rows.map(r=>Number(r.total_position_value||r.position_count||0)),itemStyle:{color:"#3d8bfd"}}]
    }, {notMerge:false, lazyUpdate:true});
  }
  const size = Array.isArray(sb.data) ? sb.data : (Array.isArray(sb) ? sb : (sb.data||[]));
  const ch2 = ensureChart($("#coh-size"), "coh-s");
  if(ch2 && size.length && shouldPaint("coh-s", cohSig + ":" + size.length)){
    ch2.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:80,right:20,top:20,bottom:40},
      tooltip:{trigger:"axis"},
      xAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      yAxis:{type:"category",data:size.map(r=>r.tier||r.cohort||"?"),axisLabel:{color:"#c5d0db"}},
      series:[{type:"bar",data:size.map(r=>Number(r.total_position_value||0)),itemStyle:{color:"#f0b90b"}}]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintFunding(body, snap){
  const fund = snap.funding || {};
  const items = ((fund.data||fund).items) || [];
  const coin = MICRO.sym.replace("USDT","");
  const row = items.find(i => String(i.symbol||"").toUpperCase()===coin) || items[0];
  setText($("#fund-lbl"), `Funding ${coin}`);
  setText($("#fund-v"), row ? Number(row.funding_rate||row.funding||0).toFixed(6) : "—");
  setText($("#fund-n"), String(items.length));

  const top = [...items].sort((a,b) => Math.abs(Number(b.funding_rate||b.funding||0)) - Math.abs(Number(a.funding_rate||a.funding||0))).slice(0,30);
  const fundSig = `fund:${coin}:${items.length}:${top.slice(0,5).map(i=>Number(i.funding_rate||i.funding||0).toFixed(6)).join(",")}`;
  const ch = ensureChart($("#fund-chart"), "fund");
  if(ch && top.length && shouldPaint("fund", fundSig)){
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:60,right:20,top:20,bottom:80},
      tooltip:{trigger:"axis"},
      xAxis:{type:"category",data:top.map(i=>i.symbol),axisLabel:{color:"#8b98a8",rotate:45,fontSize:10}},
      yAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      series:[{
        type:"bar",
        data:top.map(i => {
          const v = Number(i.funding_rate||i.funding||0);
          return {value:v, itemStyle:{color: v>=0 ? "#0ecb81" : "#f6465d"}};
        })
      }]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintLiqRisk(body, snap){
  const risk = snap.risk || {};
  const rows = Array.isArray(risk.data) ? risk.data : (Array.isArray(risk) ? risk : []);
  const clusters = snap.clusters || {};
  const top = rows.slice(0,25);
  const riskSig = `risk:${top.length}:${top.slice(0,5).map(r=>Number(r.percent_risk||0).toFixed(4)).join(",")}`;
  const ch = ensureChart($("#risk-chart"), "risk");
  if(ch && top.length && shouldPaint("risk", riskSig)){
    ch.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:70,right:20,top:20,bottom:40},
      tooltip:{trigger:"axis"},
      xAxis:{type:"value",axisLabel:{color:"#8b98a8",formatter:v=>(v*100).toFixed(1)+"%"},splitLine:{lineStyle:{color:"#232a35"}}},
      yAxis:{type:"category",data:top.map(r=>r.symbol).reverse(),axisLabel:{color:"#c5d0db"}},
      series:[{type:"bar",data:top.map(r=>Number(r.percent_risk||0)).reverse(),itemStyle:{color:"#f6465d"}}]
    }, {notMerge:false, lazyUpdate:true});
  }
  const raw = (clusters.data||clusters) || {};
  const buckets = raw.buckets || {};
  const bids = (buckets.bid||[]).slice(0,30), asks = (buckets.ask||[]).slice(0,30);
  const ch2 = ensureChart($("#cluster-chart"), "clusters");
  if(ch2 && (bids.length || asks.length) && shouldPaint("clusters", `${riskSig}:${bids.length}:${asks.length}`)){
    ch2.setOption({
      ...animOpt, backgroundColor:"transparent",
      grid:{left:70,right:20,top:20,bottom:40},
      tooltip:{trigger:"item"},
      xAxis:{type:"value",axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      yAxis:{type:"value",scale:true,axisLabel:{color:"#8b98a8"},splitLine:{lineStyle:{color:"#232a35"}}},
      series:[
        {name:"Bid Clusters",type:"scatter",symbolSize:10,data:bids.map(b=>[Number(b.cluster_notional||0),Number(b.price_low||0)]),itemStyle:{color:"#0ecb81"}},
        {name:"Ask Clusters",type:"scatter",symbolSize:10,data:asks.map(a=>[Number(a.cluster_notional||0),Number(a.price_low||0)]),itemStyle:{color:"#f6465d"}}
      ]
    }, {notMerge:false, lazyUpdate:true});
  }
}

function paintMarketData(HEAT){
  const body = $("#micro-body"); if(!body || !HEAT) return;
  ensureTabDOM(body, "market");
  const items = (HEAT && HEAT.items) || [];
  const err = $("#hm-market-err");
  if(err){
    if(HEAT.error || HEAT.message){
      setText(err, (HEAT.message || HEAT.error) + (HEAT.stale ? " (stale last-good)" : ""));
      err.style.color = HEAT.stale ? "#f0b90b" : "#f6465d";
    } else {
      setText(err, `${items.length} pairs aktif`);
      err.style.color = "#8b98a8";
    }
  }
  const host = $("#hm-market");
  if(!host || !items.length) return;
  const ch = ensureChart(host, "mkt");
  if(!ch) return;
  const mktSig = items.slice(0, 15).map(i => `${i.symbol}:${Number(i.pct||0).toFixed(2)}`).join("|");
  if(!shouldPaint("mkt", mktSig)){
    setMeta("BINANCE", HEAT.ts || Date.now(), {});
    return;
  }
  const data = items.slice(0,60).map(i => {
    const name = i.symbol.replace("USDT","");
    const pct = Number(i.pct);
    return {
      name,
      value: Math.max(1, Number(i.quoteVolume)||0),
      pct,
      itemStyle:{
        color: pct >= 0
          ? `rgba(14,203,129,${Math.min(.92,.35+Math.abs(pct)/16)})`
          : `rgba(246,70,93,${Math.min(.92,.35+Math.abs(pct)/16)})`,
        borderColor: "#0b0e11",
        borderWidth: 2,
      }
    };
  });
  ch.setOption({
    ...animOpt, backgroundColor:"transparent",
    tooltip:{
      formatter: p => {
        const d = p.data || {};
        const pct = d.pct;
        return `<b>${d.name||""}USDT</b><br>${pct==null?"—":((pct>=0?"+":"")+Number(pct).toFixed(2)+"%")}<br>QV $${(Number(d.value||0)/1e6).toFixed(1)}M`;
      }
    },
    series:[{
      type:"treemap",
      animation:false,
      roam:false, nodeClick:false, breadcrumb:{show:false},
      left:0, right:0, top:0, bottom:0,
      label:{
        color:"#eef3f8", fontSize:12, fontFamily:"IBM Plex Mono",
        formatter: p => {
          const pct = p.data && p.data.pct;
          const s = pct==null ? "—" : ((pct>=0?"+":"")+Number(pct).toFixed(2)+"%");
          return (p.name||"") + "\n" + s;
        }
      },
      data
    }]
  }, {notMerge:false, lazyUpdate:true});
  setMeta("BINANCE", HEAT.ts || Date.now(), {});
}

/* ---- tape helpers (flicker-free in-place update) ---- */
function pushTape(t){
  if(!t || !t.usd) return;
  const tid = t.id || (t.t + "-" + Math.round(t.usd));
  if(MICRO.tape.some(x => x.id === tid)) return;
  MICRO.tape.push({...t, id: tid});
  if(MICRO.tape.length > 40) MICRO.tape.shift();
  renderTape();
}
function renderTape(){
  const el = $("#micro-tape"); if(!el) return;
  const items = MICRO.tape.slice(-16).reverse();
  const sig = items.map(t => t.id).join(",");
  if(MICRO.lastTapeSig === sig) return;
  MICRO.lastTapeSig = sig;
  el.innerHTML = items.map(t => {
    const col = t.side === "buy" ? "#0ecb81" : "#f6465d";
    return `<span class="tape-item" style="color:${col}">${t.side==="buy"?"▲":"▼"} $${Math.round(t.usd).toLocaleString()} @ ${Number(t.p).toFixed(2)}</span>`;
  }).join("");
}

/* ---- SSE + rAF 60fps accounting ---- */
function startSSE(){
  stopSSE();
  MICRO.alive = true;
  try{
    const es = new EventSource(`/api/micro/stream?symbol=${encodeURIComponent(MICRO.sym)}&window=${encodeURIComponent(MICRO.window)}`);
    MICRO.es = es;
    es.onmessage = (ev) => {
      if(!MICRO.alive || !MICRO.visible) return;
      try{
        const msg = JSON.parse(ev.data);
        if(msg.type === "hello") return;
        if(msg.symbol && msg.symbol !== MICRO.sym) return;
        MICRO.pending = msg;
      }catch(e){}
    };
  }catch(e){}
  MICRO._poll = setInterval(() => {
    if(!MICRO.alive || !MICRO.visible) return;
    if(MICRO.tab !== "orderflow"){
      softReload(false);
    }
  }, MICRO.tab === "market" ? 5000 : 3000);
}
function restartSSE(){ startSSE(); }
function stopSSE(){
  if(MICRO.es){ try{ MICRO.es.close(); }catch(e){} MICRO.es = null; }
  if(MICRO._poll){ clearInterval(MICRO._poll); MICRO._poll = null; }
}

function startAnimLoop(){
  if(MICRO.animLoop) return;
  const loop = (now) => {
    MICRO.animLoop = requestAnimationFrame(loop);
    if(!MICRO.alive) return;
    MICRO.frames++;
    if(now - MICRO.fpsT > 600){
      MICRO.fps = Math.round(MICRO.frames * 1000 / (now - MICRO.fpsT));
      MICRO.frames = 0; MICRO.fpsT = now;
      setText($("#micro-fps"), `fps ${MICRO.fps}`);
    }
    if(!MICRO.visible) return;
    if(MICRO.pending){
      const m = MICRO.pending; MICRO.pending = null;
      if(m.health) setMeta((MICRO.lastSnap[MICRO.tab]||{}).source || "BINANCE", m.ts, m.health);
      if(MICRO.tab === "orderflow" && m.orderflow){
        const of = m.orderflow;
        const prev = MICRO.lastSnap.orderflow;
        const lastB = (of.bars || []).slice(-1)[0] || {};
        const prevB = (prev && prev.bars || []).slice(-1)[0] || {};
        const changed = !prev || lastB.t !== prevB.t || Math.abs((lastB.delta||0) - (prevB.delta||0)) > 1e-4;
        MICRO.lastSnap[snapKey("orderflow", MICRO.sym, MICRO.window)] = of;
        MICRO.lastSnap.orderflow = of;
        if(changed){
          MICRO.updateCounts.orderflow = (MICRO.updateCounts.orderflow || 0) + 1;
          paintOrderFlow($("#micro-body"), of);
          setMeta("BINANCE", m.ts, m.health);
        }
      }
    }
  };
  MICRO.animLoop = requestAnimationFrame(loop);
}

function stopMicro(full=true){
  MICRO.alive = false;
  stopSSE();
  if(MICRO.prefetchTimer){ clearTimeout(MICRO.prefetchTimer); MICRO.prefetchTimer = 0; }
  document.removeEventListener("visibilitychange", onVis);
  window.removeEventListener("resize", onResize);
  if(MICRO.animLoop){ cancelAnimationFrame(MICRO.animLoop); MICRO.animLoop = 0; }
  if(full) disposeCharts();
}
window.MICRO = MICRO;
window.microHeatmapStop = () => stopMicro(true);
})();
