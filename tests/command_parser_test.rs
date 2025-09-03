use ferroterm::command_parser::*;

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