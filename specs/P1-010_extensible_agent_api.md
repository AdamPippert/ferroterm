# PRD P1-010 – Extensible Agent “Kitten” API

## Vision
Open plugin ecosystem (Rust/Python) without bloating core binary.

## Architecture
* UDS RPC (protobuf) exposes `spawn_pane`, `render_overlay`, `query_context`.
* Capability manifest per plugin; enforced by broker.
* Rate-limit 30 RPC/s to protect FPS.

## Success Metrics
* Third-party ASCII-chart plugin renders ≤ 30 ms, CPU < 5 %.
* No > 1 MB/s leak after 24 h soak.

## Security
* Plugins default to same container isolation as panes.
