# Dual Render Back-Ends PRD

## Overview
The Dual Render Back-Ends system provides both GPU-accelerated Vulkan rendering for high-performance local use and ANSI fallback for remote/SSH scenarios. This ensures optimal performance on modern hardware while maintaining full usability in resource-constrained environments.

## Dependencies
- TTY Engine (for terminal data source)
- Plain-Text Config (for backend selection preferences)
- Input/Keymap System (for handling rendering mode switches)

## Functional Requirements

### 1. Backend Detection and Selection
- **FR-1.1**: Automatically detect GPU capabilities and select appropriate backend
- **FR-1.2**: Support runtime override with `--no-gpu` flag
- **FR-1.3**: Graceful fallback from GPU to ANSI when GPU unavailable
- **FR-1.4**: Allow backend selection via configuration file

### 2. Rendering Parity
- **FR-2.1**: Maintain identical glyph shaping across both backends
- **FR-2.2**: Preserve color accuracy and terminal semantics
- **FR-2.3**: Ensure consistent input-latency behavior
- **FR-2.4**: Support all standard terminal features in both modes

### 3. Image Handling
- **FR-3.1**: Vulkan backend uses DMA-Buf textures for images
- **FR-3.2**: ANSI backend supports Sixel and iTerm inline PNG protocols
- **FR-3.3**: Consistent image positioning and sizing across backends
- **FR-3.4**: Memory-efficient image caching strategies

### 4. Shared Interface
- **FR-4.1**: Implement unified TerminalRenderer trait
- **FR-4.2**: Abstract backend-specific details from higher-level components
- **FR-4.3**: Support hot-swapping between backends at runtime
- **FR-4.4**: Maintain consistent API for terminal operations

## Non-Functional Requirements

### Performance
- **NFR-1.1**: GPU backend: ≥ 450 FPS for 1M-line scroll, p95 frame time < 3 ms
- **NFR-1.2**: ANSI backend: ≥ 60 FPS for 1M-line scroll, CPU ≤ 200% of one core
- **NFR-1.3**: Renderer memory usage ≤ 40 MB RSS
- **NFR-1.4**: Backend switching time < 100ms

### Compatibility
- **NFR-2.1**: Support for laptop developers (RTX 4060/Wayland at 144Hz)
- **NFR-2.2**: Full functionality over SSH and slow network connections
- **NFR-2.3**: Compatible with nested tmux sessions
- **NFR-2.4**: Works in single-user-mode console environments

### Reliability
- **NFR-3.1**: Graceful handling of GPU driver crashes
- **NFR-3.2**: Robust fallback mechanisms
- **NFR-3.3**: Consistent visual output across backend switches

## Pre-conditions
- Operating system with either GPU support or ANSI terminal capabilities
- Required graphics libraries available (Vulkan drivers for GPU backend)
- Terminal interface properly initialized
- Shared TerminalRenderer trait implemented

## Post-conditions
- Appropriate backend selected and initialized
- Rendering pipeline established with target performance
- Image handling capabilities configured for selected backend
- Fallback mechanisms ready if needed

## Edge Cases
1. **GPU driver crashes**: Automatic fallback to ANSI backend
2. **Insufficient VRAM**: Degrade to software rendering or ANSI fallback  
3. **SSH connection drops**: Maintain session state for reconnection
4. **Mixed environments**: Handle transitions between local and remote usage
5. **Driver compatibility issues**: Maintain deny-list for problematic drivers
6. **Rapid backend switching**: Prevent resource leaks during transitions

## Success Metrics
- Performance targets met: ≥450 FPS (GPU), ≥60 FPS (ANSI)
- Memory usage stays ≤40 MB RSS under all conditions
- Screenshot hash identical when switching backends with `--no-gpu`
- Zero visual artifacts in both rendering modes
- 100% feature parity between GPU and ANSI backends

## Testing Requirements
1. Performance benchmarks for both backends (`bench_scroll --lines 1000000`)
2. Visual regression tests comparing backend outputs
3. Driver compatibility testing across GPU vendors
4. SSH/remote usage scenario testing
5. Backend switching stress tests
6. Memory usage validation under sustained load
7. Golden reference tests to prevent rendering divergence
