# Ferroterm PRD Priority Mapping

## Overview
This document establishes a unified priority ordering for all Ferroterm PRDs, resolving circular dependencies and ensuring proper implementation sequence. Tasks are renumbered to reflect correct dependency order.

## Priority Classification

### Phase 1: Foundation Layer (Weeks 1-2)
**Core components with no dependencies**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T001 | TTY Engine | `core-runtime/tty-engine-prd.md` | None (foundation) |
| T002 | Plain-Text Config | `config-state/plain-text-config-prd.md` | None |
| T003 | GPU Renderer | `core-runtime/renderer-prd.md` | TTY Engine |

### Phase 2: Core I/O Layer (Weeks 3-4)
**Input/output and rendering systems**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T004 | Input/Keymap System (Basic) | `core-runtime/input-keymap-prd.md` | TTY Engine, Config System |
| T005 | Dual Render Back-Ends | `P0-001_dual_render_back_ends.md` | TTY Engine, Plain-Text Config, Input/Keymap |
| T006 | Command Prefix Parser (Basic) | `agent-subsystem/command-parser-prd.md` | Input/Keymap, Config |
| T007 | Input/Keymap System (Complete) | `core-runtime/input-keymap-prd.md` | Command Prefix Parser integration |

### Phase 3: AI Infrastructure (Weeks 5-6)
**AI/ML components with resolved circular dependencies**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T008 | Profile Cache (Stub Interface) | `config-state/profile-cache-prd.md` | Plain-Text Config |
| T009 | Model Host | `agent-subsystem/model-host-prd.md` | Command Parser, Config, Profile Cache |
| T010 | Streaming UI | `agent-subsystem/streaming-ui-prd.md` | Model Host, Renderer, Input/Keymap |
| T011 | Profile Cache (Complete Integration) | `config-state/profile-cache-prd.md` | Model Host integration |

### Phase 4: Terminal Features (Weeks 7-8)
**Advanced terminal functionality**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T012 | tmux-Compatible Multiplexer | `P0-002_tmux_compatible_multiplexer.md` | TTY Engine, GPU Renderer, Input/Keymap, Config |
| T013 | Privacy & Telemetry Controls | `P0-006_privacy_and_telemetry.md` | Config System |
| T014 | Minimal Config & Auto-Theme | `P0-007_minimal_config_and_auto_theme.md` | Plain-Text Config |
| T015 | OS-Awareness Agent | `P0-005_os_awareness_agent.md` | Model Host, Command Parser |

### Phase 5: Containerization (Weeks 9-10)
**Container isolation and shared memory**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T016 | OCI Session Launcher (Stub Memory) | `P0-003_oci_crun_session_launcher.md` | TTY Engine, Model Host, Plain-Text Config |
| T017 | Shared Memory Buffers API | `P0-004_shared_memory_buffers_api.md` | TTY Engine, OCI Session Launcher, Command Parser, Model Host |
| T018 | OCI Session Launcher (Complete Integration) | `P0-003_oci_crun_session_launcher.md` | Shared Memory Buffers integration |

### Phase 6: Advanced Features (Weeks 11-12)
**Media, performance optimization, and extensibility**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T019 | Image/Media Display | `P1-008_image_media_display.md` | Renderer, Shared Memory |
| T020 | Model Hot-Swap VRAM Arbitration | `P1-009_model_hot_swap_vram_arbitration.md` | Model Host, OCI Launcher |
| T021 | Plugin SDK | `extensibility/plugin-sdk-prd.md` | Core Runtime, Config, Command Parser |
| T022 | Extensible Agent API | `P1-010_extensible_agent_api.md` | Model Host, Plugin SDK |

### Phase 7: Platform & Distribution (Weeks 13-14)
**Platform-specific features and deployment**

| Task ID | Component | Location | Dependencies |
|---------|-----------|----------|-------------|
| T023 | macOS Metal Backend | `P2-011_macos_metal_backend.md` | Renderer |
| T024 | Enhanced Font/Ligature System | `P2-012_enhanced_font_ligature_system.md` | Renderer |
| T025 | Installer | `distribution-operations/installer-prd.md` | Build System |
| T026 | Security Hardening | `distribution-operations/security-prd.md` | All Components (cross-cutting) |
| T027 | Telemetry Infrastructure | `distribution-operations/telemetry-prd.md` | Config System, Privacy Controls |

## Implementation Dependencies

### Critical Path
The following represents the critical dependency path that determines minimum implementation time:

```
T001: TTY Engine → 
T003: GPU Renderer → 
T004: Input/Keymap (Basic) → 
T006: Command Parser → 
T009: Model Host → 
T010: Streaming UI
```

### Parallel Development Tracks

**Track 1: Core Runtime (Weeks 1-4)**
- T001: TTY Engine
- T002: Plain-Text Config
- T003: GPU Renderer
- T004-T007: Input/Keymap System (staged)
- T005: Dual Render Back-Ends

**Track 2: AI Stack (Weeks 5-6)**
- T006: Command Prefix Parser
- T008-T011: Profile Cache (staged)
- T009: Model Host
- T010: Streaming UI

**Track 3: Terminal Features (Weeks 7-8)**
- T012: tmux-Compatible Multiplexer
- T013: Privacy & Telemetry Controls
- T014: Minimal Config & Auto-Theme
- T015: OS-Awareness Agent

**Track 4: Advanced Systems (Weeks 9-12)**
- T016-T018: OCI/Shared Memory (staged)
- T019: Image/Media Display
- T020: VRAM Arbitration
- T021-T022: Plugin System

**Track 5: Distribution (Weeks 13-14)**
- T023-T024: Platform-specific backends
- T025: Installer
- T026: Security Hardening
- T027: Telemetry Infrastructure

## Circular Dependency Resolution

### Staged Implementation Strategy
Components with circular dependencies are implemented in stages:

1. **Input/Keymap ↔ Command Parser**:
   - T004: Basic Input/Keymap (without agent integration)
   - T006: Command Parser (using basic input interface)
   - T007: Complete Input/Keymap (with agent integration)

2. **Model Host ↔ Profile Cache**:
   - T008: Profile Cache stub interface
   - T009: Model Host (using stub cache)
   - T011: Complete Profile Cache integration

3. **OCI ↔ Shared Memory**:
   - T016: OCI Session Launcher (basic container support)
   - T017: Shared Memory Buffers API
   - T018: Complete OCI integration with shared memory

### Interface-First Development
All circular dependencies use interface/trait definitions:
- Define traits early in the implementation cycle
- Implement with stub/mock implementations initially
- Replace stubs with full implementations in later stages

## Milestone Alignment

### Week 2: Foundation Complete
- T001: TTY Engine (full functionality)
- T002: Plain-Text Config (live reload)
- T003: GPU Renderer (basic text rendering)

### Week 4: I/O Systems Ready
- T004: Input/Keymap (basic key handling)
- T005: Dual Render Back-Ends (GPU + ANSI fallback)
- T006: Command Parser (prefix detection)
- T007: Complete Input/Keymap (agent integration)

### Week 6: AI Integration Complete
- T008: Profile Cache (stub interface)
- T009: Model Host (local model support)
- T010: Streaming UI (real-time responses)
- T011: Complete Profile Cache (model-specific settings)

### Week 8: Terminal Features Ready
- T012: tmux-Compatible Multiplexer
- T013: Privacy & Telemetry Controls
- T014: Minimal Config & Auto-Theme
- T015: OS-Awareness Agent

### Week 10: Containerization Complete
- T016: OCI Session Launcher (basic)
- T017: Shared Memory Buffers API
- T018: Complete OCI integration

### Week 12: Advanced Features Ready
- T019: Image/Media Display
- T020: VRAM Arbitration
- T021: Plugin SDK
- T022: Extensible Agent API

### Week 14: Production Ready
- T023: macOS Metal Backend
- T024: Enhanced Font/Ligature System
- T025: Installer
- T026: Security Hardening
- T027: Telemetry Infrastructure

## Update Protocol

When priorities change:
1. Update this mapping document
2. Adjust milestone timelines accordingly  
3. Communicate changes to all development tracks
4. Update individual PRDs if dependencies change

This priority system ensures systematic development while allowing parallel work streams.