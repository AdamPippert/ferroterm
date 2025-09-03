use ferroterm::{
    config::ConfigManager,
    input::{Key, KeyEvent},
    tty::{PtyConfig, TtyEngine},
};

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use winit::{
    event::{ElementState, KeyEvent as WinitKeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key as WinitKey, NamedKey},
    window::{Window, WindowBuilder},
};



// Application state
struct FerrotermApp {
    window: Option<Arc<Window>>,
    tty_engine: Arc<TtyEngine>,
    config_manager: Arc<ConfigManager>,
    main_pty_id: Option<u64>,
    is_initialized: bool,
    startup_time: Instant,
    frame_count: u64,
    last_fps_time: Instant,
}

impl FerrotermApp {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let startup_time = Instant::now();
        info!("Starting Ferroterm terminal emulator...");

        // 1. Initialize configuration system first
        info!("Initializing configuration system...");
        let config_manager = Arc::new(ConfigManager::new()?);

        // 2. Initialize TTY Engine
        info!("Initializing TTY Engine...");
        let tty_engine = Arc::new(TtyEngine::new());

        Ok(Self {
            window: None,
            tty_engine,
            config_manager,
            main_pty_id: None,
            is_initialized: false,
            startup_time,
            frame_count: 0,
            last_fps_time: startup_time,
        })
    }

    async fn initialize_graphics(&mut self, window: Arc<Window>) -> Result<(), Box<dyn std::error::Error>> {
        info!("Initializing graphics subsystem...");
        
        let config = self.config_manager.get_config();
        let window_size = window.inner_size();
        
        // Calculate terminal grid dimensions from font size
        let font_size = config.ui.font_size as f32;
        let estimated_char_width = font_size * 0.6; // Rough monospace character width
        let estimated_char_height = font_size * 1.2 * config.ui.line_height;
        
        let term_cols = (window_size.width as f32 / estimated_char_width) as u32;
        let term_rows = (window_size.height as f32 / estimated_char_height) as u32;
        
        info!("Terminal grid: {}x{} ({}x{} pixels)", term_cols, term_rows, window_size.width, window_size.height);

        // 6. TODO: Initialize renderer later when modules are fixed
        info!("Renderer initialization skipped (modules need fixing)");

        // 7. Create main PTY session
        info!("Creating main PTY session...");
        let mut pty_config = PtyConfig::default();
        pty_config.rows = term_rows as u16;
        pty_config.cols = term_cols as u16;
        
        if let Ok(shell) = std::env::var("SHELL") {
            pty_config.shell = shell;
        }
        
        self.main_pty_id = Some(self.tty_engine.create_pty(pty_config).await?);
        
        // Store window reference
        self.window = Some(window);
        self.is_initialized = true;

        let elapsed = self.startup_time.elapsed();
        info!("Ferroterm initialized successfully in {:?}", elapsed);
        
        // Check if we met the startup time target
        if elapsed <= Duration::from_millis(100) {
            info!("✓ Met startup time target (≤100ms)");
        } else {
            warn!("⚠ Startup time exceeded target: {:?} > 100ms", elapsed);
        }

        Ok(())
    }

    fn handle_window_resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if let Some(pty_id) = self.main_pty_id {
            let config = self.config_manager.get_config();
            let font_size = config.ui.font_size as f32;
            let estimated_char_width = font_size * 0.6;
            let estimated_char_height = font_size * 1.2 * config.ui.line_height;
            
            let term_cols = (new_size.width as f32 / estimated_char_width) as u32;
            let term_rows = (new_size.height as f32 / estimated_char_height) as u32;
            
            debug!("Resizing terminal: {}x{}", term_cols, term_rows);
            
            // Resize PTY
            if let Err(e) = self.tty_engine.resize_pty(pty_id, term_rows as u16, term_cols as u16) {
                error!("Failed to resize PTY: {}", e);
            }
        }
    }

    fn handle_key_input(&mut self, key_event: WinitKeyEvent) {
        if !self.is_initialized {
            return;
        }

        // Convert winit key event to our internal format
        let our_key_event = match self.convert_key_event(key_event) {
            Some(event) => event,
            None => return, // Ignore unsupported keys
        };

        // Process input through input processor
        if let Some(pty_id) = self.main_pty_id {
            // For now, convert key to simple string and send to PTY
            // TODO: Implement proper input processing with the InputProcessor
            let key_str = self.key_event_to_string(our_key_event);
            if !key_str.is_empty() {
                self.send_to_pty(pty_id, key_str.as_bytes());
            }
        }
    }

    fn convert_key_event(&self, winit_event: WinitKeyEvent) -> Option<KeyEvent> {
        if winit_event.state != ElementState::Pressed {
            return None; // Only handle key press events
        }

        let key = match winit_event.logical_key {
            WinitKey::Character(ref s) => {
                if let Some(c) = s.chars().next() {
                    Key::Char(c)
                } else {
                    return None;
                }
            }
            WinitKey::Named(NamedKey::Enter) => Key::Enter,
            WinitKey::Named(NamedKey::Tab) => Key::Tab,
            WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
            WinitKey::Named(NamedKey::Delete) => Key::Delete,
            WinitKey::Named(NamedKey::Escape) => Key::Escape,
            WinitKey::Named(NamedKey::ArrowUp) => Key::Up,
            WinitKey::Named(NamedKey::ArrowDown) => Key::Down,
            WinitKey::Named(NamedKey::ArrowLeft) => Key::Left,
            WinitKey::Named(NamedKey::ArrowRight) => Key::Right,
            WinitKey::Named(NamedKey::Home) => Key::Home,
            WinitKey::Named(NamedKey::End) => Key::End,
            WinitKey::Named(NamedKey::PageUp) => Key::PageUp,
            WinitKey::Named(NamedKey::PageDown) => Key::PageDown,
            WinitKey::Named(NamedKey::F1) => Key::F1,
            WinitKey::Named(NamedKey::F2) => Key::F2,
            WinitKey::Named(NamedKey::F3) => Key::F3,
            WinitKey::Named(NamedKey::F4) => Key::F4,
            WinitKey::Named(NamedKey::F5) => Key::F5,
            WinitKey::Named(NamedKey::F6) => Key::F6,
            WinitKey::Named(NamedKey::F7) => Key::F7,
            WinitKey::Named(NamedKey::F8) => Key::F8,
            WinitKey::Named(NamedKey::F9) => Key::F9,
            WinitKey::Named(NamedKey::F10) => Key::F10,
            WinitKey::Named(NamedKey::F11) => Key::F11,
            WinitKey::Named(NamedKey::F12) => Key::F12,
            WinitKey::Named(NamedKey::Insert) => Key::Insert,
            _ => return None, // Ignore other keys
        };

        let modifiers = HashSet::new(); // TODO: Extract modifiers from winit event

        Some(KeyEvent {
            key,
            modifiers,
            text: None,
            repeat: winit_event.repeat,
            timestamp: Instant::now(),
            key_code: None,
        })
    }

    fn key_event_to_string(&self, key_event: KeyEvent) -> String {
        match key_event.key {
            Key::Char(c) => c.to_string(),
            Key::Enter => "\r".to_string(),
            Key::Tab => "\t".to_string(),
            Key::Backspace => "\x08".to_string(),
            Key::Delete => "\x7f".to_string(),
            Key::Escape => "\x1b".to_string(),
            Key::Up => "\x1b[A".to_string(),
            Key::Down => "\x1b[B".to_string(),
            Key::Right => "\x1b[C".to_string(),
            Key::Left => "\x1b[D".to_string(),
            Key::Home => "\x1b[H".to_string(),
            Key::End => "\x1b[F".to_string(),
            Key::PageUp => "\x1b[5~".to_string(),
            Key::PageDown => "\x1b[6~".to_string(),
            _ => String::new(), // Ignore other keys for now
        }
    }

    fn send_to_pty(&self, pty_id: u64, data: &[u8]) {
        let tty_engine = self.tty_engine.clone();
        let data = data.to_vec();
        tokio::spawn(async move {
            if let Err(e) = tty_engine.write_to_pty(pty_id, &data).await {
                error!("Failed to write to PTY: {}", e);
            }
        });
    }

    fn render_frame(&mut self) {
        // TODO: Implement rendering when renderer modules are fixed
        self.frame_count += 1;
        
        // Calculate FPS every second
        let now = Instant::now();
        if now.duration_since(self.last_fps_time) >= Duration::from_secs(1) {
            let fps = self.frame_count;
            if fps < 60 {
                debug!("FPS: {}", fps);
            }
            self.frame_count = 0;
            self.last_fps_time = now;
        }
    }

    fn handle_pty_output(&mut self) {
        if let Some(pty_id) = self.main_pty_id {
            let tty_engine = self.tty_engine.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                match tty_engine.read_from_pty(pty_id, &mut buffer).await {
                    Ok(bytes_read) if bytes_read > 0 => {
                        let output = &buffer[..bytes_read];
                        // TODO: Process terminal escape sequences and update renderer grid
                        debug!("PTY output: {} bytes", bytes_read);
                        // For now, just print to stdout as a basic fallback
                        print!("{}", String::from_utf8_lossy(output));
                    }
                    Ok(_) => {
                        // No data available
                    }
                    Err(e) => {
                        error!("Failed to read from PTY: {}", e);
                    }
                }
            });
        }
    }

    async fn shutdown(&mut self) {
        info!("Shutting down Ferroterm...");
        
        // Close PTY session
        if let Some(pty_id) = self.main_pty_id {
            if let Err(e) = self.tty_engine.destroy_pty(pty_id).await {
                error!("Failed to destroy PTY: {}", e);
            }
        }
        
        // Additional cleanup would go here
        info!("Shutdown complete");
    }
}



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Ferroterm v{} starting...", env!("CARGO_PKG_VERSION"));

    // Create application
    let mut app = FerrotermApp::new()?;

    // Create window before event loop
    let config = app.config_manager.get_config();

    // Calculate window size from terminal dimensions
    let font_size = config.ui.font_size as f32;
    let estimated_char_width = font_size * 0.6;
    let estimated_char_height = font_size * 1.2 * config.ui.line_height;

    let window_width = (config.ui.window_width as f32 * estimated_char_width) as u32;
    let window_height = (config.ui.window_height as f32 * estimated_char_height) as u32;

    let window_attributes = WindowBuilder::new()
        .with_title("Ferroterm")
        .with_inner_size(winit::dpi::LogicalSize::new(window_width, window_height))
        .with_min_inner_size(winit::dpi::LogicalSize::new(400, 200));

    // Create event loop
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let window = Arc::new(window_attributes.build(&event_loop)?);

    // Store window reference in app
    app.window = Some(window.clone());

    // Initialize graphics synchronously for now
    // TODO: Make initialize_graphics synchronous or handle async properly
    info!("Graphics initialization skipped for now (async issues in event loop context)");

    // Run event loop with closure-based event handling
    info!("Starting main event loop...");
    event_loop.run(move |event, event_loop| {
        match event {
            winit::event::Event::Resumed => {
                // Window is already created
            }
            winit::event::Event::WindowEvent { event, .. } => {
                match event {
                    WindowEvent::CloseRequested => {
                        info!("Close requested");
                        // TODO: Handle shutdown properly in event loop context
                        info!("Shutdown skipped for now (async issues in event loop context)");
                        event_loop.exit();
                    }
                    WindowEvent::Resized(new_size) => {
                        debug!("Window resized to {:?}", new_size);
                        app.handle_window_resize(new_size);
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        app.handle_key_input(event);
                    }
                    WindowEvent::RedrawRequested => {
                        app.render_frame();
                        app.handle_pty_output();

                        // Request next frame for continuous rendering
                        if let Some(window) = &app.window {
                            window.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
            winit::event::Event::DeviceEvent { .. } => {
                // Handle device events if needed
            }
            winit::event::Event::AboutToWait => {
                // Handle periodic tasks
                if app.is_initialized {
                    if let Some(window) = &app.window {
                        window.request_redraw();
                    }
                }
            }
            _ => {}
        }
    })?;

    Ok(())
}