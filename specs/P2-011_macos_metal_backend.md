i# PRD P2-011 – macOS Metal Back-End

## Conditional Scope
Ship only if alpha telemetry shows ≥ 30 % macOS share.

## Objectives
* Match Vulkan targets using `metal-rs`.
* Reuse glyph atlas & layout engine.

## KPI
* 1 M-line scroll ≥ 300 FPS on M2 MacBook Pro.

## Risks
* Metal residency quirks – add abstraction-layer tests.
