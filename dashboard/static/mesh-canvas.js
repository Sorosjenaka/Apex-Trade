import React, { useEffect, useRef } from 'react';
import { useReactFlow } from '@xyflow/react';

// One viewport-sized canvas, one animation loop. All curves are cached in graph coordinates.
export function MeshCanvas({ edges, sceneKey }) {
  const ref = useRef(null), scene = useRef(edges), dirty = useRef(true);
  const flow = useReactFlow();
  const flowRef = useRef(flow); flowRef.current=flow;
  scene.current = edges;
  useEffect(() => { dirty.current = true; }, [sceneKey]);
  useEffect(() => {
    const canvas=ref.current, root=canvas?.closest('.react-flow');
    if (!canvas || !root) return;
    const ctx=canvas.getContext('2d'), cache=document.createElement('canvas'), base=cache.getContext('2d');
    let frame=0, running=false, intersecting=true, geometries=[], bounds={x:0,y:0}, dpr=1, cssWidth=0, cssHeight=0;
    let nodesContainer=null, observed=new Set(), count=0, sum=0, max=0, started=0, builds=0;
    const mark=()=>{dirty.current=true;};
    const sizeObserver=new ResizeObserver(mark), childrenObserver=new MutationObserver(syncNodes);
    sizeObserver.observe(root);
    function syncNodes(){
      const found=root.querySelector('.react-flow__nodes');
      if(found && found!==nodesContainer){ if(nodesContainer) childrenObserver.disconnect(); nodesContainer=found; childrenObserver.observe(found,{childList:true}); }
      const current=new Set(found ? found.querySelectorAll(':scope > .react-flow__node') : []);
      for(const el of observed) if(!current.has(el)){sizeObserver.unobserve(el);observed.delete(el);}
      for(const el of current) if(!observed.has(el)){sizeObserver.observe(el);observed.add(el);}
      mark();
    }
    syncNodes();
    const bootstrap=new MutationObserver(()=>{if(!nodesContainer){syncNodes();if(nodesContainer) bootstrap.disconnect();}});
    if(!nodesContainer) bootstrap.observe(root,{childList:true,subtree:true});
    const intersection=new IntersectionObserver(entries=>{intersecting=entries[0].isIntersecting; updateRunning();});
    intersection.observe(root);
    function resize(){
      const rect=canvas.getBoundingClientRect(), ratio=Math.min(window.devicePixelRatio||1,2);
      const w=Math.max(1,Math.round(rect.width*ratio)), h=Math.max(1,Math.round(rect.height*ratio));
      if(canvas.width!==w || canvas.height!==h){canvas.width=w;canvas.height=h;}
      cssWidth=rect.width;cssHeight=rect.height;dpr=ratio;
    }
    function rebuild(){
      resize();
      const rect=canvas.getBoundingClientRect(), v=flowRef.current.getViewport();
      if (!rect.width || !rect.height || !v.zoom || !nodesContainer) return false;
      const endpoints=new Map();
      for(const el of nodesContainer.querySelectorAll(':scope > .react-flow__node')){
        const id=el.getAttribute('data-id'), pair={};
        for(const type of ['source','target']){
          const handle=el.querySelector('.react-flow__handle.'+type);
          if(!handle) continue;
          const r=handle.getBoundingClientRect();
          pair[type]={x:(r.left+r.width/2-rect.left-v.x)/v.zoom,y:(r.top+r.height/2-rect.top-v.y)/v.zoom};
        }
        endpoints.set(id,pair);
      }
      const curves=[];
      for(const edge of scene.current){
        const a=endpoints.get(edge.source)?.source, b=endpoints.get(edge.target)?.target;
        if(!a || !b) continue;
        const dx=Math.max(42,Math.abs(b.x-a.x)*.5);
        curves.push({x1:a.x,y1:a.y,cx1:a.x+dx,cy1:a.y,cx2:b.x-dx,cy2:b.y,x2:b.x,y2:b.y,color:edge.data.color, phase:(curves.length*.137)%1});
      }
      if(curves.length!==scene.current.length){ dirty.current=true; return false; }
      let minX=Infinity,minY=Infinity,maxX=-Infinity,maxY=-Infinity;
      for(const p of curves){minX=Math.min(minX,p.x1,p.cx1,p.cx2,p.x2);maxX=Math.max(maxX,p.x1,p.cx1,p.cx2,p.x2);minY=Math.min(minY,p.y1,p.y2);maxY=Math.max(maxY,p.y1,p.y2);}
      if(!curves.length){geometries=[];dirty.current=false;return true;}
      bounds={x:Math.floor(minX-12),y:Math.floor(minY-12)};
      const width=Math.ceil(maxX-bounds.x+12), height=Math.ceil(maxY-bounds.y+12);
      // Bound cache memory when a future graph grows dramatically.
      if(width>6000 || height>6000) return false;
      cache.width=width;cache.height=height;
      base.setTransform(1,0,0,1,-bounds.x,-bounds.y);
      for(const p of curves){
        base.beginPath();base.moveTo(p.x1,p.y1);base.bezierCurveTo(p.cx1,p.cy1,p.cx2,p.cy2,p.x2,p.y2);
        base.strokeStyle=p.color;base.globalAlpha=.18;base.lineWidth=5;base.setLineDash([]);base.stroke();
        base.globalAlpha=.75;base.lineWidth=1.5;base.stroke();
        base.strokeStyle='#fff';base.globalAlpha=.22;base.lineWidth=1;base.setLineDash([10,14]);base.stroke();
      }
      base.setLineDash([]);base.globalAlpha=1;
      geometries=curves;dirty.current=false;builds++;
      return true;
    }
    function draw(now){
      if(!running) return;
      const before=performance.now();
      if(dirty.current) rebuild();
      const v=flowRef.current.getViewport();
      ctx.setTransform(1,0,0,1,0,0);ctx.clearRect(0,0,canvas.width,canvas.height);
      ctx.setTransform(dpr*v.zoom,0,0,dpr*v.zoom,dpr*v.x,dpr*v.y);
      if(geometries.length) ctx.drawImage(cache,bounds.x,bounds.y);
      for(const p of geometries){
        const t=(now/1500+p.phase)%1, u=1-t;
        const x=u*u*u*p.x1+3*u*u*t*p.cx1+3*u*t*t*p.cx2+t*t*t*p.x2;
        const y=u*u*u*p.y1+3*u*u*t*p.cy1+3*u*t*t*p.cy2+t*t*t*p.y2;
        if(x*v.zoom+v.x < -10 || x*v.zoom+v.x > cssWidth+10 || y*v.zoom+v.y < -10 || y*v.zoom+v.y > cssHeight+10) continue;
        ctx.globalAlpha=.25;ctx.fillStyle=p.color;ctx.beginPath();ctx.arc(x,y,5,0,Math.PI*2);ctx.fill();
        ctx.globalAlpha=1;ctx.fillStyle='#fff';ctx.beginPath();ctx.arc(x,y,2.6,0,Math.PI*2);ctx.fill();
      }
      ctx.globalAlpha=1;
      const elapsed=performance.now()-before;
      if(!started) started=now;
      count++;sum+=elapsed;max=Math.max(max,elapsed);
      window.__apexMeshStats={frames:count,elapsedMs:now-started,drawMeanMs:sum/count,drawMaxMs:max,cacheBuilds:builds,curves:geometries.length};
      frame=requestAnimationFrame(draw);
    }
    function updateRunning(){
      const active=intersecting && !document.hidden;
      if(active && !running){running=true;frame=requestAnimationFrame(draw);}
      else if(!active && running){running=false;cancelAnimationFrame(frame);}
    }
    document.addEventListener('visibilitychange',updateRunning);window.addEventListener('resize',mark);
    updateRunning();
    return ()=>{running=false;cancelAnimationFrame(frame);intersection.disconnect();bootstrap.disconnect();childrenObserver.disconnect();sizeObserver.disconnect();document.removeEventListener('visibilitychange',updateRunning);window.removeEventListener('resize',mark);delete window.__apexMeshStats;};
  }, []);
  return React.createElement('canvas',{ref,className:'cr-mesh-canvas','aria-hidden':true});
}
