# PRD Dependency Analysis

## Detailed Dependency Mapping

### Foundation Components (No Dependencies)
- **P0-101**: TTY Engine (None - foundational)
- **P0-104**: Plain-Text Config (None - consumed by all)

### Level 1 Dependencies (Foundation Only)
- **P0-102**: GPU Renderer → TTY Engine
- **P0-006**: Privacy & Telemetry → Config System
- **P0-007**: Minimal Config & Auto-Theme → Plain-Text Config  
- **P2-302**: Installer → Build System

### Level 2 Dependencies
- **P0-103**: Input/Keymap → TTY Engine, Config System, **Agent Command Parser** ⚠️
- **P0-001**: Dual Render Backends → TTY Engine, Plain-Text Config, Input/Keymap
- **P1-201**: Command Parser → Input/Keymap, Model Host ⚠️, Config
- **P2-304**: Telemetry → Config System, Privacy Controls

### Level 3 Dependencies  
- **P0-002**: tmux Multiplexer → TTY Engine, GPU Renderer, Input/Keymap, Plain-Text Config
- **P1-202**: Model Host → Command Parser, Config, Profile Cache ⚠️, Streaming UI ⚠️
- **P1-204**: Profile Cache → Plain-Text Config, Model Host ⚠️

### Level 4+ Complex Dependencies
- **P0-003**: OCI Session Launcher → TTY Engine, Model Host, Plain-Text Config, Shared Memory Buffers ⚠️
- **P0-004**: Shared Memory Buffers → TTY Engine, OCI Session Launcher ⚠️, Command Parser, Model Host
- **P0-005**: OS-Awareness Agent → Model Host, Command Parser
- **P1-203**: Streaming UI → Model Host, Renderer, Input/Keymap, Config
- **P1-008**: Image/Media Display → Renderer, Shared Memory
- **P1-009**: VRAM Arbitration → Model Host, OCI Launcher
- **P1-010**: Extensible Agent API → Model Host, Plugin SDK ⚠️
- **P2-011**: macOS Metal Backend → Renderer
- **P2-012**: Enhanced Font/Ligature → Renderer  
- **P2-301**: Plugin SDK → Core Runtime, Config, Command Parser
- **P2-303**: Security → All Components (cross-cutting)

## Circular Dependency Issues ⚠️

### Major Circular Dependencies Identified:

#### 1. **Input/Keymap ↔ Command Parser**
- **P0-103**: Input/Keymap → Agent Command Parser
- **P1-201**: Command Parser → Input/Keymap
- **Problem**: Each depends on the other for initialization

#### 2. **Model Host ↔ Command Parser ↔ Streaming UI**
- **P1-201**: Command Parser → Model Host  
- **P1-202**: Model Host → Command Parser, Streaming UI
- **P1-203**: Streaming UI → Model Host
- **Problem**: Circular dependency chain

#### 3. **Model Host ↔ Profile Cache**
- **P1-202**: Model Host → Profile Cache
- **P1-204**: Profile Cache → Model Host
- **Problem**: Each needs the other for operation

#### 4. **OCI Session Launcher ↔ Shared Memory Buffers**
- **P0-003**: OCI Session Launcher → Shared Memory Buffers
- **P0-004**: Shared Memory Buffers → OCI Session Launcher
- **Problem**: Container isolation vs data sharing

#### 5. **Plugin SDK ↔ Extensible Agent API**
- **P1-010**: Extensible Agent API → Plugin SDK
- **P2-301**: Plugin SDK → Command Parser (for custom prefix commands)
- **Problem**: Plugin system circular reference

## Implementation Order Problems

### Current Priority Order vs Dependencies:

| Current Order | Component | Missing Prerequisites |
|---------------|-----------|---------------------|
| P0-001 | Dual Render Backends | ❌ Requires P0-103 (Input/Keymap) |
| P0-002 | tmux Multiplexer | ❌ Requires P0-102 (GPU Renderer) |
| P0-003 | OCI Session Launcher | ❌ Requires P1-202 (Model Host) |
| P0-004 | Shared Memory Buffers | ❌ Requires P0-003, P1-201, P1-202 |
| P0-005 | OS-Awareness Agent | ❌ Requires P1-202, P1-201 |

### Correct Implementation Order:

#### Phase 1: True Foundations
1. **P0-101**: TTY Engine
2. **P0-104**: Plain-Text Config
3. **P0-102**: GPU Renderer (depends on TTY Engine)

#### Phase 2: Core Input/Output (Resolve Circular Dependencies)
4. **P0-103**: Input/Keymap (implement without agent parser initially)
5. **P0-001**: Dual Render Backends 
6. **P1-201**: Command Parser (implement with basic input interface)
7. **Update P0-103**: Add agent parser integration

#### Phase 3: AI Infrastructure
8. **P1-204**: Profile Cache (implement with stub model host interface)
9. **P1-202**: Model Host 
10. **P1-203**: Streaming UI
11. **Update P1-204**: Connect to real model host

#### Phase 4: Advanced Features
12. **P0-002**: tmux Multiplexer
13. **P0-006**: Privacy & Telemetry
14. **P0-007**: Config & Auto-Theme
15. **P0-005**: OS-Awareness Agent

#### Phase 5: Containerization
16. **P0-003**: OCI Session Launcher (stub shared memory initially)
17. **P0-004**: Shared Memory Buffers
18. **Update P0-003**: Integrate real shared memory

#### Phase 6: Extensions & Distribution
19. **P1-008**: Image/Media Display
20. **P1-009**: VRAM Arbitration
21. **P2-301**: Plugin SDK
22. **P1-010**: Extensible Agent API
23. **P2-011**: macOS Metal Backend
24. **P2-012**: Enhanced Font/Ligature
25. **P2-302**: Installer
26. **P2-303**: Security
27. **P2-304**: Telemetry

## Dependency Resolution Strategies

### 1. **Staged Implementation**
Break circular dependencies by implementing components in stages:
- Stage 1: Core functionality without cross-dependencies
- Stage 2: Add integration points
- Stage 3: Full feature integration

### 2. **Interface First**
Define interfaces/traits before implementation:
- Create trait definitions early
- Implement with stubs/mocks initially  
- Replace stubs with real implementations later

### 3. **Dependency Injection**
Use dependency injection patterns to break compile-time circular dependencies:
- Pass dependencies through constructors
- Use trait objects for late binding
- Implement observer/event patterns

### 4. **Configuration-Driven Dependencies**
Some dependencies can be optional based on configuration:
- Agent features can be disabled initially
- Container integration can be optional
- Plugin system can be opt-in

## Recommendations

### Immediate Actions:
1. **Reorder Priority Mapping** to reflect correct dependency sequence
2. **Implement Interface-First Strategy** for circular dependencies
3. **Create Dependency Injection Framework** in core components
4. **Define Clear Component Boundaries** to minimize coupling

### Architecture Changes:
1. **Event-Driven Architecture** for loose coupling between components
2. **Plugin Architecture** for optional features
3. **Configuration-Based Feature Toggling** for staged rollouts
4. **Clear Separation of Concerns** between core and optional features

The current PRD order has significant dependency issues that would block implementation. A revised order respecting the dependency graph is essential for successful development.