# Command Prefix Parser PRD

## Overview
The Command Prefix Parser detects and processes the agent invocation prefix ('p' by default) at the beginning of command lines, extracting prompts and model specifications for the AI agent.

## Dependencies
- Input/Keymap System (for receiving typed commands)
- Model Host (for passing parsed commands)
- Config System (for prefix customization)

## Functional Requirements

### 1. Prefix Detection
- **FR-1.1**: Detect 'p' character at line start as agent trigger
- **FR-1.2**: Support configurable prefix character
- **FR-1.3**: Detect model override syntax: 'p --model <id>'
- **FR-1.4**: Only trigger at actual line start (not after whitespace)

### 2. Prompt Extraction
- **FR-2.1**: Collect entire line after prefix as prompt
- **FR-2.2**: Preserve whitespace and special characters in prompt
- **FR-2.3**: Support multi-line prompts with line continuation
- **FR-2.4**: Handle quoted strings properly

### 3. Context Collection
- **FR-3.1**: Capture current stdin state
- **FR-3.2**: Collect configurable amount of scrollback history
- **FR-3.3**: Include environment variables if configured
- **FR-3.4**: Add current working directory to context

### 4. Command Routing
- **FR-4.1**: Route parsed commands to appropriate model
- **FR-4.2**: Pass through non-prefix commands unchanged
- **FR-4.3**: Support command cancellation (Ctrl+C)
- **FR-4.4**: Handle prefix escape sequences

## Non-Functional Requirements

### Performance
- **NFR-1.1**: O(1) prefix detection
- **NFR-1.2**: < 1μs parsing overhead for non-prefix commands
- **NFR-1.3**: No perceptible delay in command execution

### Reliability
- **NFR-2.1**: Zero false positives for prefix detection
- **NFR-2.2**: Graceful handling of malformed commands
- **NFR-2.3**: Thread-safe parsing operations

## Pre-conditions
- Terminal in appropriate input mode
- Command line available for parsing
- Model host initialized and ready

## Post-conditions
- Command correctly identified as agent or regular
- Prompt and parameters extracted if agent command
- Context prepared for model invocation
- Regular commands passed through unmodified

## Edge Cases
1. **Prefix in middle of line**: Should not trigger
2. **Escaped prefix**: Handle backslash or other escape methods
3. **Empty prompt**: Provide helpful error message
4. **Invalid model specification**: Fall back to default model
5. **Prefix in heredoc or string**: Should not trigger
6. **Rapid prefix typing/deletion**: Maintain correct state

## Success Metrics
- 100% accuracy in prefix detection
- Zero false triggers in 10,000 commands
- < 1ms total parsing time
- Successful handling of 20+ edge cases

## Testing Requirements
1. Unit tests for all parsing scenarios
2. Fuzzing tests for malformed input
3. Performance benchmarks for parsing speed
4. Integration tests with various shells
5. Edge case validation suite
6. Concurrency stress tests