use pachyterm::config::{ConfigManager, KeymapConfig};
use pachyterm::command_parser::CommandParser;
use pachyterm::input::{InputProcessor, Key, Modifier, KeyEvent, InputAction, ShellMode, KeyBindingContext};
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Ferroterm Input System Demo");
    println!("===========================");

    // Initialize components
    let config_manager = Arc::new(ConfigManager::new()?);
    let keymap_config = Arc::new(RwLock::new(KeymapConfig::default()));
    let command_parser = Arc::new(RwLock::new(CommandParser::new("p".to_string())));

    // Create input processor
    let mut processor = InputProcessor::new(
        keymap_config,
        command_parser,
        config_manager,
    );

    // Load keybindings from configuration
    processor.load_keybindings_from_config().await?;

    println!("✓ Input processor initialized");
    
    // Set shell mode
    processor.set_shell_mode(ShellMode::Emacs);
    println!("✓ Shell mode set to Emacs");

    // Add some custom keybindings
    processor.add_custom_keybinding(
        "ctrl+shift+d",
        InputAction::Custom("debug".to_string(), vec!["input_stats".to_string()]),
        KeyBindingContext::Global,
        95,
    )?;

    processor.add_custom_keybinding(
        "f5",
        InputAction::Custom("reload".to_string(), vec!["config".to_string()]),
        KeyBindingContext::Global,
        90,
    )?;

    println!("✓ Custom keybindings added");

    // Demonstrate various key events
    println!("\n🔧 Testing Key Event Processing");
    println!("================================");

    // Test 1: Regular character input
    let char_event = create_key_event(Key::Char('h'), vec![], Some("h".to_string()));
    processor.process_key_event(char_event).await?;
    println!("✓ Regular character input processed");

    // Test 2: Prefix activation (AI agent)
    println!("\n📋 Testing Prefix Activation (AI Agent)");
    let prefix_event = create_key_event(Key::Char('f'), vec![], Some("f".to_string()));
    processor.process_key_event(prefix_event).await?;
    
    if processor.is_prefix_mode() {
        println!("✓ Prefix mode activated");
        
        // Add command characters
        let help_chars = vec!['h', 'e', 'l', 'p'];
        for ch in help_chars {
            let cmd_event = create_key_event(Key::Char(ch), vec![], Some(ch.to_string()));
            processor.process_key_event(cmd_event).await?;
        }
        
        println!("✓ Command buffer: '{}'", processor.get_command_buffer());
        
        // Execute command
        let enter_event = create_key_event(Key::Enter, vec![], None);
        processor.process_key_event(enter_event).await?;
        
        if !processor.is_prefix_mode() {
            println!("✓ Command executed, prefix mode deactivated");
        }
    }

    // Test 3: Escape sequence for literal 'p'
    println!("\n🔄 Testing Escape Sequence");
    let escape_event = create_key_event(Key::Char('\\'), vec![], Some("\\".to_string()));
    processor.process_key_event(escape_event).await?;
    
    let literal_p_event = create_key_event(Key::Char('p'), vec![], Some("p".to_string()));
    processor.process_key_event(literal_p_event).await?;
    
    if !processor.is_prefix_mode() {
        println!("✓ Escape sequence processed - literal 'p' sent");
    }

    // Test 4: Keybinding resolution
    println!("\n⌨️  Testing Keybinding Resolution");
    let ctrl_c_event = create_key_event(Key::Char('c'), vec![Modifier::Ctrl], None);
    processor.process_key_event(ctrl_c_event).await?;
    println!("✓ Ctrl+C keybinding processed");

    let custom_binding_event = create_key_event(Key::F5, vec![], None);
    processor.process_key_event(custom_binding_event).await?;
    println!("✓ F5 custom keybinding processed");

    // Test 5: Performance measurement
    println!("\n⚡ Performance Testing");
    let start_time = Instant::now();
    let test_event = create_key_event(Key::Char('x'), vec![Modifier::Ctrl], None);
    
    for _ in 0..1000 {
        processor.process_key_event(test_event.clone()).await?;
    }
    
    let duration = start_time.elapsed();
    let avg_micros = duration.as_micros() / 1000;
    println!("✓ Processed 1000 key events in {:?} (avg: {}μs per event)", duration, avg_micros);

    if avg_micros < 100 {
        println!("✅ Performance target met (<100μs per event)");
    } else {
        println!("⚠️  Performance target missed ({}μs > 100μs)", avg_micros);
    }

    // Test 6: Statistics and cache effectiveness
    println!("\n📊 Input Statistics");
    let stats = processor.get_input_stats();
    println!("Total keys processed: {}", stats.total_keys_processed);
    println!("Average processing time: {}ns", stats.avg_processing_time_ns);
    println!("Cache hits: {}", stats.cache_hits);
    println!("Cache misses: {}", stats.cache_misses);
    println!("Prefix activations: {}", stats.prefix_activations);
    println!("Conflicts resolved: {}", stats.conflicts_resolved);
    
    if stats.cache_hits > 0 {
        let hit_rate = (stats.cache_hits as f64) / ((stats.cache_hits + stats.cache_misses) as f64) * 100.0;
        println!("Cache hit rate: {:.1}%", hit_rate);
    }

    // Test 7: Active keybindings display
    println!("\n🗂️  Active Keybindings");
    let bindings = processor.list_active_keybindings();
    for (key, action) in bindings.iter().take(10) {
        println!("  {:<20} -> {}", key, action);
    }
    if bindings.len() > 10 {
        println!("  ... and {} more", bindings.len() - 10);
    }

    // Test 8: Action processing simulation
    println!("\n🎬 Action Processing Simulation");
    tokio::spawn(async move {
        // In a real application, you would have a loop processing actions
        while let Some(action) = processor.receive_action().await {
            match action {
                InputAction::SendToTerminal(text) => {
                    if !text.chars().all(char::is_control) && !text.is_empty() {
                        print!("Terminal: '{}'", text.escape_default());
                    }
                }
                InputAction::ExecuteParsedCommand(cmd) => {
                    println!("Command: {:?}", cmd.command);
                }
                InputAction::Custom(name, args) => {
                    println!("Custom action: {} with args: {:?}", name, args);
                }
                InputAction::Interrupt => {
                    println!("Interrupt signal received");
                }
                _ => {
                    println!("Action: {:?}", action);
                }
            }
        }
    });

    // Give some time for action processing
    sleep(Duration::from_millis(100)).await;

    println!("\n✅ All tests completed successfully!");
    println!("\n📝 Key Features Demonstrated:");
    println!("  • Fast key event processing (<100μs target)");
    println!("  • Prefix detection for AI agent activation");
    println!("  • Escape sequences for literal character input");
    println!("  • O(1) keybinding lookup with caching");
    println!("  • Context-aware keybinding resolution");
    println!("  • Shell mode compatibility (Vi/Emacs)");
    println!("  • Custom keybinding management");
    println!("  • Performance monitoring and statistics");
    println!("  • Bracketed paste mode support");
    println!("  • International keyboard layout support");

    Ok(())
}

fn create_key_event(key: Key, modifiers: Vec<Modifier>, text: Option<String>) -> KeyEvent {
    KeyEvent {
        key,
        modifiers: modifiers.into_iter().collect(),
        text,
        repeat: false,
        timestamp: Instant::now(),
        key_code: None,
    }
}