# Ferroterm PRD Index and Dependencies

## Overview
This document provides a comprehensive index of all Ferroterm Product Requirements Documents (PRDs) and their interdependencies. Each component PRD details specific functional and non-functional requirements, pre/post conditions, and edge cases.

## PRD Structure

### Core Runtime Layer
Foundation components that provide the terminal emulation and rendering capabilities.

1. **[TTY Engine](core-runtime/tty-engine-prd.md)**
   - Foundational component with no dependencies
   - Manages PTY lifecycle, I/O relay, and signal handling
   - Required by: All components that interact with terminal

2. **[GPU Renderer](core-runtime/renderer-prd.md)**
   - Dependencies: TTY Engine
   - Provides GPU-accelerated text rendering
   - Required by: Streaming UI

3. **[Input/Keymap System](core-runtime/input-keymap-prd.md)**
   - Dependencies: TTY Engine, Config System, Agent Command Parser
   - Handles keyboard input and configurable keybindings
   - Required by: Command prefix detection

### Agent Subsystem Layer
AI integration components that enable the 'p' prefix functionality.

4. **[Command Prefix Parser](agent-subsystem/command-parser-prd.md)**
   - Dependencies: Input/Keymap System, Model Host, Config System
   - Detects and processes agent commands
   - Required by: Model execution pipeline

5. **[Model Host](agent-subsystem/model-host-prd.md)**
   - Dependencies: Command Parser, Config System, Profile Cache, Streaming UI
   - Manages AI model loading and execution
   - Required by: Agent functionality

6. **[Streaming UI](agent-subsystem/streaming-ui-prd.md)**
   - Dependencies: Model Host, Renderer, Input/Keymap, Config System
   - Displays AI responses with streaming support
   - Required by: User-facing AI features

### Configuration & State Layer
System configuration and persistent state management.

7. **[Plain-Text Config](config-state/plain-text-config-prd.md)**
   - Dependencies: None (but consumed by all components)
   - TOML-based configuration with live reload
   - Required by: All configurable components

8. **[Profile Cache](config-state/profile-cache-prd.md)**
   - Dependencies: Plain-Text Config, Model Host
   - Manages per-model configurations
   - Required by: Model customization features

### Extensibility Layer
Plugin system for third-party extensions.

9. **[Plugin SDK](extensibility/plugin-sdk-prd.md)**
   - Dependencies: Core Runtime, Config System, Command Parser
   - Enables third-party plugins
   - Required by: Extension ecosystem

### Distribution & Operations Layer
Installation, monitoring, and security infrastructure.

10. **[Installer](distribution-operations/installer-prd.md)**
    - Dependencies: Build system
    - Cross-platform installation mechanisms
    - Required by: End-user distribution

11. **[Telemetry](distribution-operations/telemetry-prd.md)**
    - Dependencies: Config System, Network stack
    - Opt-in usage statistics and error reporting
    - Required by: Product improvement

12. **[Security](distribution-operations/security-prd.md)**
    - Dependencies: Build system, CI/CD, Model Host, Plugin SDK
    - Comprehensive security measures
    - Required by: All components (security is cross-cutting)

## Dependency Graph

```
┌─────────────────┐
│   TTY Engine    │ (Foundation - No deps)
└────────┬────────┘
         │
    ┌────┴────┐
    │         ▼
    │  ┌─────────────┐     ┌──────────────┐
    │  │  Renderer   │     │ Plain-Text   │ (No deps)
    │  └──────┬──────┘     │   Config     │
    │         │            └──────┬───────┘
    │         │                   │
    │         ▼                   ▼
    │  ┌─────────────┐     ┌──────────────┐
    └─▶│Input/Keymap │◀────│Profile Cache │
       └──────┬──────┘     └──────┬───────┘
              │                   │
              ▼                   │
       ┌─────────────┐           │
       │Command Parser│◀──────────┘
       └──────┬──────┘
              │
              ▼
       ┌─────────────┐
       │ Model Host  │
       └──────┬──────┘
              │
              ▼
       ┌─────────────┐
       │Streaming UI │
       └─────────────┘

Parallel Systems:
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│ Plugin SDK  │  │  Installer  │  │  Telemetry  │
└─────────────┘  └─────────────┘  └─────────────┘

Cross-cutting:
┌─────────────┐
│  Security   │ (Affects all components)
└─────────────┘
```

## Implementation Priority

### Phase 1: Core Foundation (Weeks 1-6)
1. TTY Engine
2. GPU Renderer
3. Input/Keymap System
4. Plain-Text Config

### Phase 2: Agent Integration (Weeks 7-10)
5. Command Prefix Parser
6. Model Host
7. Streaming UI
8. Profile Cache

### Phase 3: Distribution & Polish (Weeks 11-14)
9. Installer
10. Security
11. Plugin SDK
12. Telemetry

## Testing Strategy

Each PRD includes specific testing requirements. Integration points between components require additional testing:

1. **TTY Engine ↔ Renderer**: Frame synchronization tests
2. **Input/Keymap ↔ Command Parser**: Prefix detection accuracy
3. **Model Host ↔ Streaming UI**: Token streaming performance
4. **Config ↔ All Components**: Live reload functionality
5. **Plugin SDK ↔ Core Runtime**: Sandbox security tests

## Success Criteria

The overall success of Ferroterm depends on meeting the criteria defined in each PRD:

- **Performance**: Time-to-first-frame ≤ 50ms, agent response ≤ 300ms
- **Compatibility**: 100% POSIX PTY conformance
- **Usability**: Single keystroke AI activation
- **Reliability**: Zero data loss, crash isolation
- **Security**: Sandboxed execution, signed releases

## Maintenance

This index should be updated whenever:
- New PRDs are added
- Dependencies change
- Implementation priorities shift
- Success criteria are modified

Each PRD is a living document and should be versioned appropriately.