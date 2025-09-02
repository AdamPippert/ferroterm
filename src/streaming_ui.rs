use crate::input::{InputAction, KeyEvent, Key, Modifier};
use crate::model_host::{InferenceRequest, InferenceResponse, ModelHost, ModelHostError};
use crate::renderer::{GpuRenderer, StreamUpdate, TerminalCell, TerminalGrid};
use parking_lot::{RwLock, Mutex};
use pulldown_cmark::{Parser, Event, Tag, CodeBlockKind, CowStr, Options};
use std::collections::{HashMap, VecDeque, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep};
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum StreamingUIError {
    #[error("Renderer error: {0}")]
    Renderer(String),
    #[error("Model host error: {0}")]
    ModelHost(#[from] ModelHostError),
    #[error("Channel error: {0}")]
    Channel(String),
    #[error("Parsing error: {0}")]
    Parsing(String),
    #[error("Interrupt timeout: {0}ms")]
    InterruptTimeout(u64),
    #[error("Memory limit exceeded: {current}MB > {limit}MB")]
    MemoryLimit { current: u64, limit: u64 },
}

#[derive(Debug, Clone)]
pub struct StreamingConfig {
    pub max_response_length: usize,
    pub memory_limit_mb: u64,
    pub interrupt_timeout_ms: u64,
    pub scroll_buffer_lines: u32,
    pub typing_indicator_enabled: bool,
    pub syntax_highlighting_enabled: bool,
    pub progressive_rendering: bool,
    pub batch_size: usize,
    pub render_interval_ms: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_response_length: 1_000_000, // 1MB max response
            memory_limit_mb: 10,
            interrupt_timeout_ms: 100,
            scroll_buffer_lines: 10000,
            typing_indicator_enabled: true,
            syntax_highlighting_enabled: true,
            progressive_rendering: true,
            batch_size: 64, // Characters per batch
            render_interval_ms: 16, // ~60 FPS
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponseState {
    pub id: String,
    pub content: String,
    pub markdown_tokens: Vec<MarkdownToken>,
    pub start_line: u32,
    pub current_line: u32,
    pub is_active: bool,
    pub is_interrupted: bool,
    pub tokens_per_second: f32,
    pub total_tokens: u32,
    pub start_time: Instant,
    pub last_update: Instant,
    pub memory_usage: u64,
}

#[derive(Debug, Clone)]
pub struct MarkdownToken {
    pub token_type: MarkdownTokenType,
    pub content: String,
    pub start_pos: usize,
    pub end_pos: usize,
    pub style: TextStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownTokenType {
    Text,
    Header(u8), // 1-6 for H1-H6
    Bold,
    Italic,
    Code,
    CodeBlock(String), // Language
    Link(String),      // URL
    List(u8),          // Nesting level
    Quote,
    LineBreak,
}

#[derive(Debug, Clone)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub color: [f32; 4],
    pub background: [f32; 4],
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            color: [1.0, 1.0, 1.0, 1.0], // White
            background: [0.0, 0.0, 0.0, 1.0], // Black
        }
    }
}

#[derive(Debug, Clone)]
pub enum StreamingEvent {
    TokenReceived(String),
    ResponseComplete,
    ResponseInterrupted,
    ErrorOccurred(String),
    TypingIndicator(bool),
    ScrollRequest(i32), // Lines to scroll
    CopyRequest(String),
}

pub struct ResponseHistory {
    responses: VecDeque<ResponseState>,
    current_index: Option<usize>,
    max_entries: usize,
}

impl ResponseHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            responses: VecDeque::with_capacity(max_entries),
            current_index: None,
            max_entries,
        }
    }

    pub fn add_response(&mut self, response: ResponseState) {
        if self.responses.len() >= self.max_entries {
            self.responses.pop_front();
        }
        self.responses.push_back(response);
        self.current_index = Some(self.responses.len() - 1);
    }

    pub fn get_current(&self) -> Option<&ResponseState> {
        if let Some(index) = self.current_index {
            self.responses.get(index)
        } else {
            self.responses.back()
        }
    }

    pub fn navigate_previous(&mut self) -> Option<&ResponseState> {
        if let Some(current) = self.current_index {
            if current > 0 {
                self.current_index = Some(current - 1);
            }
        } else if !self.responses.is_empty() {
            self.current_index = Some(self.responses.len() - 1);
        }
        self.get_current()
    }

    pub fn navigate_next(&mut self) -> Option<&ResponseState> {
        if let Some(current) = self.current_index {
            if current + 1 < self.responses.len() {
                self.current_index = Some(current + 1);
            }
        }
        self.get_current()
    }

    pub fn clear(&mut self) {
        self.responses.clear();
        self.current_index = None;
    }

    pub fn len(&self) -> usize {
        self.responses.len()
    }
}

pub struct VirtualScrollBuffer {
    lines: Vec<String>,
    styled_lines: Vec<Vec<TerminalCell>>,
    visible_start: u32,
    visible_height: u32,
    total_lines: u32,
    max_lines: u32,
}

impl VirtualScrollBuffer {
    pub fn new(max_lines: u32, visible_height: u32) -> Self {
        Self {
            lines: Vec::new(),
            styled_lines: Vec::new(),
            visible_start: 0,
            visible_height,
            total_lines: 0,
            max_lines,
        }
    }

    pub fn add_line(&mut self, line: String, styled_line: Vec<TerminalCell>) {
        if self.lines.len() >= self.max_lines as usize {
            self.lines.remove(0);
            self.styled_lines.remove(0);
        } else {
            self.total_lines += 1;
        }

        self.lines.push(line);
        self.styled_lines.push(styled_line);

        // Auto-scroll to bottom
        if self.total_lines > self.visible_height {
            self.visible_start = self.total_lines - self.visible_height;
        }
    }

    pub fn scroll(&mut self, delta: i32) {
        let new_start = (self.visible_start as i32 + delta).max(0) as u32;
        let max_start = self.total_lines.saturating_sub(self.visible_height);
        self.visible_start = new_start.min(max_start);
    }

    pub fn get_visible_lines(&self) -> &[Vec<TerminalCell>] {
        let start = self.visible_start as usize;
        let end = (start + self.visible_height as usize).min(self.styled_lines.len());
        &self.styled_lines[start..end]
    }

    pub fn is_at_bottom(&self) -> bool {
        if self.total_lines <= self.visible_height {
            return true;
        }
        self.visible_start + self.visible_height >= self.total_lines
    }

    pub fn scroll_to_bottom(&mut self) {
        if self.total_lines > self.visible_height {
            self.visible_start = self.total_lines - self.visible_height;
        } else {
            self.visible_start = 0;
        }
    }
}

pub struct SyntaxHighlighter {
    enabled: bool,
    theme: SyntaxTheme,
}

#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    pub keyword: [f32; 4],
    pub string: [f32; 4],
    pub comment: [f32; 4],
    pub number: [f32; 4],
    pub function: [f32; 4],
    pub variable: [f32; 4],
    pub operator: [f32; 4],
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        Self {
            keyword: [0.5, 0.7, 1.0, 1.0],    // Blue
            string: [0.7, 1.0, 0.7, 1.0],     // Green
            comment: [0.6, 0.6, 0.6, 1.0],    // Gray
            number: [1.0, 0.8, 0.4, 1.0],     // Orange
            function: [1.0, 1.0, 0.6, 1.0],   // Yellow
            variable: [0.9, 0.9, 0.9, 1.0],   // Light gray
            operator: [1.0, 0.6, 0.6, 1.0],   // Pink
        }
    }
}

impl SyntaxHighlighter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            theme: SyntaxTheme::default(),
        }
    }

    pub fn highlight(&self, code: &str, language: &str) -> Vec<TerminalCell> {
        if !self.enabled {
            return self.plain_highlight(code);
        }

        // Simple syntax highlighting for common languages
        match language {
            "rust" | "rs" => self.highlight_rust(code),
            "python" | "py" => self.highlight_python(code),
            "javascript" | "js" | "typescript" | "ts" => self.highlight_javascript(code),
            "json" => self.highlight_json(code),
            "yaml" | "yml" => self.highlight_yaml(code),
            "bash" | "shell" | "sh" => self.highlight_bash(code),
            "markdown" | "md" => self.highlight_markdown(code),
            _ => self.plain_highlight(code),
        }
    }

    fn plain_highlight(&self, code: &str) -> Vec<TerminalCell> {
        code.chars()
            .map(|ch| TerminalCell {
                character: ch,
                foreground: [0.8, 0.8, 0.8, 1.0], // Light gray for code
                background: [0.1, 0.1, 0.1, 1.0], // Dark background
                bold: false,
                italic: false,
                underline: false,
                strikethrough: false,
                dim: false,
                reverse: false,
                blink: false,
                wide: false,
                double_height: false,
                dirty: true,
            })
            .collect()
    }

    fn highlight_rust(&self, code: &str) -> Vec<TerminalCell> {
        let keywords = ["fn", "let", "mut", "struct", "enum", "impl", "trait", "pub", "use", "mod"];
        let mut cells = Vec::new();
        let mut i = 0;
        let chars: Vec<char> = code.chars().collect();

        while i < chars.len() {
            let ch = chars[i];
            
            if ch.is_alphabetic() || ch == '_' {
                // Check for keywords
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                
                let color = if keywords.contains(&word.as_str()) {
                    self.theme.keyword
                } else if word.chars().next().unwrap().is_uppercase() {
                    self.theme.function // Types
                } else {
                    self.theme.variable
                };

                for &ch in &chars[start..i] {
                    cells.push(TerminalCell {
                        character: ch,
                        foreground: color,
                        background: [0.1, 0.1, 0.1, 1.0],
                        bold: keywords.contains(&word.as_str()),
                        italic: false,
                        underline: false,
                        strikethrough: false,
                        dim: false,
                        reverse: false,
                        blink: false,
                        wide: false,
                        double_height: false,
                        dirty: true,
                    });
                }
            } else if ch == '"' {
                // String literal
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 2; // Skip escaped character
                    } else {
                        i += 1;
                    }
                }
                if i < chars.len() {
                    i += 1; // Include closing quote
                }

                for &ch in &chars[start..i] {
                    cells.push(TerminalCell {
                        character: ch,
                        foreground: self.theme.string,
                        background: [0.1, 0.1, 0.1, 1.0],
                        bold: false,
                        italic: false,
                        underline: false,
                        strikethrough: false,
                        dim: false,
                        reverse: false,
                        blink: false,
                        wide: false,
                        double_height: false,
                        dirty: true,
                    });
                }
            } else if ch == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                // Single-line comment
                let start = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }

                for &ch in &chars[start..i] {
                    cells.push(TerminalCell {
                        character: ch,
                        foreground: self.theme.comment,
                        background: [0.1, 0.1, 0.1, 1.0],
                        bold: false,
                        italic: true,
                        underline: false,
                        strikethrough: false,
                        dim: false,
                        reverse: false,
                        blink: false,
                        wide: false,
                        double_height: false,
                        dirty: true,
                    });
                }
            } else if ch.is_ascii_digit() {
                // Numbers
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }

                for &ch in &chars[start..i] {
                    cells.push(TerminalCell {
                        character: ch,
                        foreground: self.theme.number,
                        background: [0.1, 0.1, 0.1, 1.0],
                        bold: false,
                        italic: false,
                        underline: false,
                        strikethrough: false,
                        dim: false,
                        reverse: false,
                        blink: false,
                        wide: false,
                        double_height: false,
                        dirty: true,
                    });
                }
            } else {
                // Default character
                cells.push(TerminalCell {
                    character: ch,
                    foreground: [0.9, 0.9, 0.9, 1.0],
                    background: [0.1, 0.1, 0.1, 1.0],
                    bold: false,
                    italic: false,
                    underline: false,
                    strikethrough: false,
                    dim: false,
                    reverse: false,
                    blink: false,
                    wide: false,
                    double_height: false,
                    dirty: true,
                });
                i += 1;
            }
        }

        cells
    }

    fn highlight_python(&self, code: &str) -> Vec<TerminalCell> {
        // Simplified Python highlighting - similar structure to Rust
        self.plain_highlight(code) // Placeholder for now
    }

    fn highlight_javascript(&self, code: &str) -> Vec<TerminalCell> {
        // Simplified JavaScript highlighting
        self.plain_highlight(code) // Placeholder for now
    }

    fn highlight_json(&self, code: &str) -> Vec<TerminalCell> {
        // JSON highlighting
        self.plain_highlight(code) // Placeholder for now
    }

    fn highlight_yaml(&self, code: &str) -> Vec<TerminalCell> {
        // YAML highlighting
        self.plain_highlight(code) // Placeholder for now
    }

    fn highlight_bash(&self, code: &str) -> Vec<TerminalCell> {
        // Bash highlighting
        self.plain_highlight(code) // Placeholder for now
    }

    fn highlight_markdown(&self, code: &str) -> Vec<TerminalCell> {
        // Markdown highlighting
        self.plain_highlight(code) // Placeholder for now
    }
}

pub struct StreamingUI {
    renderer: Arc<RwLock<GpuRenderer>>,
    model_host: Arc<ModelHost>,
    config: Arc<RwLock<StreamingConfig>>,
    
    // State management
    current_response: Arc<RwLock<Option<ResponseState>>>,
    response_history: Arc<RwLock<ResponseHistory>>,
    virtual_buffer: Arc<RwLock<VirtualScrollBuffer>>,
    
    // Rendering components
    syntax_highlighter: SyntaxHighlighter,
    typing_indicator: Arc<RwLock<bool>>,
    
    // Event channels
    event_tx: mpsc::UnboundedSender<StreamingEvent>,
    event_rx: Arc<Mutex<mpsc::UnboundedReceiver<StreamingEvent>>>,
    interrupt_tx: Arc<RwLock<Option<mpsc::UnboundedSender<()>>>>,
    
    // Performance metrics
    frame_times: Arc<RwLock<VecDeque<Duration>>>,
    last_render_time: Arc<RwLock<Instant>>,
    
    // Memory tracking
    memory_usage: Arc<RwLock<u64>>,
    
    // Render loop control
    render_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl StreamingUI {
    pub fn new(
        renderer: Arc<RwLock<GpuRenderer>>,
        model_host: Arc<ModelHost>,
        config: StreamingConfig,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let grid = renderer.read().get_grid();
        let grid_height = grid.read().height;
        
        Self {
            renderer,
            model_host,
            config: Arc::new(RwLock::new(config.clone())),
            current_response: Arc::new(RwLock::new(None)),
            response_history: Arc::new(RwLock::new(ResponseHistory::new(100))),
            virtual_buffer: Arc::new(RwLock::new(VirtualScrollBuffer::new(
                config.scroll_buffer_lines,
                grid_height,
            ))),
            syntax_highlighter: SyntaxHighlighter::new(config.syntax_highlighting_enabled),
            typing_indicator: Arc::new(RwLock::new(false)),
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            interrupt_tx: Arc::new(RwLock::new(None)),
            frame_times: Arc::new(RwLock::new(VecDeque::with_capacity(60))),
            last_render_time: Arc::new(RwLock::new(Instant::now())),
            memory_usage: Arc::new(RwLock::new(0)),
            render_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the streaming UI render loop
    pub async fn start(&self) -> Result<(), StreamingUIError> {
        let render_handle = {
            let ui = self.clone();
            tokio::spawn(async move {
                ui.render_loop().await;
            })
        };
        
        *self.render_handle.write() = Some(render_handle);
        Ok(())
    }

    /// Stop the streaming UI
    pub async fn stop(&self) -> Result<(), StreamingUIError> {
        if let Some(handle) = self.render_handle.write().take() {
            handle.abort();
        }
        Ok(())
    }

    /// Start a new streaming response
    pub async fn start_streaming_response(
        &self,
        request: InferenceRequest,
    ) -> Result<String, StreamingUIError> {
        let response_id = Uuid::new_v4().to_string();
        
        // Create response state
        let response = ResponseState {
            id: response_id.clone(),
            content: String::new(),
            markdown_tokens: Vec::new(),
            start_line: self.get_current_line(),
            current_line: self.get_current_line(),
            is_active: true,
            is_interrupted: false,
            tokens_per_second: 0.0,
            total_tokens: 0,
            start_time: Instant::now(),
            last_update: Instant::now(),
            memory_usage: 0,
        };

        *self.current_response.write() = Some(response);

        // Start typing indicator
        if self.config.read().typing_indicator_enabled {
            *self.typing_indicator.write() = true;
            self.event_tx.send(StreamingEvent::TypingIndicator(true))
                .map_err(|e| StreamingUIError::Channel(e.to_string()))?;
        }

        // Set up interrupt channel
        let (interrupt_tx, interrupt_rx) = mpsc::unbounded_channel();
        *self.interrupt_tx.write() = Some(interrupt_tx);

        // Start streaming from model host
        let model_host = Arc::clone(&self.model_host);
        let event_tx = self.event_tx.clone();
        let response_id_clone = response_id.clone();
        
        tokio::spawn(async move {
            tokio::select! {
                result = model_host.infer(request) => {
                    match result {
                        Ok(response) => {
                            let _ = event_tx.send(StreamingEvent::TokenReceived(response.text));
                            let _ = event_tx.send(StreamingEvent::ResponseComplete);
                        }
                        Err(e) => {
                            let _ = event_tx.send(StreamingEvent::ErrorOccurred(e.to_string()));
                        }
                    }
                }
                _ = interrupt_rx.recv() => {
                    let _ = event_tx.send(StreamingEvent::ResponseInterrupted);
                }
            }
        });

        Ok(response_id)
    }

    /// Handle user interrupt (Ctrl+C)
    pub async fn interrupt_response(&self) -> Result<(), StreamingUIError> {
        let start_time = Instant::now();
        let timeout = Duration::from_millis(self.config.read().interrupt_timeout_ms);

        // Send interrupt signal
        if let Some(interrupt_tx) = self.interrupt_tx.read().as_ref() {
            interrupt_tx.send(())
                .map_err(|e| StreamingUIError::Channel(e.to_string()))?;
        }

        // Mark current response as interrupted
        if let Some(response) = self.current_response.write().as_mut() {
            response.is_interrupted = true;
            response.is_active = false;
        }

        // Wait for acknowledgment or timeout
        let mut timeout_interval = interval(Duration::from_millis(10));
        while start_time.elapsed() < timeout {
            timeout_interval.tick().await;
            if let Some(response) = self.current_response.read().as_ref() {
                if !response.is_active {
                    return Ok(());
                }
            }
        }

        Err(StreamingUIError::InterruptTimeout(
            self.config.read().interrupt_timeout_ms,
        ))
    }

    /// Handle input events
    pub async fn handle_input(&self, action: InputAction) -> Result<(), StreamingUIError> {
        match action {
            InputAction::Interrupt => {
                self.interrupt_response().await?;
            }
            InputAction::ScrollUp => {
                self.event_tx.send(StreamingEvent::ScrollRequest(-5))
                    .map_err(|e| StreamingUIError::Channel(e.to_string()))?;
            }
            InputAction::ScrollDown => {
                self.event_tx.send(StreamingEvent::ScrollRequest(5))
                    .map_err(|e| StreamingUIError::Channel(e.to_string()))?;
            }
            InputAction::Copy => {
                if let Some(response) = self.current_response.read().as_ref() {
                    self.event_tx.send(StreamingEvent::CopyRequest(response.content.clone()))
                        .map_err(|e| StreamingUIError::Channel(e.to_string()))?;
                }
            }
            _ => {} // Handle other input actions as needed
        }
        Ok(())
    }

    /// Navigate response history
    pub async fn navigate_history(&self, direction: i32) -> Result<(), StreamingUIError> {
        let mut history = self.response_history.write();
        let response = if direction < 0 {
            history.navigate_previous()
        } else {
            history.navigate_next()
        };

        if let Some(response) = response {
            // Re-render the historical response
            self.render_response_content(&response.content).await?;
        }

        Ok(())
    }

    /// Get current line position in the terminal
    fn get_current_line(&self) -> u32 {
        let grid = self.renderer.read().get_grid();
        grid.read().cursor_y
    }

    /// Parse markdown content into tokens
    fn parse_markdown(&self, content: &str) -> Vec<MarkdownToken> {
        let mut tokens = Vec::new();
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_FOOTNOTES);
        
        let parser = Parser::new_ext(content, options);
        let mut current_pos = 0;
        let mut in_code_block = false;
        let mut code_language = String::new();

        for event in parser {
            match event {
                Event::Start(Tag::Heading(level, _, _)) => {
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::Header(level as u8),
                        content: String::new(),
                        start_pos: current_pos,
                        end_pos: current_pos,
                        style: TextStyle {
                            bold: true,
                            underline: level <= 2,
                            color: match level {
                                1 => [1.0, 0.8, 0.2, 1.0], // Gold
                                2 => [0.8, 1.0, 0.8, 1.0], // Light green
                                _ => [0.9, 0.9, 1.0, 1.0], // Light blue
                            },
                            ..Default::default()
                        },
                    });
                }
                Event::Start(Tag::Strong) => {
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::Bold,
                        content: String::new(),
                        start_pos: current_pos,
                        end_pos: current_pos,
                        style: TextStyle {
                            bold: true,
                            ..Default::default()
                        },
                    });
                }
                Event::Start(Tag::Emphasis) => {
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::Italic,
                        content: String::new(),
                        start_pos: current_pos,
                        end_pos: current_pos,
                        style: TextStyle {
                            italic: true,
                            ..Default::default()
                        },
                    });
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                    in_code_block = true;
                    code_language = lang.to_string();
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::CodeBlock(code_language.clone()),
                        content: String::new(),
                        start_pos: current_pos,
                        end_pos: current_pos,
                        style: TextStyle {
                            color: [0.8, 0.8, 0.8, 1.0],
                            background: [0.1, 0.1, 0.1, 1.0],
                            ..Default::default()
                        },
                    });
                }
                Event::Start(Tag::Code) => {
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::Code,
                        content: String::new(),
                        start_pos: current_pos,
                        end_pos: current_pos,
                        style: TextStyle {
                            color: [1.0, 0.8, 0.6, 1.0],
                            background: [0.15, 0.1, 0.1, 1.0],
                            ..Default::default()
                        },
                    });
                }
                Event::Text(text) => {
                    let token_type = if in_code_block {
                        MarkdownTokenType::CodeBlock(code_language.clone())
                    } else {
                        MarkdownTokenType::Text
                    };
                    
                    tokens.push(MarkdownToken {
                        token_type,
                        content: text.to_string(),
                        start_pos: current_pos,
                        end_pos: current_pos + text.len(),
                        style: Default::default(),
                    });
                    current_pos += text.len();
                }
                Event::SoftBreak | Event::HardBreak => {
                    tokens.push(MarkdownToken {
                        token_type: MarkdownTokenType::LineBreak,
                        content: "\n".to_string(),
                        start_pos: current_pos,
                        end_pos: current_pos + 1,
                        style: Default::default(),
                    });
                    current_pos += 1;
                }
                Event::End(Tag::CodeBlock(_)) => {
                    in_code_block = false;
                    code_language.clear();
                }
                _ => {}
            }
        }

        tokens
    }

    /// Convert markdown tokens to styled terminal cells
    fn tokens_to_cells(&self, tokens: &[MarkdownToken], terminal_width: u32) -> Vec<Vec<TerminalCell>> {
        let mut lines = Vec::new();
        let mut current_line = Vec::new();
        let mut current_width = 0u32;

        for token in tokens {
            match &token.token_type {
                MarkdownTokenType::CodeBlock(language) => {
                    // Apply syntax highlighting
                    let highlighted = self.syntax_highlighter.highlight(&token.content, language);
                    for cell in highlighted {
                        if cell.character == '\n' || current_width >= terminal_width {
                            lines.push(std::mem::take(&mut current_line));
                            current_width = 0;
                        }
                        
                        if cell.character != '\n' {
                            current_line.push(cell);
                            current_width += 1;
                        }
                    }
                }
                MarkdownTokenType::LineBreak => {
                    lines.push(std::mem::take(&mut current_line));
                    current_width = 0;
                }
                _ => {
                    for ch in token.content.chars() {
                        if ch == '\n' || current_width >= terminal_width {
                            lines.push(std::mem::take(&mut current_line));
                            current_width = 0;
                        }
                        
                        if ch != '\n' {
                            let cell = TerminalCell {
                                character: ch,
                                foreground: token.style.color,
                                background: token.style.background,
                                bold: token.style.bold,
                                italic: token.style.italic,
                                underline: token.style.underline,
                                dim: token.style.dim,
                                strikethrough: false,
                                reverse: false,
                                blink: false,
                                wide: false,
                                double_height: false,
                                dirty: true,
                            };
                            current_line.push(cell);
                            current_width += 1;
                        }
                    }
                }
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        lines
    }

    /// Render response content to the terminal
    async fn render_response_content(&self, content: &str) -> Result<(), StreamingUIError> {
        let tokens = self.parse_markdown(content);
        let grid = self.renderer.read().get_grid();
        let terminal_width = grid.read().width;
        
        let styled_lines = self.tokens_to_cells(&tokens, terminal_width);
        
        // Update virtual buffer
        {
            let mut buffer = self.virtual_buffer.write();
            for (i, line) in styled_lines.iter().enumerate() {
                let line_text: String = line.iter().map(|cell| cell.character).collect();
                buffer.add_line(line_text, line.clone());
            }
        }

        // Update renderer grid
        self.update_renderer_grid().await?;
        
        Ok(())
    }

    /// Update the renderer grid with visible content
    async fn update_renderer_grid(&self) -> Result<(), StreamingUIError> {
        let buffer = self.virtual_buffer.read();
        let visible_lines = buffer.get_visible_lines();
        
        self.renderer.read().update_grid(|grid| {
            // Clear grid
            for y in 0..grid.height {
                for x in 0..grid.width {
                    if let Some(cell) = grid.get_cell(x, y) {
                        let mut clear_cell = *cell;
                        clear_cell.character = ' ';
                        clear_cell.dirty = true;
                        grid.set_cell(x, y, clear_cell);
                    }
                }
            }

            // Render visible content
            for (line_idx, line) in visible_lines.iter().enumerate() {
                let y = line_idx as u32;
                if y >= grid.height {
                    break;
                }

                for (col_idx, cell) in line.iter().enumerate() {
                    let x = col_idx as u32;
                    if x >= grid.width {
                        break;
                    }
                    
                    grid.set_cell(x, y, *cell);
                }
            }

            // Add typing indicator if active
            if *self.typing_indicator.read() && visible_lines.len() < grid.height as usize {
                let indicator_y = visible_lines.len() as u32;
                let indicator_chars = "▋ Generating...";
                
                for (i, ch) in indicator_chars.chars().enumerate() {
                    let x = i as u32;
                    if x >= grid.width {
                        break;
                    }
                    
                    let cell = TerminalCell {
                        character: ch,
                        foreground: [0.6, 0.6, 0.6, 1.0], // Dim white
                        background: [0.0, 0.0, 0.0, 1.0],
                        bold: false,
                        italic: true,
                        underline: false,
                        strikethrough: false,
                        dim: true,
                        reverse: false,
                        blink: i == 0, // Blink the cursor
                        wide: false,
                        double_height: false,
                        dirty: true,
                    };
                    grid.set_cell(x, indicator_y, cell);
                }
            }
        });

        Ok(())
    }

    /// Main render loop
    async fn render_loop(&self) {
        let mut render_interval = interval(Duration::from_millis(
            self.config.read().render_interval_ms
        ));
        let mut event_rx = self.event_rx.lock().await;

        loop {
            tokio::select! {
                _ = render_interval.tick() => {
                    if let Err(e) = self.render_frame().await {
                        tracing::error!("Render frame error: {}", e);
                    }
                }
                event = event_rx.recv() => {
                    if let Some(event) = event {
                        if let Err(e) = self.handle_streaming_event(event).await {
                            tracing::error!("Event handling error: {}", e);
                        }
                    } else {
                        break; // Channel closed
                    }
                }
            }
        }
    }

    /// Handle streaming events
    async fn handle_streaming_event(&self, event: StreamingEvent) -> Result<(), StreamingUIError> {
        match event {
            StreamingEvent::TokenReceived(token) => {
                if let Some(response) = self.current_response.write().as_mut() {
                    response.content.push_str(&token);
                    response.total_tokens += 1;
                    response.last_update = Instant::now();
                    
                    // Calculate tokens per second
                    let elapsed = response.start_time.elapsed().as_secs_f32();
                    if elapsed > 0.0 {
                        response.tokens_per_second = response.total_tokens as f32 / elapsed;
                    }
                    
                    // Update memory usage
                    response.memory_usage = (response.content.len() * std::mem::size_of::<u8>()) as u64;
                    *self.memory_usage.write() = response.memory_usage;
                    
                    // Check memory limit
                    let config = self.config.read();
                    if response.memory_usage > config.memory_limit_mb * 1024 * 1024 {
                        return Err(StreamingUIError::MemoryLimit {
                            current: response.memory_usage / (1024 * 1024),
                            limit: config.memory_limit_mb,
                        });
                    }

                    // Progressive rendering
                    if config.progressive_rendering && response.content.len() % config.batch_size == 0 {
                        self.render_response_content(&response.content).await?;
                    }
                }
            }
            StreamingEvent::ResponseComplete => {
                *self.typing_indicator.write() = false;
                
                if let Some(mut response) = self.current_response.write().take() {
                    response.is_active = false;
                    
                    // Final render
                    self.render_response_content(&response.content).await?;
                    
                    // Add to history
                    self.response_history.write().add_response(response);
                }
            }
            StreamingEvent::ResponseInterrupted => {
                *self.typing_indicator.write() = false;
                
                if let Some(mut response) = self.current_response.write().as_mut() {
                    response.is_interrupted = true;
                    response.is_active = false;
                    response.content.push_str("\n[INTERRUPTED]");
                    
                    // Render with interruption marker
                    self.render_response_content(&response.content).await?;
                }
            }
            StreamingEvent::ErrorOccurred(error) => {
                *self.typing_indicator.write() = false;
                
                if let Some(mut response) = self.current_response.write().as_mut() {
                    response.is_active = false;
                    response.content.push_str(&format!("\n[ERROR: {}]", error));
                    
                    // Render with error marker
                    self.render_response_content(&response.content).await?;
                }
            }
            StreamingEvent::ScrollRequest(delta) => {
                self.virtual_buffer.write().scroll(delta);
                self.update_renderer_grid().await?;
            }
            StreamingEvent::CopyRequest(content) => {
                // TODO: Implement clipboard copy
                tracing::info!("Copy request: {} chars", content.len());
            }
            StreamingEvent::TypingIndicator(enabled) => {
                *self.typing_indicator.write() = enabled;
            }
        }
        Ok(())
    }

    /// Render a single frame
    async fn render_frame(&self) -> Result<(), StreamingUIError> {
        let start_time = Instant::now();
        
        // Update renderer grid
        self.update_renderer_grid().await?;
        
        // Render to screen
        if let Err(e) = self.renderer.write().render() {
            return Err(StreamingUIError::Renderer(e.to_string()));
        }
        
        // Track performance
        let frame_time = start_time.elapsed();
        {
            let mut frame_times = self.frame_times.write();
            if frame_times.len() >= 60 {
                frame_times.pop_front();
            }
            frame_times.push_back(frame_time);
        }
        
        *self.last_render_time.write() = Instant::now();
        
        // Check performance targets
        if frame_time > Duration::from_millis(16) {
            tracing::warn!("Slow frame: {:?} (target: 16ms)", frame_time);
        }
        
        Ok(())
    }

    /// Get current performance metrics
    pub fn get_performance_metrics(&self) -> PerformanceMetrics {
        let frame_times = self.frame_times.read();
        let avg_frame_time = if frame_times.is_empty() {
            Duration::from_millis(0)
        } else {
            frame_times.iter().sum::<Duration>() / frame_times.len() as u32
        };
        
        let fps = if avg_frame_time.as_millis() > 0 {
            1000.0 / avg_frame_time.as_millis() as f32
        } else {
            0.0
        };
        
        PerformanceMetrics {
            average_frame_time: avg_frame_time,
            current_fps: fps,
            memory_usage_mb: *self.memory_usage.read() / (1024 * 1024),
            frames_dropped: frame_times.iter().filter(|&&t| t > Duration::from_millis(16)).count(),
        }
    }

    /// Get current response state
    pub fn get_current_response(&self) -> Option<ResponseState> {
        self.current_response.read().clone()
    }

    /// Clear response history
    pub fn clear_history(&self) {
        self.response_history.write().clear();
    }

    /// Update configuration
    pub fn update_config(&self, config: StreamingConfig) {
        *self.config.write() = config;
    }
}

impl Clone for StreamingUI {
    fn clone(&self) -> Self {
        Self {
            renderer: Arc::clone(&self.renderer),
            model_host: Arc::clone(&self.model_host),
            config: Arc::clone(&self.config),
            current_response: Arc::clone(&self.current_response),
            response_history: Arc::clone(&self.response_history),
            virtual_buffer: Arc::clone(&self.virtual_buffer),
            syntax_highlighter: SyntaxHighlighter::new(self.config.read().syntax_highlighting_enabled),
            typing_indicator: Arc::clone(&self.typing_indicator),
            event_tx: self.event_tx.clone(),
            event_rx: Arc::clone(&self.event_rx),
            interrupt_tx: Arc::clone(&self.interrupt_tx),
            frame_times: Arc::clone(&self.frame_times),
            last_render_time: Arc::clone(&self.last_render_time),
            memory_usage: Arc::clone(&self.memory_usage),
            render_handle: Arc::clone(&self.render_handle),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub average_frame_time: Duration,
    pub current_fps: f32,
    pub memory_usage_mb: u64,
    pub frames_dropped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigManager;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_markdown_parsing() {
        let content = "# Header\n\nSome **bold** text and *italic* text.\n\n```rust\nfn main() {}\n```";
        
        // Mock renderer and model host for testing
        // In a real test, we'd need to set up proper mocks
        
        let streaming_ui = create_test_streaming_ui().await;
        let tokens = streaming_ui.parse_markdown(content);
        
        assert!(!tokens.is_empty());
        assert!(tokens.iter().any(|t| matches!(t.token_type, MarkdownTokenType::Header(1))));
        assert!(tokens.iter().any(|t| matches!(t.token_type, MarkdownTokenType::Bold)));
        assert!(tokens.iter().any(|t| matches!(t.token_type, MarkdownTokenType::Italic)));
        assert!(tokens.iter().any(|t| matches!(t.token_type, MarkdownTokenType::CodeBlock(_))));
    }

    #[tokio::test]
    async fn test_virtual_scrolling() {
        let mut buffer = VirtualScrollBuffer::new(100, 10);
        
        // Add more lines than visible
        for i in 0..20 {
            let line = format!("Line {}", i);
            let styled_line = line.chars().map(|ch| TerminalCell {
                character: ch,
                foreground: [1.0, 1.0, 1.0, 1.0],
                background: [0.0, 0.0, 0.0, 1.0],
                bold: false,
                italic: false,
                underline: false,
                strikethrough: false,
                dim: false,
                reverse: false,
                blink: false,
                wide: false,
                double_height: false,
                dirty: true,
            }).collect();
            buffer.add_line(line, styled_line);
        }
        
        // Should auto-scroll to bottom
        assert!(buffer.is_at_bottom());
        
        // Scroll up
        buffer.scroll(-5);
        assert!(!buffer.is_at_bottom());
        
        // Scroll back to bottom
        buffer.scroll_to_bottom();
        assert!(buffer.is_at_bottom());
    }

    #[tokio::test]
    async fn test_syntax_highlighting() {
        let highlighter = SyntaxHighlighter::new(true);
        let code = "fn main() {\n    println!(\"Hello, world!\");\n}";
        let highlighted = highlighter.highlight(code, "rust");
        
        assert_eq!(highlighted.len(), code.len());
        
        // Check that keywords are highlighted differently
        let fn_cells: Vec<_> = highlighted.iter()
            .take(2)
            .collect();
        assert!(fn_cells.iter().any(|cell| cell.bold)); // "fn" should be bold
    }

    async fn create_test_streaming_ui() -> StreamingUI {
        // This is a simplified test setup
        // In practice, you'd want to create proper mocks
        
        use crate::model_host::ModelHost;
        use crate::renderer::GpuRenderer;
        
        // Mock renderer setup would go here
        // For now, we'll skip the actual test due to complexity of setting up GPU context
        panic!("Test requires proper GPU context setup");
    }

    #[test]
    fn test_response_history() {
        let mut history = ResponseHistory::new(3);
        
        // Add responses
        for i in 0..5 {
            let response = ResponseState {
                id: format!("response-{}", i),
                content: format!("Content {}", i),
                markdown_tokens: Vec::new(),
                start_line: 0,
                current_line: 0,
                is_active: false,
                is_interrupted: false,
                tokens_per_second: 0.0,
                total_tokens: 10,
                start_time: Instant::now(),
                last_update: Instant::now(),
                memory_usage: 100,
            };
            history.add_response(response);
        }
        
        // Should only keep last 3
        assert_eq!(history.len(), 3);
        
        // Should start at most recent
        assert_eq!(history.get_current().unwrap().id, "response-4");
        
        // Navigate backward
        history.navigate_previous();
        assert_eq!(history.get_current().unwrap().id, "response-3");
        
        // Navigate forward
        history.navigate_next();
        assert_eq!(history.get_current().unwrap().id, "response-4");
    }
}