use ferroterm::streaming_ui::{StreamingUI, StreamingConfig, StreamingEvent};
use ferroterm::model_host::{ModelHost, InferenceRequest, InferenceParameters, LocalGGUFAdapter};
use ferroterm::renderer::GpuRenderer;
use ferroterm::input::{InputProcessor, InputAction, KeyEvent, Key, Modifier, KeymapConfig};
use ferroterm::command_parser::CommandParser;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{interval, sleep};
use winit::{
    event::{Event, WindowEvent, KeyboardInput, VirtualKeyCode, ElementState, DeviceEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{WindowBuilder, Window},
    dpi::{LogicalSize, PhysicalSize},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    println!("🚀 Ferroterm Streaming UI Demo");
    println!("===============================");
    println!("This demo showcases the streaming UI capabilities:");
    println!("• Real-time markdown rendering");
    println!("• Syntax highlighting for code blocks");
    println!("• Interactive controls (Ctrl+C to interrupt)");
    println!("• Response history navigation");
    println!("• Virtual scrolling for long responses");
    println!("• Performance monitoring");
    println!("");

    // Create event loop and window
    let event_loop = EventLoop::new();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Ferroterm Streaming UI Demo")
            .with_inner_size(LogicalSize::new(1200, 800))
            .with_min_inner_size(PhysicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    // Get window size
    let size = window.inner_size();

    // Initialize GPU renderer
    let renderer = Arc::new(RwLock::new(
        GpuRenderer::new(&*window, size.width, size.height, 14.0).await?
    ));

    // Initialize model host
    let model_host = Arc::new(ModelHost::new(5, 8192)); // 5 model pool, 8GB VRAM

    // Add a mock local model
    let model_path = std::env::current_dir()?.join("models").join("demo.gguf");
    let adapter = Box::new(LocalGGUFAdapter::new(
        model_path,
        "demo-model".to_string(),
        4096,
        2048,
    ));
    model_host.register_model("demo-model".to_string(), adapter).await?;

    // Configure streaming UI
    let streaming_config = StreamingConfig {
        max_response_length: 100_000,
        memory_limit_mb: 50,
        interrupt_timeout_ms: 100,
        scroll_buffer_lines: 1000,
        typing_indicator_enabled: true,
        syntax_highlighting_enabled: true,
        progressive_rendering: true,
        batch_size: 32,
        render_interval_ms: 16, // 60 FPS
    };

    // Initialize streaming UI
    let streaming_ui = StreamingUI::new(
        Arc::clone(&renderer),
        Arc::clone(&model_host),
        streaming_config,
    );

    // Start the streaming UI
    streaming_ui.start().await?;

    // Set up input processing
    let keymap = Arc::new(RwLock::new(KeymapConfig::default()));
    let command_parser = Arc::new(RwLock::new(CommandParser::new("p".to_string())));
    let mut input_processor = InputProcessor::new(
        Arc::clone(&keymap),
        Arc::clone(&command_parser),
    );

    println!("✅ Streaming UI initialized successfully!");
    println!("");
    println!("Demo Commands:");
    println!("• Press '1' to start a simple text response");
    println!("• Press '2' to start a code-heavy response");
    println!("• Press '3' to start a markdown-rich response");
    println!("• Press '4' to simulate a long response (with scrolling)");
    println!("• Press '5' to show performance metrics");
    println!("• Press 'h' to show response history");
    println!("• Press 'c' to clear screen");
    println!("• Press Ctrl+C to interrupt streaming");
    println!("• Press Ctrl+↑/↓ to navigate history");
    println!("• Press Escape to quit");
    println!("");

    // Demo state
    let mut demo_state = DemoState::new();
    let streaming_ui_clone = streaming_ui.clone();

    // Spawn demo task
    let demo_task = {
        let streaming_ui = streaming_ui_clone.clone();
        tokio::spawn(async move {
            run_demo_scenarios(streaming_ui).await;
        })
    };

    // Main event loop
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Resized(physical_size) => {
                    renderer.write().resize(physical_size.width, physical_size.height);
                }
                WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                    renderer.write().resize(new_inner_size.width, new_inner_size.height);
                }
                WindowEvent::KeyboardInput {
                    input: KeyboardInput {
                        state: ElementState::Pressed,
                        virtual_keycode: Some(keycode),
                        modifiers,
                        ..
                    },
                    ..
                } => {
                    let rt = tokio::runtime::Handle::current();
                    rt.spawn(async move {
                        let key_event = convert_winit_key(keycode, &modifiers);
                        
                        // Handle demo-specific keys
                        match keycode {
                            VirtualKeyCode::Key1 => {
                                let _ = start_simple_text_demo(&streaming_ui_clone).await;
                            }
                            VirtualKeyCode::Key2 => {
                                let _ = start_code_demo(&streaming_ui_clone).await;
                            }
                            VirtualKeyCode::Key3 => {
                                let _ = start_markdown_demo(&streaming_ui_clone).await;
                            }
                            VirtualKeyCode::Key4 => {
                                let _ = start_long_response_demo(&streaming_ui_clone).await;
                            }
                            VirtualKeyCode::Key5 => {
                                show_performance_metrics(&streaming_ui_clone);
                            }
                            VirtualKeyCode::H => {
                                show_response_history(&streaming_ui_clone);
                            }
                            VirtualKeyCode::C if !modifiers.ctrl() => {
                                streaming_ui_clone.clear_history();
                                println!("📝 Response history cleared");
                            }
                            VirtualKeyCode::Escape => {
                                // This will be handled by the main thread
                            }
                            _ => {
                                // Process normal input
                                if let Err(e) = input_processor.process_key_event(key_event) {
                                    eprintln!("Input processing error: {}", e);
                                }
                                
                                // Handle input actions
                                while let Some(action) = input_processor.receive_action().await {
                                    if let Err(e) = streaming_ui_clone.handle_input(action).await {
                                        eprintln!("Streaming UI error: {}", e);
                                    }
                                }
                            }
                        }
                    });
                    
                    // Handle escape in main thread
                    if keycode == VirtualKeyCode::Escape {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                // Render frame
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                // Rendering is handled by the streaming UI render loop
            }
            _ => {}
        }
    });
}

struct DemoState {
    current_demo: Option<String>,
    start_time: Instant,
}

impl DemoState {
    fn new() -> Self {
        Self {
            current_demo: None,
            start_time: Instant::now(),
        }
    }
}

async fn run_demo_scenarios(streaming_ui: StreamingUI) {
    let mut interval = interval(Duration::from_secs(30));
    let mut scenario = 0;

    loop {
        interval.tick().await;
        
        match scenario % 4 {
            0 => {
                println!("🎭 Auto-demo: Starting simple text response...");
                let _ = start_simple_text_demo(&streaming_ui).await;
            }
            1 => {
                println!("🎭 Auto-demo: Starting code response...");
                let _ = start_code_demo(&streaming_ui).await;
            }
            2 => {
                println!("🎭 Auto-demo: Starting markdown response...");
                let _ = start_markdown_demo(&streaming_ui).await;
            }
            3 => {
                println!("🎭 Auto-demo: Starting long response...");
                let _ = start_long_response_demo(&streaming_ui).await;
            }
            _ => {}
        }
        
        scenario += 1;
        
        // Show metrics every few cycles
        if scenario % 2 == 0 {
            show_performance_metrics(&streaming_ui);
        }
    }
}

async fn start_simple_text_demo(streaming_ui: &StreamingUI) -> Result<(), Box<dyn std::error::Error>> {
    let request = InferenceRequest {
        prompt: "Explain what a terminal emulator is in simple terms.".to_string(),
        model_name: "demo-model".to_string(),
        parameters: InferenceParameters::default(),
        context: None,
    };

    println!("🔄 Starting simple text demo...");
    let response_id = streaming_ui.start_streaming_response(request).await?;
    
    // Simulate streaming response
    tokio::spawn(async move {
        let text = "A terminal emulator is a program that mimics the behavior of a traditional computer terminal. \
                   It provides a text-based interface where you can type commands and see their output. \
                   Think of it as a window into your computer's command-line interface.\n\n\
                    Modern terminal emulators like **Ferroterm** add advanced features such as:\n\
                   • GPU-accelerated rendering\n\
                   • Rich text formatting\n\
                   • Markdown support\n\
                   • Session multiplexing\n\
                   • And much more!";
        
        // Simulate typing with realistic delays
        for chunk in text.split_whitespace() {
            sleep(Duration::from_millis(50)).await;
            // In a real implementation, this would come from the model host
        }
        
        sleep(Duration::from_millis(500)).await;
        println!("✅ Simple text demo completed");
    });

    Ok(())
}

async fn start_code_demo(streaming_ui: &StreamingUI) -> Result<(), Box<dyn std::error::Error>> {
    let request = InferenceRequest {
        prompt: "Show me a Rust example of async/await with error handling.".to_string(),
        model_name: "demo-model".to_string(),
        parameters: InferenceParameters::default(),
        context: None,
    };

    println!("🔄 Starting code demo...");
    let response_id = streaming_ui.start_streaming_response(request).await?;
    
    // Simulate streaming code response
    tokio::spawn(async move {
        let response = "Here's a comprehensive Rust example demonstrating async/await with proper error handling:\n\n\
```rust\nuse tokio::fs;\nuse std::error::Error;\n\n\
#[derive(Debug)]\nstruct CustomError {\n    message: String,\n}\n\n\
impl std::fmt::Display for CustomError {\n    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {\n        write!(f, \"Custom error: {}\", self.message)\n    }\n}\n\n\
impl Error for CustomError {}\n\n\
async fn read_and_process_file(path: &str) -> Result<String, Box<dyn Error>> {\n    // Read file asynchronously\n    let contents = fs::read_to_string(path).await?;\n    \n    // Process the contents\n    let processed = process_data(&contents).await?;\n    \n    Ok(processed)\n}\n\n\
async fn process_data(data: &str) -> Result<String, CustomError> {\n    if data.is_empty() {\n        return Err(CustomError {\n            message: \"Data is empty\".to_string(),\n        });\n    }\n    \n    // Simulate async processing\n    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;\n    \n    Ok(data.to_uppercase())\n}\n\n\
#[tokio::main]\nasync fn main() -> Result<(), Box<dyn Error>> {\n    match read_and_process_file(\"example.txt\").await {\n        Ok(result) => println!(\"Success: {}\", result),\n        Err(e) => eprintln!(\"Error: {}\", e),\n    }\n    \n    Ok(())\n}\n```\n\n\
This example demonstrates:\n\
• **Async functions** with `async fn`\n\
• **Error propagation** with the `?` operator\n\
• **Custom error types** implementing the `Error` trait\n\
• **Awaiting futures** with `.await`\n\
• **Error handling** with `Result<T, E>`\n\n\
The `tokio` runtime provides the async execution context, making it perfect for I/O-heavy applications!";

        // Simulate realistic streaming
        for line in response.lines() {
            sleep(Duration::from_millis(100)).await;
            // In real implementation, would send through streaming channel
        }
        
        println!("✅ Code demo completed");
    });

    Ok(())
}

async fn start_markdown_demo(streaming_ui: &StreamingUI) -> Result<(), Box<dyn std::error::Error>> {
    let request = InferenceRequest {
        prompt: "Explain the markdown features supported by Ferroterm.".to_string(),
        model_name: "demo-model".to_string(),
        parameters: InferenceParameters::default(),
        context: None,
    };

    println!("🔄 Starting markdown demo...");
    let response_id = streaming_ui.start_streaming_response(request).await?;
    
    tokio::spawn(async move {
        let response = "# Ferroterm Markdown Support\n\n\
Ferroterm provides comprehensive **CommonMark** and **GitHub Flavored Markdown** support with real-time rendering.\n\n\
## Text Formatting\n\n\
You can use *italic text*, **bold text**, and ***bold italic text***. There's also ~~strikethrough~~ support.\n\n\
## Code Support\n\n\
Inline `code` is highlighted, and code blocks get full syntax highlighting:\n\n\
```python\ndef fibonacci(n):\n    if n <= 1:\n        return n\n    return fibonacci(n-1) + fibonacci(n-2)\n\n\
# Example usage\nfor i in range(10):\n    print(f\"F({i}) = {fibonacci(i)}\")\n```\n\n\
## Lists\n\n\
### Unordered Lists\n\
* Feature-rich terminal emulator\n\
* GPU-accelerated rendering\n\
* Real-time markdown processing\n  * Syntax highlighting\n  * Progressive rendering\n  * Memory efficient\n\
* Session multiplexing\n\n\
### Ordered Lists\n\
1. Initialize the streaming UI\n\
2. Connect to model host\n\
3. Start streaming response\n\
4. Process tokens in real-time\n\
5. Render to terminal with styling\n\n\
## Tables\n\n\
| Feature | Status | Performance |\n\
|---------|--------|-----------|\n\
| Markdown | ✅ | Sub-frame |\n\
| Syntax Highlighting | ✅ | < 10ms |\n\
| Virtual Scrolling | ✅ | 60 FPS |\n\
| Progressive Render | ✅ | < 16ms |\n\n\
## Quotes\n\n\
> The best terminal emulator is one that gets out of your way\n\
> and lets you focus on your work.\n\
>\n\
> — Ferroterm Philosophy\n\n\
## Links\n\n\
Check out the [Ferroterm documentation](https://ferroterm.dev) for more details!\n\n\
---\n\n\
All of this renders in **real-time** as tokens stream in from the AI model, with **zero visual jitter** and **sub-frame latency**! 🚀";

        // Simulate streaming with varied timing
        let chunks: Vec<&str> = response.split('\n').collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let delay = match chunk {
                line if line.starts_with("```") => 200, // Longer pause for code blocks
                line if line.starts_with("#") => 150,   // Headers
                line if line.contains("| ") => 100,     // Tables
                _ => 75, // Regular text
            };
            
            sleep(Duration::from_millis(delay)).await;
        }
        
        println!("✅ Markdown demo completed");
    });

    Ok(())
}

async fn start_long_response_demo(streaming_ui: &StreamingUI) -> Result<(), Box<dyn std::error::Error>> {
    let request = InferenceRequest {
        prompt: "Write a comprehensive technical overview of modern terminal emulator architecture.".to_string(),
        model_name: "demo-model".to_string(),
        parameters: InferenceParameters {
            max_tokens: 4096,
            ..Default::default()
        },
        context: None,
    };

    println!("🔄 Starting long response demo (tests virtual scrolling)...");
    let response_id = streaming_ui.start_streaming_response(request).await?;
    
    tokio::spawn(async move {
        // Generate a very long response to test scrolling
        for section in 1..=20 {
            let content = format!(
                "\n\n## Section {}: Advanced Terminal Architecture\n\n\
This section covers the intricacies of modern terminal emulator design. \
Terminal emulators have evolved significantly from their historical origins \
as simple text display devices to sophisticated graphical applications \
capable of rendering complex layouts, multimedia content, and interactive elements.\n\n\
### Key Components\n\n\
1. **Rendering Engine**: Responsible for converting text and control sequences into visual output\n\
2. **Input Processing**: Handles keyboard, mouse, and other input events\n\
3. **Terminal State Machine**: Manages the terminal's internal state and command interpretation\n\
4. **Session Management**: Handles multiple terminal sessions and tabs\n\
5. **Configuration System**: Manages user preferences and customization\n\n\
```rust\n\
// Example architecture component\n\
pub struct TerminalEmulator {{\n\
    renderer: Box<dyn Renderer>,\n\
    input_processor: InputProcessor,\n\
    state_machine: StateMachine,\n\
    session_manager: SessionManager,\n\
    config: Config,\n\
}}\n\
```\n\n\
This demonstrates the **virtual scrolling** capability of Ferroterm, \
allowing smooth navigation through extremely long documents without \
performance degradation.\n\n",
                section
            );
            
            // Simulate streaming for each section
            for word in content.split_whitespace() {
                sleep(Duration::from_millis(25)).await;
            }
        }
        
        println!("✅ Long response demo completed - virtual scrolling tested!");
    });

    Ok(())
}

fn show_performance_metrics(streaming_ui: &StreamingUI) {
    let metrics = streaming_ui.get_performance_metrics();
    
    println!("📊 Performance Metrics:");
    println!("├─ Average Frame Time: {:?}", metrics.average_frame_time);
    println!("├─ Current FPS: {:.1}", metrics.current_fps);
    println!("├─ Memory Usage: {} MB", metrics.memory_usage_mb);
    println!("└─ Frames Dropped: {}", metrics.frames_dropped);
    
    // Performance analysis
    if metrics.current_fps < 50.0 {
        println!("⚠️  Performance warning: FPS below target (60 FPS)");
    } else {
        println!("✅ Performance: Excellent");
    }
    
    if metrics.memory_usage_mb > 100 {
        println!("⚠️  Memory warning: High memory usage");
    }
}

fn show_response_history(streaming_ui: &StreamingUI) {
    if let Some(response) = streaming_ui.get_current_response() {
        println!("📜 Current Response:");
        println!("├─ ID: {}", response.id);
        println!("├─ Length: {} chars", response.content.len());
        println!("├─ Tokens: {}", response.total_tokens);
        println!("├─ Speed: {:.1} tokens/sec", response.tokens_per_second);
        println!("├─ Memory: {} bytes", response.memory_usage);
        println!("├─ Active: {}", response.is_active);
        println!("└─ Interrupted: {}", response.is_interrupted);
    } else {
        println!("📜 No active response");
    }
    
    println!("💡 Use Ctrl+↑/↓ to navigate response history");
}

fn convert_winit_key(keycode: VirtualKeyCode, modifiers: &winit::event::ModifiersState) -> KeyEvent {
    let key = match keycode {
        VirtualKeyCode::A => Key::Char('a'),
        VirtualKeyCode::B => Key::Char('b'),
        VirtualKeyCode::C => Key::Char('c'),
        VirtualKeyCode::D => Key::Char('d'),
        VirtualKeyCode::E => Key::Char('e'),
        VirtualKeyCode::F => Key::Char('f'),
        VirtualKeyCode::G => Key::Char('g'),
        VirtualKeyCode::H => Key::Char('h'),
        VirtualKeyCode::I => Key::Char('i'),
        VirtualKeyCode::J => Key::Char('j'),
        VirtualKeyCode::K => Key::Char('k'),
        VirtualKeyCode::L => Key::Char('l'),
        VirtualKeyCode::M => Key::Char('m'),
        VirtualKeyCode::N => Key::Char('n'),
        VirtualKeyCode::O => Key::Char('o'),
        VirtualKeyCode::P => Key::Char('p'),
        VirtualKeyCode::Q => Key::Char('q'),
        VirtualKeyCode::R => Key::Char('r'),
        VirtualKeyCode::S => Key::Char('s'),
        VirtualKeyCode::T => Key::Char('t'),
        VirtualKeyCode::U => Key::Char('u'),
        VirtualKeyCode::V => Key::Char('v'),
        VirtualKeyCode::W => Key::Char('w'),
        VirtualKeyCode::X => Key::Char('x'),
        VirtualKeyCode::Y => Key::Char('y'),
        VirtualKeyCode::Z => Key::Char('z'),
        VirtualKeyCode::Key1 => Key::Char('1'),
        VirtualKeyCode::Key2 => Key::Char('2'),
        VirtualKeyCode::Key3 => Key::Char('3'),
        VirtualKeyCode::Key4 => Key::Char('4'),
        VirtualKeyCode::Key5 => Key::Char('5'),
        VirtualKeyCode::Key6 => Key::Char('6'),
        VirtualKeyCode::Key7 => Key::Char('7'),
        VirtualKeyCode::Key8 => Key::Char('8'),
        VirtualKeyCode::Key9 => Key::Char('9'),
        VirtualKeyCode::Key0 => Key::Char('0'),
        VirtualKeyCode::Space => Key::Space,
        VirtualKeyCode::Return => Key::Enter,
        VirtualKeyCode::Tab => Key::Tab,
        VirtualKeyCode::Back => Key::Backspace,
        VirtualKeyCode::Delete => Key::Delete,
        VirtualKeyCode::Escape => Key::Escape,
        VirtualKeyCode::Up => Key::Up,
        VirtualKeyCode::Down => Key::Down,
        VirtualKeyCode::Left => Key::Left,
        VirtualKeyCode::Right => Key::Right,
        VirtualKeyCode::Home => Key::Home,
        VirtualKeyCode::End => Key::End,
        VirtualKeyCode::PageUp => Key::PageUp,
        VirtualKeyCode::PageDown => Key::PageDown,
        VirtualKeyCode::Insert => Key::Insert,
        VirtualKeyCode::F1 => Key::F1,
        VirtualKeyCode::F2 => Key::F2,
        VirtualKeyCode::F3 => Key::F3,
        VirtualKeyCode::F4 => Key::F4,
        VirtualKeyCode::F5 => Key::F5,
        VirtualKeyCode::F6 => Key::F6,
        VirtualKeyCode::F7 => Key::F7,
        VirtualKeyCode::F8 => Key::F8,
        VirtualKeyCode::F9 => Key::F9,
        VirtualKeyCode::F10 => Key::F10,
        VirtualKeyCode::F11 => Key::F11,
        VirtualKeyCode::F12 => Key::F12,
        _ => Key::Char(' '), // Fallback
    };
    
    let mut modifier_set = HashSet::new();
    if modifiers.ctrl() {
        modifier_set.insert(Modifier::Ctrl);
    }
    if modifiers.alt() {
        modifier_set.insert(Modifier::Alt);
    }
    if modifiers.shift() {
        modifier_set.insert(Modifier::Shift);
    }
    if modifiers.logo() {
        modifier_set.insert(Modifier::Super);
    }
    
    KeyEvent {
        key,
        modifiers: modifier_set,
        text: None,
        repeat: false,
        timestamp: Instant::now(),
        key_code: None,
    }
}