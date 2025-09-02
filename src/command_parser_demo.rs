// Standalone demo of the AI command prefix parser
// This shows the key functionality without dependencies on other modules

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use thiserror::Error;
use serde::{Deserialize, Serialize};

#[derive(Error, Debug)]
pub enum CommandParseError {
    #[error("Invalid command syntax: {0}")]
    Syntax(String),
    #[error("Unknown command: {0}")]
    UnknownCommand(String),
    #[error("Missing required argument: {0}")]
    MissingArgument(String),
    #[error("Invalid argument value: {0}")]
    InvalidArgument(String),
    #[error("Context collection error: {0}")]
    Context(String),
    #[error("Model specification error: {0}")]
    Model(String),
    #[error("Quote parsing error: {0}")]
    Quote(String),
}

#[derive(Debug, Clone)]
pub enum Command {
    // AI Agent command
    Agent(AgentCommand),
    // Pass-through for regular terminal commands
    Terminal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommand {
    pub prompt: String,
    pub model_override: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub context: AgentContext,
    pub is_continuation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    pub scrollback_lines: Vec<String>,
    pub environment_vars: HashMap<String, String>,
    pub current_directory: PathBuf,
    pub shell_state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub command: Command,
    pub raw_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParseState {
    LineStart,
    PrefixDetected,
    ParsingArgs,
    CollectingPrompt,
    InQuote(char),
    Escaped,
    Continuation,
}

pub struct CommandParser {
    prefix: String,
    escape_sequence: String,
    state: ParseState,
    context_lines: u32,
    include_env: bool,
    current_buffer: String,
    quote_char: Option<char>,
    continuation_buffer: String,
    pub scrollback: Vec<String>,
}

impl CommandParser {
    pub fn new(prefix: String) -> Self {
        Self {
            escape_sequence: format!("\\{}", prefix),
            prefix,
            state: ParseState::LineStart,
            context_lines: 100,
            include_env: true,
            current_buffer: String::new(),
            quote_char: None,
            continuation_buffer: String::new(),
            scrollback: Vec::new(),
        }
    }

    pub fn with_config(prefix: String, context_lines: u32, include_env: bool) -> Self {
        let mut parser = Self::new(prefix);
        parser.context_lines = context_lines;
        parser.include_env = include_env;
        parser
    }

    /// Parse a complete line of input
    pub fn parse(&mut self, input: &str) -> Result<ParsedCommand, CommandParseError> {
        // Fast path: O(1) prefix detection
        if !self.is_agent_prefix(input) {
            // Pass through regular terminal commands unchanged
            return Ok(ParsedCommand {
                command: Command::Terminal(input.to_string()),
                raw_input: input.to_string(),
            });
        }

        // Parse agent command
        self.parse_agent_command(input)
    }

    /// O(1) prefix detection - only checks line start
    #[inline]
    pub fn is_agent_prefix(&self, input: &str) -> bool {
        if input.is_empty() {
            return false;
        }

        // Check for escape sequence first
        if input.starts_with(&self.escape_sequence) {
            return false;
        }

        // Check exact prefix match at line start
        if let Some(first_char) = input.chars().next() {
            if self.prefix.len() == 1 {
                first_char == self.prefix.chars().next().unwrap_or('p')
            } else {
                input.starts_with(&self.prefix)
            }
        } else {
            false
        }
    }

    /// Parse agent command with full syntax support
    fn parse_agent_command(&mut self, input: &str) -> Result<ParsedCommand, CommandParseError> {
        let input = input.trim_start();
        
        // Handle escape sequence for literal prefix
        if input.starts_with(&self.escape_sequence) {
            let literal_content = &input[self.escape_sequence.len()..];
            return Ok(ParsedCommand {
                command: Command::Terminal(format!("{}{}", self.prefix, literal_content)),
                raw_input: input.to_string(),
            });
        }

        // Remove prefix
        let remaining = if self.prefix.len() == 1 {
            &input[1..]
        } else {
            input.strip_prefix(&self.prefix)
                .ok_or_else(|| CommandParseError::Syntax("Invalid prefix".to_string()))?
        };

        // Parse command line using state machine
        let mut parser = AgentCommandParser::new(remaining.trim());
        let (model_override, temperature, max_tokens, prompt) = parser.parse()?;

        // Collect context
        let context = self.collect_context()?;

        let agent_command = AgentCommand {
            prompt,
            model_override,
            temperature,
            max_tokens,
            context,
            is_continuation: !self.continuation_buffer.is_empty(),
        };

        Ok(ParsedCommand {
            command: Command::Agent(agent_command),
            raw_input: input.to_string(),
        })
    }

    /// Collect terminal context for the AI agent
    pub fn collect_context(&self) -> Result<AgentContext, CommandParseError> {
        let mut context = AgentContext {
            scrollback_lines: Vec::new(),
            environment_vars: HashMap::new(),
            current_directory: env::current_dir()
                .map_err(|e| CommandParseError::Context(format!("Failed to get current directory: {}", e)))?,
            shell_state: None,
        };

        // Collect scrollback history
        let start = self.scrollback.len().saturating_sub(self.context_lines as usize);
        context.scrollback_lines = self.scrollback[start..].to_vec();

        // Collect relevant environment variables if enabled
        if self.include_env {
            let relevant_vars = [
                "PATH", "HOME", "USER", "PWD", "SHELL", "TERM", "LANG", "LC_ALL",
                "EDITOR", "PAGER", "PS1", "HOSTNAME", "DISPLAY", "XDG_SESSION_TYPE",
            ];

            for var in &relevant_vars {
                if let Ok(value) = env::var(var) {
                    context.environment_vars.insert(var.to_string(), value);
                }
            }
        }

        Ok(context)
    }

    /// Update scrollback buffer with new terminal output
    pub fn update_scrollback(&mut self, lines: Vec<String>) {
        self.scrollback.extend(lines);
        // Keep only recent history for memory efficiency
        if self.scrollback.len() > (self.context_lines as usize * 2) {
            let start = self.scrollback.len() - (self.context_lines as usize * 2);
            self.scrollback.drain(..start);
        }
    }

    /// Handle multi-line prompt continuation
    pub fn add_continuation(&mut self, line: String) -> bool {
        if !self.continuation_buffer.is_empty() || line.ends_with('\\') {
            if line.ends_with('\\') {
                self.continuation_buffer.push_str(&line[..line.len() - 1]);
                self.continuation_buffer.push('\n');
                true
            } else {
                self.continuation_buffer.push_str(&line);
                false
            }
        } else {
            false
        }
    }

    /// Get accumulated continuation buffer
    pub fn get_continuation(&mut self) -> String {
        std::mem::take(&mut self.continuation_buffer)
    }

    /// Cancel current command parsing state
    pub fn cancel_command(&mut self) {
        self.state = ParseState::LineStart;
        self.current_buffer.clear();
        self.continuation_buffer.clear();
        self.quote_char = None;
    }

    pub fn update_prefix(&mut self, new_prefix: String) {
        self.prefix = new_prefix.clone();
        self.escape_sequence = format!("\\{}", new_prefix);
    }

    pub fn get_prefix(&self) -> &str {
        &self.prefix
    }
}

/// Specialized parser for agent command syntax
struct AgentCommandParser {
    input: String,
    pos: usize,
    state: ParseState,
}

impl AgentCommandParser {
    fn new(input: &str) -> Self {
        Self {
            input: input.to_string(),
            pos: 0,
            state: ParseState::LineStart,
        }
    }

    fn parse(&mut self) -> Result<(Option<String>, Option<f32>, Option<u32>, String), CommandParseError> {
        let mut model_override = None;
        let mut temperature = None;
        let mut max_tokens = None;
        let mut prompt_parts = Vec::new();

        while self.pos < self.input.len() {
            self.skip_whitespace();
            
            if self.pos >= self.input.len() {
                break;
            }

            if self.peek() == Some('-') && self.peek_next() == Some('-') {
                // Parse argument
                let (arg_name, arg_value) = self.parse_argument()?;
                match arg_name.as_str() {
                    "model" => {
                        model_override = Some(arg_value.ok_or_else(|| {
                            CommandParseError::MissingArgument("model name".to_string())
                        })?);
                    }
                    "temp" | "temperature" => {
                        let temp_str = arg_value.ok_or_else(|| {
                            CommandParseError::MissingArgument("temperature value".to_string())
                        })?;
                        temperature = Some(temp_str.parse::<f32>().map_err(|_| {
                            CommandParseError::InvalidArgument(format!("Invalid temperature: {}", temp_str))
                        })?);
                        
                        // Validate temperature range
                        if let Some(temp) = temperature {
                            if temp < 0.0 || temp > 2.0 {
                                return Err(CommandParseError::InvalidArgument(
                                    "Temperature must be between 0.0 and 2.0".to_string()
                                ));
                            }
                        }
                    }
                    "max-tokens" | "tokens" => {
                        let tokens_str = arg_value.ok_or_else(|| {
                            CommandParseError::MissingArgument("max tokens value".to_string())
                        })?;
                        max_tokens = Some(tokens_str.parse::<u32>().map_err(|_| {
                            CommandParseError::InvalidArgument(format!("Invalid max tokens: {}", tokens_str))
                        })?);
                    }
                    _ => {
                        return Err(CommandParseError::InvalidArgument(
                            format!("Unknown argument: {}", arg_name)
                        ));
                    }
                }
            } else {
                // Collect remaining input as prompt
                prompt_parts.push(self.collect_remaining());
                break;
            }
        }

        let prompt = prompt_parts.join(" ").trim().to_string();
        if prompt.is_empty() {
            return Err(CommandParseError::MissingArgument("prompt text".to_string()));
        }

        Ok((model_override, temperature, max_tokens, prompt))
    }

    fn parse_argument(&mut self) -> Result<(String, Option<String>), CommandParseError> {
        // Skip --
        self.advance();
        self.advance();

        let arg_name = self.collect_until_whitespace_or_equals();
        if arg_name.is_empty() {
            return Err(CommandParseError::Syntax("Empty argument name".to_string()));
        }

        self.skip_whitespace();

        let arg_value = if self.peek() == Some('=') {
            self.advance(); // Skip =
            Some(self.parse_argument_value()?)
        } else if self.pos < self.input.len() && self.peek() != Some('-') {
            Some(self.parse_argument_value()?)
        } else {
            None
        };

        Ok((arg_name, arg_value))
    }

    fn parse_argument_value(&mut self) -> Result<String, CommandParseError> {
        self.skip_whitespace();
        
        if self.peek() == Some('"') || self.peek() == Some('\'') {
            self.parse_quoted_string()
        } else {
            Ok(self.collect_until_whitespace_or_dash())
        }
    }

    fn parse_quoted_string(&mut self) -> Result<String, CommandParseError> {
        let quote_char = self.advance().unwrap();
        let mut result = String::new();
        let mut escaped = false;

        while let Some(ch) = self.advance() {
            if escaped {
                match ch {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    '\'' => result.push('\''),
                    _ => {
                        result.push('\\');
                        result.push(ch);
                    }
                }
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_char {
                return Ok(result);
            } else {
                result.push(ch);
            }
        }

        Err(CommandParseError::Quote(format!("Unterminated quoted string starting with {}", quote_char)))
    }

    fn collect_remaining(&mut self) -> String {
        let remaining = &self.input[self.pos..];
        self.pos = self.input.len();
        remaining.to_string()
    }

    fn collect_until_whitespace_or_equals(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || ch == '=' {
                break;
            }
            self.advance();
        }
        self.input[start..self.pos].to_string()
    }

    fn collect_until_whitespace_or_dash(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || (ch == '-' && self.peek_next() == Some('-')) {
                break;
            }
            self.advance();
        }
        self.input[start..self.pos].to_string()
    }

    fn peek(&self) -> Option<char> {
        self.input.chars().nth(self.pos)
    }

    fn peek_next(&self) -> Option<char> {
        self.input.chars().nth(self.pos + 1)
    }

    fn advance(&mut self) -> Option<char> {
        if let Some(ch) = self.peek() {
            self.pos += ch.len_utf8();
            Some(ch)
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if !ch.is_whitespace() {
                break;
            }
            self.advance();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_o1_prefix_detection() {
        let mut parser = CommandParser::new("f".to_string());
        
        // Should detect prefix
        assert!(parser.is_agent_prefix("f hello world"));
        assert!(parser.is_agent_prefix("f --model gpt-4 test"));
        assert!(parser.is_agent_prefix("f"));
        
        // Should not detect prefix
        assert!(!parser.is_agent_prefix("hello p world"));
        assert!(!parser.is_agent_prefix(" p test"));
        assert!(!parser.is_agent_prefix("\\f escaped"));
        assert!(!parser.is_agent_prefix(""));
    }

    #[test]
    fn test_zero_false_positives() {
        let mut parser = CommandParser::new("f".to_string());
        
        let test_cases = vec![
            "ls -la",
            "cd /home/user", 
            "grep pattern file.txt",
            "echo 'hello world'",
            "python script.py",
            " p test", // Leading space
            "some p command", // Middle
            "\\p escaped", // Escape sequence
            "",
            "pwd",
            "ps aux | grep process",
        ];

        for case in test_cases {
            let result = parser.parse(case).unwrap();
            match result.command {
                Command::Terminal(_) => {}, // Expected
                _ => panic!("False positive detected for: {}", case),
            }
        }
    }

    #[test]
    fn test_agent_command_parsing() {
        let mut parser = CommandParser::new("f".to_string());
        
        let result = parser.parse("p hello world").unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.prompt, "hello world");
                assert!(cmd.model_override.is_none());
                assert!(cmd.temperature.is_none());
            }
            _ => panic!("Expected Agent command"),
        }
    }

    #[test]
    fn test_model_override_syntax() {
        let mut parser = CommandParser::new("f".to_string());
        
        let result = parser.parse("p --model gpt-4 explain rust").unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.prompt, "explain rust");
                assert_eq!(cmd.model_override, Some("gpt-4".to_string()));
            }
            _ => panic!("Expected Agent command"),
        }
    }

    #[test]
    fn test_temperature_parameter() {
        let mut parser = CommandParser::new("f".to_string());
        
        let result = parser.parse("p --temp 0.8 creative story").unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.prompt, "creative story");
                assert_eq!(cmd.temperature, Some(0.8));
            }
            _ => panic!("Expected Agent command"),
        }

        // Test invalid temperature
        let result = parser.parse("p --temp 5.0 test");
        assert!(result.is_err());
    }

    #[test]
    fn test_quoted_strings() {
        let mut parser = CommandParser::new("f".to_string());
        
        let result = parser.parse(r#"p --model "gpt-4" "explain 'nested quotes'""#).unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.model_override, Some("gpt-4".to_string()));
                assert_eq!(cmd.prompt, "explain 'nested quotes'");
            }
            _ => panic!("Expected Agent command"),
        }
    }

    #[test]
    fn test_escape_sequences() {
        let mut parser = CommandParser::new("f".to_string());
        
        let result = parser.parse("\\p literal prefix").unwrap();
        match result.command {
            Command::Terminal(cmd) => {
                assert_eq!(cmd, "p literal prefix");
            }
            _ => panic!("Expected Terminal command for escaped prefix"),
        }
    }

    #[test]
    fn test_edge_cases() {
        let mut parser = CommandParser::new("f".to_string());
        
        // Empty prompt should fail
        let result = parser.parse("p --model gpt-4");
        assert!(result.is_err());
        
        // Only prefix should fail  
        let result = parser.parse("p");
        assert!(result.is_err());
        
        // Unknown argument should fail
        let result = parser.parse("p --unknown-arg value test");
        assert!(result.is_err());
    }

    #[test]
    fn test_context_collection() {
        let parser = CommandParser::new("p".to_string());
        let context = parser.collect_context().unwrap();
        
        // Should have current directory
        assert!(!context.current_directory.as_os_str().is_empty());
        
        // Should have some environment variables if enabled
        if parser.include_env {
            assert!(!context.environment_vars.is_empty());
        }
    }

    #[test]
    fn test_scrollback_management() {
        let mut parser = CommandParser::new("f".to_string());
        
        // Add some scrollback lines
        let lines = vec![
            "line 1".to_string(),
            "line 2".to_string(), 
            "line 3".to_string(),
        ];
        parser.update_scrollback(lines.clone());
        
        assert_eq!(parser.scrollback.len(), 3);
        assert_eq!(parser.scrollback, lines);
        
        // Test memory management with large scrollback
        let large_lines: Vec<String> = (0..300).map(|i| format!("line {}", i)).collect();
        parser.update_scrollback(large_lines);
        
        // Should be limited to prevent memory bloat
        assert!(parser.scrollback.len() <= 200); // 2 * context_lines
    }

    #[test]
    fn test_multi_line_continuation() {
        let mut parser = CommandParser::new("f".to_string());
        
        // Test continuation with backslash
        assert!(parser.add_continuation("first line \\".to_string()));
        assert!(!parser.add_continuation("second line".to_string()));
        
        let continued = parser.get_continuation();
        assert_eq!(continued, "first line \nsecond line");
    }

    #[test]
    fn test_argument_parsing_edge_cases() {
        let mut parser = CommandParser::new("f".to_string());
        
        // Multiple arguments
        let result = parser.parse("p --model gpt-4 --temp 0.5 --tokens 1000 complex prompt").unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.model_override, Some("gpt-4".to_string()));
                assert_eq!(cmd.temperature, Some(0.5));
                assert_eq!(cmd.max_tokens, Some(1000));
                assert_eq!(cmd.prompt, "complex prompt");
            }
            _ => panic!("Expected Agent command"),
        }

        // Equals syntax
        let result = parser.parse("p --model=gpt-4 test prompt").unwrap();
        match result.command {
            Command::Agent(cmd) => {
                assert_eq!(cmd.model_override, Some("gpt-4".to_string()));
            }
            _ => panic!("Expected Agent command"),
        }
    }

    #[test]
    fn test_performance_benchmark() {
        let mut parser = CommandParser::new("f".to_string());
        
        let test_commands = vec![
            "ls -la",
            "p simple prompt", 
            "grep pattern file.txt",
            "p --model gpt-4 complex prompt with args",
            "cd /home/user",
            "p --temp 0.8 --tokens 500 multi-arg prompt",
        ];

        let start = std::time::Instant::now();
        
        for _ in 0..1000 {
            for cmd in &test_commands {
                let _ = parser.parse(cmd);
            }
        }
        
        let elapsed = start.elapsed();
        let avg_per_command = elapsed.as_nanos() / (1000 * test_commands.len() as u128);
        
        // Should be much less than 1μs per command for non-prefix commands
        assert!(avg_per_command < 1000, "Average parsing time too high: {}ns", avg_per_command);
    }

    #[test] 
    fn test_comprehensive_edge_cases() {
        let mut parser = CommandParser::new("f".to_string());
        
        let edge_cases = vec![
            // Heredoc-like structures (should not trigger)
            ("cat << 'EOF'\np content\nEOF", false),
            // String literals (should not trigger)  
            ("echo \"p test\"", false),
            ("echo 'p test'", false),
            // Actual prefixes
            ("p test", true),
            ("p --model gpt test", true),
            // Comments (should not trigger)
            ("# p comment", false),
            // Path-like strings
            ("/path/to/p/file", false),
            // Variable assignments
            ("P=value", false),
            // Rapid typing simulation
            ("f", true), // Would error due to empty prompt, but prefix detected
            ("pp", false),
            ("ppp", false),
        ];

        for (cmd, should_be_agent) in edge_cases {
            let result = parser.parse(cmd);
            if should_be_agent {
                // Should be agent command (might error on validation but prefix detected)
                match result {
                    Ok(ParsedCommand { command: Command::Agent(_), .. }) => {},
                    Err(CommandParseError::MissingArgument(_)) => {}, // Empty prompt case
                    _ => panic!("Expected agent command detection for: {}", cmd),
                }
            } else {
                match result {
                    Ok(ParsedCommand { command: Command::Terminal(_), .. }) => {},
                    _ => panic!("Expected terminal command for: {}", cmd),
                }
            }
        }
    }
}

fn main() {
    println!("AI Command Prefix Parser Demo");
    println!("=============================");

    let mut parser = CommandParser::new("p".to_string());

    let test_commands = vec![
        "ls -la",
        "p hello world",
        "grep pattern file.txt", 
        "p --model gpt-4 explain rust ownership",
        "cd /home/user",
        r#"p --temp 0.8 --tokens 500 "write a haiku""#,
        "\\p literal prefix",
        " p not a prefix",
        "echo 'p inside string'",
        "p --model mistral-7b --temp 0.3 debug this error: segfault",
    ];

    println!("\nTesting prefix detection and command parsing:");
    println!("--------------------------------------------");

    for cmd in test_commands {
        println!("\nInput: {}", cmd);
        match parser.parse(cmd) {
            Ok(parsed) => {
                match parsed.command {
                    Command::Agent(agent_cmd) => {
                        println!("  ✓ Agent command detected");
                        println!("    Prompt: {}", agent_cmd.prompt);
                        if let Some(model) = agent_cmd.model_override {
                            println!("    Model: {}", model);
                        }
                        if let Some(temp) = agent_cmd.temperature {
                            println!("    Temperature: {}", temp);
                        }
                        if let Some(tokens) = agent_cmd.max_tokens {
                            println!("    Max tokens: {}", tokens);
                        }
                        println!("    Context lines: {}", agent_cmd.context.scrollback_lines.len());
                        println!("    Env vars: {}", agent_cmd.context.environment_vars.len());
                    }
                    Command::Terminal(terminal_cmd) => {
                        println!("  → Terminal command: {}", terminal_cmd);
                    }
                }
            }
            Err(e) => {
                println!("  ✗ Parse error: {}", e);
            }
        }
    }

    // Performance benchmark
    println!("\n\nPerformance Benchmark:");
    println!("---------------------");

    let benchmark_commands = vec!["ls -la", "p test", "grep test file.txt"];
    let start = std::time::Instant::now();
    
    for _ in 0..10000 {
        for cmd in &benchmark_commands {
            let _ = parser.parse(cmd);
        }
    }
    
    let elapsed = start.elapsed();
    let total_commands = 10000 * benchmark_commands.len();
    let avg_per_command = elapsed.as_nanos() / total_commands as u128;
    
    println!("Processed {} commands in {:?}", total_commands, elapsed);
    println!("Average time per command: {}ns", avg_per_command);
    println!("Commands per second: {:.0}", 1_000_000_000.0 / avg_per_command as f64);

    // Test edge cases
    println!("\n\nEdge Case Testing:");
    println!("------------------");

    let edge_cases = vec![
        "p",  // Empty prompt
        "p --model",  // Missing model value
        "p --temp 5.0 test",  // Invalid temperature  
        "p --unknown test",  // Unknown argument
        r#"p "unterminated string"#,  // Would be fine - this is terminated
        "p --model=gpt-4 test",  // Equals syntax
    ];

    for case in edge_cases {
        println!("\nTesting: {}", case);
        match parser.parse(case) {
            Ok(parsed) => {
                println!("  ✓ Parsed successfully");
                if let Command::Agent(cmd) = parsed.command {
                    println!("    Prompt: '{}'", cmd.prompt);
                }
            }
            Err(e) => {
                println!("  ✗ Error (expected): {}", e);
            }
        }
    }

    println!("\n\nDemo completed successfully!");
}