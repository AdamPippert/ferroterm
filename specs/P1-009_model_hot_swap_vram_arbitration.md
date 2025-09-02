# PRD P1-009 – Model Hot-Swap & VRAM Arbitration

## Objective
Switch models mid-session without renderer crash or VRAM overflow.

## Functional Requirements
1. Host updates `/models/current` symlink; sends `SIGHUP` to vLLM.
2. Renderer polls `VK_EXT_memory_budget`; throttles FPS if usage > 90 %.
3. Shared atomic reveals LLM VRAM claim.

## KPIs
| Metric | Target |
|--------|--------|
| Hot-swap latency | ≤ 3 s |
| First token | ≤ 500 ms |
| Stability | 48 h cycle benchmark, no OOM, FPS ≥ 30 |

## Dependencies
* vLLM hot-reload patch
* Vulkan memory-budget extension

## Risks
* Consumer GPU fragmentation – mitigate via contiguous allocator.
