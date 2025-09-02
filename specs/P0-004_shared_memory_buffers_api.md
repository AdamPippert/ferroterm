# Shared Memory Buffers API PRD

## Overview
The Shared Memory Buffers API provides zero-copy data sharing between the terminal host and containerized LLM agents. This system eliminates serialization overhead for large scrollback buffers and file diffs, significantly reducing agent response latency.

## Dependencies
- TTY Engine (for generating scrollback data)
- OCI Session Launcher (for container integration)
- Command Prefix Parser (for determining context requirements)
- Model Host (for consuming shared memory data)

## Functional Requirements

### 1. Memory Layout Management
- **FR-1.1**: Implement structured memory layout: [Header][Cmd Ring][File Slice Ring][Agent Scratch]
- **FR-1.2**: Ensure 4 KiB alignment for optimal performance
- **FR-1.3**: Support checksum-based deduplication
- **FR-1.4**: Manage memory segment lifecycle and cleanup

### 2. Ring Buffer Operations
- **FR-2.1**: Store ≥ 10,000 command history entries in command ring
- **FR-2.2**: Store ≥ 2,000 file lines in file slice ring
- **FR-2.3**: Implement circular buffer semantics with overflow handling
- **FR-2.4**: Support concurrent read access from multiple agents

### 3. Memory Mapping
- **FR-3.1**: Create memory-mapped ring buffers using optimal backend
- **FR-3.2**: Provide read-only access to containerized agents
- **FR-3.3**: Support zero-copy data access patterns
- **FR-3.4**: Handle memory mapping failures gracefully

### 4. Fallback Mechanisms
- **FR-4.1**: Fallback to `memfd_create` when `/dev/shm` unavailable
- **FR-4.2**: Support alternative shared memory backends
- **FR-4.3**: Degrade gracefully to serialization if memory sharing fails
- **FR-4.4**: Provide consistent API across backend implementations

### 5. Data Synchronization
- **FR-5.1**: Implement efficient producer-consumer synchronization
- **FR-5.2**: Support atomic updates to shared data structures
- **FR-5.3**: Prevent data corruption from concurrent access
- **FR-5.4**: Handle reader/writer lifecycle coordination

## Non-Functional Requirements

### Performance
- **NFR-1.1**: Buffer access latency < 1 μs
- **NFR-1.2**: Extra RSS per pane ≤ 256 KB
- **NFR-1.3**: Zero-copy data access for all read operations
- **NFR-1.4**: Minimal CPU overhead for synchronization

### Reliability
- **NFR-2.1**: Robust handling of memory allocation failures
- **NFR-2.2**: Graceful degradation when shared memory unavailable
- **NFR-2.3**: Protection against memory corruption
- **NFR-2.4**: Clean resource cleanup on process termination

### Compatibility
- **NFR-3.1**: Support various Linux shared memory backends
- **NFR-3.2**: Compatible with container security constraints
- **NFR-3.3**: Work with different memory allocators
- **NFR-3.4**: Handle platform-specific memory alignment requirements

## Pre-conditions
- Host system with shared memory support (/dev/shm or memfd_create)
- Container runtime configured for shared memory access
- Sufficient system memory for buffer allocation
- Memory mapping libraries (memmap2) available

## Post-conditions
- Shared memory segments created and mapped
- Ring buffers initialized with proper layout
- Read-only access configured for agent containers
- Synchronization mechanisms operational

## Edge Cases
1. **Memory allocation failure**: Fallback to serialization-based communication
2. **Container security restrictions**: Use alternative sharing mechanisms
3. **Memory corruption detection**: Implement checksums and validation
4. **Process crash during write**: Ensure reader protection and recovery
5. **Ring buffer overflow**: Implement proper wraparound and data preservation
6. **Permission denied on shared memory**: Graceful fallback to IPC alternatives

## Success Metrics
- Buffer access latency consistently < 1 μs
- Memory overhead per pane ≤ 256 KB RSS
- Zero data corruption incidents in stress testing
- 100% successful fallback when shared memory unavailable
- Measurable reduction in agent response latency vs. serialization

## Testing Requirements
1. Performance benchmarks for buffer access patterns
2. Memory usage validation under various load conditions
3. Concurrency stress testing with multiple readers/writers
4. Fallback mechanism validation across platforms
5. Memory corruption detection and recovery tests
6. Container security and isolation verification
7. Cross-platform compatibility testing
