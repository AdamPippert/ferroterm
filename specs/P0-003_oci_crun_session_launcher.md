# OCI Session Launcher PRD

## Overview
The OCI Session Launcher isolates each terminal pane in a rootless OCI container embedding a Ramalama-wrapped vLLM process. This architecture preserves host cleanliness while enabling on-the-fly model swaps and secure isolation between sessions.

## Dependencies
- TTY Engine (for managing containerized terminal sessions)
- Model Host (for vLLM integration)
- Plain-Text Config (for container and model configuration)
- Shared Memory Buffers API (for efficient data sharing)

## Functional Requirements

### 1. Container Management
- **FR-1.1**: Launch rootless OCI containers using crun runtime
- **FR-1.2**: Support cold-start container initialization ≤ 40 ms
- **FR-1.3**: Enable hot-start for subsequent sessions ≤ 3 s
- **FR-1.4**: Manage container lifecycle (creation, monitoring, cleanup)

### 2. Model Integration
- **FR-2.1**: Embed Ramalama-wrapped vLLM processes in containers
- **FR-2.2**: Load vLLM-7B models ≤ 12 s from cold start
- **FR-2.3**: Support model specification via CLI flags
- **FR-2.4**: Enable GPU and CPU-only model execution modes

### 3. Resource Sharing
- **FR-3.1**: Implement overlayfs layer sharing for model weights
- **FR-3.2**: Prevent duplicate model loading across sessions
- **FR-3.3**: Optimize memory usage through shared layers
- **FR-3.4**: Support concurrent model access

### 4. Session Management
- **FR-4.1**: Store logs and crash dumps under ~/.ferroterm/sessions/<id>
- **FR-4.2**: Maintain session metadata and state
- **FR-4.3**: Support session enumeration and cleanup
- **FR-4.4**: Handle graceful session termination

### 5. CLI Interface
- **FR-5.1**: Support `pterm new --model <name> --gpu` command syntax
- **FR-5.2**: Support `pterm new --no-gpu --model <name>` for CPU-only mode
- **FR-5.3**: Provide session listing and management commands
- **FR-5.4**: Enable model override at session creation

## Non-Functional Requirements

### Performance
- **NFR-1.1**: Container cold-start time ≤ 40 ms
- **NFR-1.2**: Model hot-start time ≤ 3 s
- **NFR-1.3**: First token generation ≤ 400 ms after model load
- **NFR-1.4**: Minimal overhead for container orchestration

### Security
- **NFR-2.1**: Default seccomp profile with no CAP_SYS_ADMIN
- **NFR-2.2**: Image signatures verified with Cosign
- **NFR-2.3**: Rootless container execution only
- **NFR-2.4**: Secure isolation between terminal sessions

### Compatibility
- **NFR-3.1**: Support Ubuntu 22.04, Fedora 40, and WSL 2
- **NFR-3.2**: Compatible with cgroup-v2 and overlayfs
- **NFR-3.3**: Work with various container runtimes
- **NFR-3.4**: Support both GPU and CPU-only environments

## Pre-conditions
- crun runtime ≥ 1.14 installed and configured
- Ramalama base images available
- vLLM ≥ 0.4 compatible
- Kernel with cgroup-v2 and overlayfs support
- Appropriate GPU drivers if using GPU mode

## Post-conditions
- Container successfully launched with embedded vLLM
- Model loaded and ready for inference
- Session logging configured and active
- Resource sharing optimized across sessions

## Edge Cases
1. **Container runtime failures**: Graceful fallback to direct process execution
2. **VRAM exhaustion**: Coordinate with VRAM arbitration system (P1-009)
3. **Model loading failures**: Provide clear error messages and fallback options
4. **Namespace leakage**: Prevent container escape and resource conflicts
5. **Overlayfs corruption**: Detect and recover from shared layer issues
6. **Session cleanup on crash**: Ensure no orphaned containers or processes

## Success Metrics
- Session cold-start time consistently ≤ 40 ms
- First token generation ≤ 400 ms after model ready
- 100% pass rate on Ubuntu 22, Fedora 40, and WSL 2
- Zero namespace leakage incidents in testing
- Memory efficiency through successful layer sharing

## Testing Requirements
1. Performance benchmarks for container startup times
2. Security audits using bats-core integration tests
3. Cross-platform compatibility testing
4. Model loading and execution validation
5. Resource sharing efficiency tests
6. Container isolation and cleanup verification
7. Stress testing with concurrent sessions
