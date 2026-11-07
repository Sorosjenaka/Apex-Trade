/* Port indikator dari crates/indicators (lib.rs, regime.rs) — formula disalin ke JS.
   Label tiap indikator menyebut file Rust sumbernya. Belum diverifikasi bit-exact vs cargo test. */
(function(global){
"use strict";
const NAN = NaN;
function finite(x){ return typeof x==="number" && isFinite(x); }
function closes(bars){ return bars.map(b=>b.close); }
function sma(v,n){
  const out=Array(v.length).fill(NAN); if(!n) return out; let sum=0;
  for(let i=0;i<v.length;i++){ sum+=v[i]; if(i>=n) sum-=v[i-n]; if(i+1>=n) out[i]=sum/n; }
  return out;
}
function ema(values,period){
  const out=Array(values.length).fill(NAN); if(!period) return out;
  const alpha=2/(period+1); let last=NAN;
  for(let i=0;i<values.length;i++){
    const v=values[i]; if(!finite(v)) continue;
    last = finite(last) ? alpha*v+(1-alpha)*last : v; out[i]=last;
  }
  return out;
}
function rma_wilder(values,period){
  const out=Array(values.length).fill(NAN); if(!period) return out;
  let seed_sum=0, seed_n=0, last=NAN;
  for(let i=0;i<values.length;i++){
    const v=values[i]; if(!finite(v)) continue;
    if(!finite(last)){
      seed_sum+=v; seed_n++; if(seed_n<period) continue; last=seed_sum/period;
    } else last=(last*(period-1)+v)/period;
    out[i]=last;
  }
  return out;
}
function true_range(b){
  return b.map((bar,i)=>{
    const pc=i===0?bar.close:b[i-1].close;
    return Math.max(bar.high-bar.low, Math.abs(bar.high-pc), Math.abs(bar.low-pc));
  });
}
function atr(b,period){ return rma_wilder(true_range(b), period); }
function rsi_wilder(c,period){
  const n=c.length,g=Array(n).fill(NAN),l=Array(n).fill(NAN);
  for(let i=1;i<n;i++){ const d=c[i]-c[i-1]; g[i]=Math.max(d,0); l[i]=Math.max(-d,0); }
  const ag=rma_wilder(g,period), al=rma_wilder(l,period);
  return c.map((_,i)=>{
    const a=ag[i],b=al[i];
    if(!finite(a)||!finite(b)||(a===0&&b===0)) return NAN;
    if(b===0) return 100;
    return 100-100/(1+a/b);
  });
}
function adx(b,period){
  const n=b.length, pdm=Array(n).fill(NAN), mdm=Array(n).fill(NAN);
  for(let i=1;i<n;i++){
    const up=b[i].high-b[i-1].high, dn=b[i-1].low-b[i].low;
    pdm[i]=(up>dn && up>0)?up:0; mdm[i]=(dn>up && dn>0)?dn:0;
  }
  let tr=true_range(b); if(n) tr[0]=NAN;
  const st=rma_wilder(tr,period), sp=rma_wilder(pdm,period), sm=rma_wilder(mdm,period);
  const dx=Array(n).fill(NAN);
  for(let i=0;i<n;i++){
    if(finite(st[i])&&st[i]>0&&finite(sp[i])&&finite(sm[i])){
      const p=100*sp[i]/st[i], m=100*sm[i]/st[i];
      if(p+m>0) dx[i]=100*Math.abs(p-m)/(p+m);
    }
  }
  return rma_wilder(dx,period);
}
function efficiency_ratio(c,n){
  const out=Array(c.length).fill(NAN);
  for(let i=n;i<c.length;i++){
    let path=0; for(let k=i-n+1;k<=i;k++) path+=Math.abs(c[k]-c[k-1]);
    out[i]=path>0?Math.abs(c[i]-c[i-n])/path:0;
  }
  return out;
}
function chop(b,n){
  const tr=true_range(b), out=Array(b.length).fill(NAN);
  for(let i=n;i<b.length;i++){
    let s=0,hh=-Infinity,ll=Infinity;
    for(let k=i-n+1;k<=i;k++){ s+=tr[k]; hh=Math.max(hh,b[k].high); ll=Math.min(ll,b[k].low); }
    if(hh>ll) out[i]=100*Math.log10(s/(hh-ll))/Math.log10(n);
  }
  return out;
}
function percent_rank(v,window){
  const out=Array(v.length).fill(NAN);
  for(let i=window;i<v.length;i++){
    if(!finite(v[i])) continue;
    const w=v.slice(i-window,i).filter(finite);
    if(!w.length) continue;
    const le=w.filter(x=>x<=v[i]).length;
    out[i]=100*le/w.length;
  }
  return out;
}
function pivots(b,left,right){
  // crates/indicators/src/lib.rs pivots — confirmed when right bars exist
  const out=[];
  for(let i=left;i<b.length-right;i++){
    let hi=true,lo=true;
    for(let j=i-left;j<=i+right;j++){
      if(j===i) continue;
      if(b[j].high>=b[i].high) hi=false;
      if(b[j].low<=b[i].low) lo=false;
    }
    if(hi) out.push({idx:i,time:b[i].time,price:b[i].high,high:true,confirmed_idx:i+right});
    if(lo) out.push({idx:i,time:b[i].time,price:b[i].low,high:false,confirmed_idx:i+right});
  }
  return out;
}
function srv(c,len,post){
  // crates/indicators/src/regime.rs — Signed Realized Variance (simplified port)
  const n=c.length, out=Array(n).fill(NAN), sign=Array(n).fill(0);
  for(let i=len;i<n;i++){
    let sum=0; for(let k=i-len+1;k<=i;k++){ const r=Math.log(c[k]/c[k-1]); sum+=r*r; }
    out[i]=sum;
    sign[i]=c[i]>=c[i-len]?1:-1;
  }
  return {srv:out, sign};
}
function delta_proxy(bars){
  // taker buy approx not in public kline OHLC — use signed volume proxy from candle direction
  return bars.map(b=>{
    const range=b.high-b.low; if(range<=0) return 0;
    const closePos=(b.close-b.low)/range;
    return (closePos-0.5)*2*(b.volume||0); // approximation — engine uses taker_buy_base
  });
}
function p90_abs(v,win){
  const out=Array(v.length).fill(NAN);
  for(let i=win;i<v.length;i++){
    const w=v.slice(i-win,i).map(Math.abs).filter(finite).sort((a,b)=>a-b);
    if(!w.length) continue;
    const rank=0.9*(w.length-1); const lo=Math.floor(rank), hi=Math.ceil(rank);
    out[i]=w[lo]+(w[hi]-w[lo])*(rank-lo);
  }
  return out;
}

const CATALOG = [
  {id:"ema", name:"EMA", src:"crates/indicators/src/lib.rs :: ema", pane:"overlay",
    defaults:{period:20,color:"#3d8bfd"}, params:[{key:"period",label:"Period",type:"number",min:1,max:400}]},
  {id:"sma", name:"SMA", src:"crates/indicators/src/lib.rs :: sma", pane:"overlay",
    defaults:{period:20,color:"#a78bfa"}, params:[{key:"period",label:"Period",type:"number",min:1,max:400}]},
  {id:"atr", name:"ATR (Wilder RMA)", src:"crates/indicators/src/lib.rs :: atr", pane:"pane",
    defaults:{period:14,color:"#f0b90b"}, params:[{key:"period",label:"Period",type:"number",min:1,max:100}]},
  {id:"adx", name:"ADX", src:"crates/indicators/src/lib.rs :: adx", pane:"pane",
    defaults:{period:14,color:"#eaecef"}, params:[{key:"period",label:"Period",type:"number",min:1,max:100}]},
  {id:"rsi", name:"RSI Wilder", src:"crates/indicators/src/lib.rs :: rsi_wilder", pane:"pane",
    defaults:{period:14,color:"#f0b90b"}, params:[{key:"period",label:"Period",type:"number",min:2,max:100}]},
  {id:"er", name:"Efficiency Ratio (Kaufman)", src:"crates/indicators/src/lib.rs :: efficiency_ratio", pane:"pane",
    defaults:{period:10,color:"#3d8bfd"}, params:[{key:"period",label:"n",type:"number",min:2,max:100}]},
  {id:"chop", name:"Choppiness Index", src:"crates/indicators/src/lib.rs :: chop", pane:"pane",
    defaults:{period:14,color:"#f6465d"}, params:[{key:"period",label:"n",type:"number",min:2,max:100}]},
  {id:"pivots", name:"Pivots confirmed", src:"crates/indicators/src/lib.rs :: pivots", pane:"overlay",
    defaults:{left:2,right:2,colorH:"#f6465d",colorL:"#0ecb81"}, params:[
      {key:"left",label:"Left",type:"number",min:1,max:10},{key:"right",label:"Right",type:"number",min:1,max:10}]},
  {id:"srv", name:"SRV (Signed Realized Var)", src:"crates/indicators/src/regime.rs :: srv", pane:"pane",
    defaults:{len:24,color:"#0ecb81"}, params:[{key:"len",label:"len",type:"number",min:2,max:200}]},
  {id:"atr_pctile", name:"ATR %rank", src:"crates/indicators/src/lib.rs :: percent_rank(atr)", pane:"pane",
    defaults:{atrPeriod:14,window:100,color:"#a78bfa"}, params:[
      {key:"atrPeriod",label:"ATR",type:"number",min:1,max:50},{key:"window",label:"Window",type:"number",min:10,max:500}]},
  {id:"delta_p90", name:"Delta proxy & P90(|delta|)", src:"aproksimasi ofcc/sell_push (engine pakai taker_buy_base)", pane:"pane",
    defaults:{win:100,color:"#f0b90b"}, params:[{key:"win",label:"Window",type:"number",min:20,max:500}], approx:true},
];

const PRESET_V10 = [
  {id:"ema", period:20}, {id:"atr", period:14}, {id:"er", period:10}, {id:"chop", period:14},
  {id:"adx", period:14}, {id:"pivots", left:2, right:2}, {id:"atr_pctile"}, {id:"srv", len:24}, {id:"delta_p90"}
];

function compute(id, bars, params={}){
  const c=closes(bars);
  switch(id){
    case "ema": return {type:"line", values:ema(c, params.period||20).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "sma": return {type:"line", values:sma(c, params.period||20).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "atr": return {type:"line", values:atr(bars, params.period||14).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "adx": return {type:"line", values:adx(bars, params.period||14).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "rsi": return {type:"line", values:rsi_wilder(c, params.period||14).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "er": return {type:"line", values:efficiency_ratio(c, params.period||10).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "chop": return {type:"line", values:chop(bars, params.period||14).map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    case "pivots": {
      const pv=pivots(bars, params.left||2, params.right||2);
      return {type:"markers", values:pv};
    }
    case "srv": {
      const r=srv(c, params.len||24, 0);
      return {type:"line", values:r.srv.map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    }
    case "atr_pctile": {
      const a=atr(bars, params.atrPeriod||14);
      const pr=percent_rank(a, params.window||100);
      return {type:"line", values:pr.map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value))};
    }
    case "delta_p90": {
      const d=delta_proxy(bars); const p=p90_abs(d, params.win||100);
      return {type:"line", values:p.map((v,i)=>({time:bars[i].time,value:v})).filter(x=>finite(x.value)), approx:true};
    }
    default: return {type:"line", values:[]};
  }
}

global.ApexIndicators = {CATALOG, PRESET_V10, compute, ema, sma, atr, adx, rsi_wilder, efficiency_ratio, chop, pivots, srv, percent_rank, rma_wilder, true_range};
})(window);
