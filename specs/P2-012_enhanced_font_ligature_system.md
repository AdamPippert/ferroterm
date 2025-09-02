# PRD P2-012 – Enhanced Font / Ligature System

## Goal
Reach Kitty-level fidelity: programmable ligatures, Nerd-Font fallback, perfect box drawing.

## Functional Requirements
* Per-char font fallback with `cosmic-text`.
* Ligature toggle per pane.
* Atlas cache persisted across sessions.

## KPI
| Test | Target |
|------|--------|
| 1 000 ligature lines | ≥ 400 FPS |
| Box drawing | Pixel-perfect HiDPI |

## Out-of-Scope
* User-supplied ligature scripts (phase 2).
