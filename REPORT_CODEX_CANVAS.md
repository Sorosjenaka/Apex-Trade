# Control Room canvas optimization — 6 November 2026

Changed `dashboard/static/flow.js`, `swarm.js`, `control-room.css` and added `mesh-canvas.js`. The active worktree's other uncommitted changes and `app.js` were preserved. Prior versions are backed up in the Codex workspace under `control-room-before-canvas-20261106`.

The SVG edges, per-edge CSS animation and SMIL particles were replaced with a single viewport canvas and one requestAnimationFrame loop. The underlying Bezier paths, including their low-opacity wide fake glow and dashed texture, are cached in an offscreen canvas and rebuilt when geometry, colors, or dimensions change. Particles move mathematically along the curves. The canvas follows React Flow pan and zoom and pauses when the graph or browser tab is not visible. Status colors and the existing illustrative-animation disclaimer remain in place. Dashed texture itself is cached/static; particles are animated.

Node component references and stable Swarm IDs are retained when their contents and layout do not change. A fixture changed one risk value and cycle number: the unchanged feed node, Market Reader node, first pill, and canvas retained their DOM identities and the changed risk value updated. Compact/expand caused a geometry cache rebuild, and navigation/zoom and mobile resizing were checked.

Verification against the active dashboard at port 8787: 11 main nodes, 44 pills, 98 cached curves, one canvas, zero SVG mesh edges, and SSE readyState 1 while CHOP_NO_TRADE was displayed. Static files returned by the server matched local files. JavaScript syntax checks and existing Swarm derivation tests passed. No CSS drop-shadow, backdrop-filter, SVG animateMotion, shadowBlur, or animated stroke-dashoffset remains in the changed mesh files.

In one limited browser-tool sample the canvas draw callback averaged about 0.7–1.0 ms over 15–20 sampled frames; this excludes GPU/compositing and the browser tool throttled frames between interactions. Therefore a sustained 60 FPS has not been established and will vary by device. No Rust, OMS, risk, arm/mode, order, restart, merge, or commit was changed.
