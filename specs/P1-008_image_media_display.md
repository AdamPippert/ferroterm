# PRD P1-008 – Image / Media Display

## Why
Kitty users love `icat`; data-scientists want inline previews.

## Functional Scope
* GPU path: Vulkan render of PNG/JPEG/WebP.
* ANSI path: Sixel or iTerm inline PNG.
* Capability negotiation via `$PACHYTERM_CAPS`.

## Performance
| Test | GPU | ANSI |
|------|-----|------|
| 1080p PNG display | ≤ 30 ms | ≤ 150 ms |

## Dependencies
* `stb_image` decoding
* DMA-Buf transfers

## Risks
* VRAM clash with large models – coordinated via PRD P1-009.

