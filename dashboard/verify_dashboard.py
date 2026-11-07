"""Verifikasi dashboard (read-only): screenshot semua halaman/tab + soak N detik dengan deteksi kedip.

Pakai:  python dashboard\\verify_dashboard.py [--port 8787] [--soak 60] [--batch 4] [--no-soak]
Output: dashboard\\shots\\*.png dan dashboard\\shots\\verify_report.json
Kedip dideteksi dari: re-init/dispose chart (echarts / lightweight-charts), node canvas/svg yang dilepas
dari DOM, kontainer yang dikosongkan, teks LOADING yang muncul lagi, dan kontainer blank.
"""
import argparse, asyncio, json, time
from pathlib import Path
from playwright.async_api import async_playwright

OUT = Path(__file__).resolve().parent / "shots"
PAGES = ["control", "terminal", "research", "portfolio", "heatmap", "market", "learning", "library"]
TABS = ["orderflow", "walls", "liqmap", "largevp", "tierflow", "cohorts", "funding_oi", "liq_risk", "market"]

INSTRUMENT = """(pg) => {
  const V = window.__V = {inits:0, disposes:0, lwc:0, removedChart:0, emptied:0, loadingHits:0, blankHits:0, samples:0, fps:[]};
  try{ if(window.echarts && !window.__echWrapped){ window.__echWrapped=1; const oi=echarts.init;
    echarts.init=function(){ V.inits++; const c=oi.apply(this,arguments); if(c&&c.dispose){const d=c.dispose; c.dispose=function(){V.disposes++; return d.apply(this,arguments)}} return c; } } }catch(e){}
  try{ if(window.LightweightCharts && !window.__lwcWrapped){ window.__lwcWrapped=1; const oc=LightweightCharts.createChart;
    LightweightCharts.createChart=function(){ V.lwc++; return oc.apply(this,arguments); } } }catch(e){}
  const root = document.querySelector('#page-'+pg) || document.body;
  const hasChart = n => n && n.nodeType===1 && (n.tagName==='CANVAS' || n.tagName==='svg' || (n.querySelector && n.querySelector('canvas,svg')));
  window.__mo = new MutationObserver(ms => { for(const m of ms){ if(m.removedNodes && m.removedNodes.length){
      let rc=false; m.removedNodes.forEach(n=>{ if(hasChart(n)) rc=true; });
      if(rc){ V.removedChart++; if(m.target && m.target.childNodes && m.target.childNodes.length===0) V.emptied++; } } } });
  window.__mo.observe(root,{childList:true,subtree:true});
  window.__sampler = setInterval(()=>{
    V.samples++;
    const t=(root.innerText||''); if(/LOADING|MEMUAT/i.test(t) && V.samples>8) V.loadingHits++;
    const cv=root.querySelectorAll('canvas,svg').length; if(t.trim().length<20 && cv===0) V.blankHits++;
    if(window.MICRO && MICRO.fps) V.fps.push(MICRO.fps);
  },500);
}"""

async def new_ctx(p, w, h):
    b = await p.chromium.launch(headless=True)
    ctx = await b.new_context(viewport={"width": w, "height": h})
    return b, ctx

async def open_page(ctx, base, errs):
    page = await ctx.new_page()
    page.on("pageerror", lambda e: errs.append("pageerror: " + str(e)[:200]))
    page.on("console", lambda m: errs.append("console.error: " + m.text[:200]) if m.type == "error" else None)
    page.on("requestfailed", lambda r: errs.append("reqfail: " + r.url[:120]))
    await page.goto(base + "/", wait_until="domcontentloaded", timeout=30000)
    await page.wait_for_timeout(1500)
    return page

async def go_target(page, kind, name):
    t0 = time.time()
    if kind == "page":
        await page.click(f'.nav-item[data-page="{name}"]')
        pg = name
    else:
        await page.click('.nav-item[data-page="heatmap"]')
        await page.wait_for_selector("#micro-tabs", timeout=8000)
        await page.click(f'#micro-tabs [data-mt="{name}"]')
        pg = "heatmap"
    # first display: konten/canvas muncul
    try:
        if kind == "tab" and name != "market":
            await page.wait_for_function(
                "()=>{const mb=document.querySelector('#micro-body'); return !!(mb && mb.querySelectorAll('canvas').length>0)}",
                timeout=15000)
        elif kind == "page" and name == "control":
            await page.wait_for_function(
                "()=>!!(document.querySelector('#cr-flow .react-flow__node') || (document.querySelector('#page-control')||{}).innerText?.length>120)",
                timeout=15000)
        else:
            await page.wait_for_function(
                "(pg)=>{const r=document.querySelector('#page-'+pg); if(!r) return false; const mb=r.querySelector('#micro-body')||r; "
                "return mb.querySelectorAll('canvas,svg').length>0 || (mb.innerText||'').trim().length>80}", arg=pg, timeout=15000)
        first = round(time.time() - t0, 2)
    except Exception:
        first = None
    await page.wait_for_timeout(400)
    return pg, first

async def soak_worker(b, base, targets, secs):
    ctx = await b.new_context(viewport={"width": 1920, "height": 1080})
    errs = []
    try:
        page = await open_page(ctx, base, errs)
        await page.wait_for_timeout(2000)
        out = []
        for kind, name in targets:
            errs.clear()
            pg, first = await go_target(page, kind, name)
            await page.evaluate(INSTRUMENT, pg)
            await page.wait_for_timeout(int(secs * 1000))
            v = await page.evaluate("()=>{clearInterval(window.__sampler); window.__mo.disconnect(); return window.__V}")
            fps = v.pop("fps", [])
            res = {"target": f"{kind}:{name}", "first_display_s": first, "errors": list(errs[:8]), "errors_n": len(errs),
                   "echarts_inits_after_warm": v["inits"], "echarts_disposes": v["disposes"], "lwc_creates": v["lwc"],
                   "chart_nodes_removed": v["removedChart"], "containers_emptied": v["emptied"],
                   "loading_text_hits": v["loadingHits"], "blank_hits": v["blankHits"], "samples": v["samples"],
                   "fps_avg": round(sum(fps) / len(fps), 1) if fps else None}
            res["flicker"] = bool(v["inits"] > 0 or v["disposes"] > 0 or v["lwc"] > 0 or v["emptied"] > 0)
            res["ok"] = (not res["flicker"]) and v["blankHits"] == 0 and first is not None and not any("pageerror" in e for e in errs)
            print(res["target"], "OK" if res["ok"] else "FAIL", "first", res["first_display_s"], "flicker", res["flicker"],
                  "blank", res["blank_hits"], "errs", res["errors_n"], flush=True)
            out.append(res)
        return out
    finally:
        await ctx.close()

async def shots(p, base, w, h):
    tag = f"{w}x{h}"
    b, ctx = await new_ctx(p, w, h)
    errs = []
    out = []
    try:
        page = await open_page(ctx, base, errs)
        for pg in PAGES:
            await go_target(page, "page", pg)
            await page.wait_for_timeout(2500)
            f = OUT / f"{tag}-{pg}.png"
            await page.screenshot(path=str(f)); out.append(f.name)
        if w >= 1900:
            for t in TABS:
                await go_target(page, "tab", t)
                await page.wait_for_timeout(2500)
                f = OUT / f"{tag}-heatmap-{t}.png"
                await page.screenshot(path=str(f)); out.append(f.name)
    finally:
        await b.close()
    return {"res": tag, "files": out, "errors": errs[:10], "errors_n": len(errs)}

async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8787)
    ap.add_argument("--soak", type=float, default=60)
    ap.add_argument("--batch", type=int, default=4)
    ap.add_argument("--no-soak", action="store_true")
    ap.add_argument("--no-shots", action="store_true")
    a = ap.parse_args()
    base = f"http://127.0.0.1:{a.port}"
    OUT.mkdir(exist_ok=True)
    report = {"base": base, "shots": [], "soak": []}
    async with async_playwright() as p:
        if not a.no_shots:
            for (w, h) in ((1920, 1080), (1366, 768)):
                report["shots"].append(await shots(p, base, w, h))
                print("shots", w, h, "ok", flush=True)
        if not a.no_soak:
            targets = [("page", x) for x in PAGES] + [("tab", t) for t in TABS]
            b = await p.chromium.launch(headless=True)
            try:
                buckets = [targets[i::a.batch] for i in range(a.batch)]
                lists = await asyncio.gather(*[soak_worker(b, base, bkt, a.soak) for bkt in buckets if bkt])
                for lst in lists:
                    report["soak"].extend(lst)
            finally:
                await b.close()
    report["all_ok"] = all(r["ok"] for r in report["soak"]) if report["soak"] else None
    (OUT / "verify_report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print("ALL_OK", report["all_ok"])

asyncio.run(main())
