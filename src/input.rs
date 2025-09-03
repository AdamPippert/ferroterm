use crate::command_parser::{CommandParser, ParsedCommand};
use crate::config::{ConfigManager, KeymapConfig};
use parking_lot::{RwLock, Mutex};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Error, Debug)]
pub enum InputError {
    #[error("Key parsing error: {0}")]
    KeyParse(String),
    #[error("Channel error: {0}")]
    Channel(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
    // Additional keys for better compatibility
    Space,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    // Keypad keys
    KpDivide, KpMultiply, KpMinus, KpPlus, KpEnter, KpPeriod,
    Kp0, Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9,
    // Media keys
    VolumeUp, VolumeDown, VolumeMute,
    MediaNext, MediaPrev, MediaStop, MediaPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super, // Windows/Cmd key
    Meta,  // Additional meta key for some platforms
    Hyper, // Hyper key (rare but possible)
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: HashSet<Modifier>,
    pub text: Option<String>,
    pub repeat: bool,
    pub timestamp: Instant,
    pub key_code: Option<u32>, // Physical key code for international layouts
}

#[derive(Debug, Clone)]
pub enum InputAction {
    SendToTerminal(String),
    ExecuteCommand(String),
    ExecuteParsedCommand(ParsedCommand),
    SwitchTab(usize),
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
    Copy,
    Paste,
    Cut,
    SelectAll,
    Clear,
    ClearLine,
    Interrupt,
    Eof,
    Suspend,
    Resume,
    // History navigation
    HistoryPrev,
    HistoryNext,
    HistorySearch,
    // Window management
    NewWindow,
    CloseWindow,
    NextWindow,
    PrevWindow,
    // Custom actions
    Custom(String, Vec<String>),
    // Shell-specific actions
    WordBack,
    WordForward,
    LineStart,
    LineEnd,
    DeleteWord,
    DeleteToEnd,
    DeleteToStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: Key,
    pub modifiers: HashSet<Modifier>,
    pub context: KeyBindingContext,
}

impl std::hash::Hash for KeyBinding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        // Sort modifiers for consistent hashing
        let mut mods: Vec<_> = self.modifiers.iter().collect();
        mods.sort_by_key(|m| match m {
            Modifier::Ctrl => 0,
            Modifier::Alt => 1,
            Modifier::Shift => 2,
            Modifier::Super => 3,
            Modifier::Meta => 4,
            Modifier::Hyper => 5,
        });
        for modifier in mods {
            modifier.hash(state);
        }
        self.context.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyBindingContext {
    Global,
    Shell,
    Agent,
    Vi,
    Emacs,
}

#[derive(Debug, Clone)]
pub struct KeyBindingAction {
    pub action: InputAction,
    pub priority: u8, // Higher priority wins conflicts
    pub condition: Option<String>, // Optional condition script
}

pub type KeyBindingMap = HashMap<KeyBinding, KeyBindingAction>;

#[derive(Debug, Clone)]
pub struct PrefixState {
    pub detected: bool,
    pub buffer: String,
    pub escape_mode: bool,
    pub start_time: Option<Instant>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub enum ShellMode {
    Emacs,
    Vi,
    Auto,
}

#[derive(Debug, Clone)]
pub struct InputState {
    pub cursor_position: usize,
    pub line_start: bool,
    pub in_paste_mode: bool,
    pub shell_mode: ShellMode,
    pub last_key_time: Instant,
    pub key_sequence: VecDeque<KeyEvent>,
    pub max_sequence_length: usize,
}

pub struct InputProcessor {
    // Configuration
    keymap_config: Arc<RwLock<KeymapConfig>>,
    keybindings: Arc<RwLock<KeyBindingMap>>,
    command_parser: Arc<RwLock<CommandParser>>,
    config_manager: Arc<ConfigManager>,
    
    // Communication
    action_sender: Sender<InputAction>,
    action_receiver: Receiver<InputAction>,
    
    // State management
    prefix_state: Arc<Mutex<PrefixState>>,
    input_state: Arc<Mutex<InputState>>,
    
    // Performance optimization
    key_lookup_cache: Arc<Mutex<HashMap<KeyBinding, Option<KeyBindingAction>>>>,
    stats: Arc<Mutex<InputStats>>,
}

#[derive(Debug, Clone, Default)]
pub struct InputStats {
    pub total_keys_processed: u64,
    pub avg_processing_time_ns: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub prefix_activations: u64,
    pub conflicts_resolved: u64,
}

impl InputProcessor {
    pub fn new(
        keymap_config: Arc<RwLock<KeymapConfig>>,
        command_parser: Arc<RwLock<CommandParser>>,
        config_manager: Arc<ConfigManager>,
    ) -> Self {
        let (action_sender, action_receiver) = mpsc::channel(1000);

        let keybindings = Arc::new(RwLock::new(Self::build_default_keybindings()));
        
        let prefix_state = Arc::new(Mutex::new(PrefixState {
            detected: false,
            buffer: String::new(),
            escape_mode: false,
            start_time: None,
            timeout_ms: 5000,
        }));

        let input_state = Arc::new(Mutex::new(InputState {
            cursor_position: 0,
            line_start: true,
            in_paste_mode: false,
            shell_mode: ShellMode::Auto,
            last_key_time: Instant::now(),
            key_sequence: VecDeque::with_capacity(10),
            max_sequence_length: 10,
        }));

        Self {
            keymap_config,
            keybindings,
            command_parser,
            config_manager,
            action_sender,
            action_receiver,
            prefix_state,
            input_state,
            key_lookup_cache: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(InputStats::default())),
        }
    }

    fn build_default_keybindings() -> KeyBindingMap {
        let mut bindings = HashMap::new();

        // Standard terminal keybindings
        Self::add_binding(&mut bindings, "ctrl+c", InputAction::Interrupt, 100, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+d", InputAction::Eof, 100, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+z", InputAction::Suspend, 100, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+l", InputAction::Clear, 90, KeyBindingContext::Global);

        // Copy/paste
        Self::add_binding(&mut bindings, "ctrl+shift+c", InputAction::Copy, 90, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+shift+v", InputAction::Paste, 90, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+shift+x", InputAction::Cut, 90, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+a", InputAction::SelectAll, 80, KeyBindingContext::Global);

        // Scrolling
        Self::add_binding(&mut bindings, "shift+pageup", InputAction::ScrollPageUp, 80, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "shift+pagedown", InputAction::ScrollPageDown, 80, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+home", InputAction::ScrollToTop, 80, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+end", InputAction::ScrollToBottom, 80, KeyBindingContext::Global);

        // Emacs-style bindings
        Self::add_binding(&mut bindings, "ctrl+a", InputAction::LineStart, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+e", InputAction::LineEnd, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+k", InputAction::DeleteToEnd, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+u", InputAction::DeleteToStart, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+w", InputAction::DeleteWord, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "alt+b", InputAction::WordBack, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "alt+f", InputAction::WordForward, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+p", InputAction::HistoryPrev, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+n", InputAction::HistoryNext, 70, KeyBindingContext::Emacs);
        Self::add_binding(&mut bindings, "ctrl+r", InputAction::HistorySearch, 70, KeyBindingContext::Emacs);

        // Window management
        Self::add_binding(&mut bindings, "ctrl+shift+t", InputAction::NewWindow, 60, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+shift+w", InputAction::CloseWindow, 60, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+tab", InputAction::NextWindow, 60, KeyBindingContext::Global);
        Self::add_binding(&mut bindings, "ctrl+shift+tab", InputAction::PrevWindow, 60, KeyBindingContext::Global);

        bindings
    }

    fn add_binding(
        bindings: &mut KeyBindingMap, 
        key_str: &str, 
        action: InputAction, 
        priority: u8,
        context: KeyBindingContext
    ) {
        if let Ok(key_binding) = Self::parse_key_binding(key_str, context) {
            bindings.insert(key_binding, KeyBindingAction {
                action,
                priority,
                condition: None,
            });
        }
    }

    fn parse_key_binding(key_str: &str, context: KeyBindingContext) -> Result<KeyBinding, InputError> {
        let parts: Vec<&str> = key_str.split('+').collect();
        if parts.is_empty() {
            return Err(InputError::KeyParse("Empty key binding".to_string()));
        }

        let key_part = parts.last().unwrap();
        let modifier_parts = &parts[..parts.len() - 1];

        let mut modifiers = HashSet::new();
        for modifier in modifier_parts {
            match modifier.to_lowercase().as_str() {
                "ctrl" => { modifiers.insert(Modifier::Ctrl); }
                "alt" => { modifiers.insert(Modifier::Alt); }
                "shift" => { modifiers.insert(Modifier::Shift); }
                "super" | "cmd" | "win" => { modifiers.insert(Modifier::Super); }
                "meta" => { modifiers.insert(Modifier::Meta); }
                "hyper" => { modifiers.insert(Modifier::Hyper); }
                _ => return Err(InputError::KeyParse(format!("Unknown modifier: {}", modifier))),
            }
        }

        let key = match key_part.to_lowercase().as_str() {
            "space" => Key::Space,
            "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "delete" | "del" => Key::Delete,
            "escape" | "esc" => Key::Escape,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" | "pgup" => Key::PageUp,
            "pagedown" | "pgdn" => Key::PageDown,
            "insert" | "ins" => Key::Insert,
            "f1" => Key::F1, "f2" => Key::F2, "f3" => Key::F3, "f4" => Key::F4,
            "f5" => Key::F5, "f6" => Key::F6, "f7" => Key::F7, "f8" => Key::F8,
            "f9" => Key::F9, "f10" => Key::F10, "f11" => Key::F11, "f12" => Key::F12,
            "f13" => Key::F13, "f14" => Key::F14, "f15" => Key::F15, "f16" => Key::F16,
            "f17" => Key::F17, "f18" => Key::F18, "f19" => Key::F19, "f20" => Key::F20,
            "f21" => Key::F21, "f22" => Key::F22, "f23" => Key::F23, "f24" => Key::F24,
            single_char if single_char.len() == 1 => {
                Key::Char(single_char.chars().next().unwrap())
            }
            _ => return Err(InputError::KeyParse(format!("Unknown key: {}", key_part))),
        };

        Ok(KeyBinding {
            key,
            modifiers,
            context,
        })
    }

    pub async fn process_key_event(&mut self, event: KeyEvent) -> Result<(), InputError> {
        let start_time = Instant::now();
        
        // Update statistics
        {
            let mut stats = self.stats.lock();
            stats.total_keys_processed += 1;
        }

        // Handle key repeat filtering if needed
        if event.repeat && !self.should_allow_repeat(&event) {
            return Ok(());
        }

        // Update input state
        self.update_input_state(&event);

        // Handle bracketed paste mode detection
        if self.detect_paste_mode(&event) {
            return self.handle_paste_mode(event).await;
        }

        // Check for prefix detection first (highest priority)
        if let Some(action) = self.check_prefix_activation(&event)? {
            self.execute_action(action).await?;
            return Ok(());
        }

        // Try keybinding resolution
        if let Some(action) = self.resolve_keybinding(&event)? {
            self.execute_action(action).await?;
            return Ok(());
        }

        // Handle regular character input
        self.handle_regular_input(event).await?;

        // Update performance statistics
        let processing_time = start_time.elapsed();
        {
            let mut stats = self.stats.lock();
            let total_time = stats.avg_processing_time_ns * (stats.total_keys_processed - 1) + processing_time.as_nanos() as u64;
            stats.avg_processing_time_ns = total_time / stats.total_keys_processed;
        }

        // Warn if processing is too slow
        if processing_time.as_micros() > 100 {
            eprintln!(
                "Warning: Input processing took {}μs (target: <100μs)", 
                processing_time.as_micros()
            );
        }

        Ok(())
    }

    fn should_allow_repeat(&self, event: &KeyEvent) -> bool {
        // Allow repeat for certain keys
        matches!(event.key, 
            Key::Backspace | Key::Delete | Key::Up | Key::Down | 
            Key::Left | Key::Right | Key::Space | Key::Char(_)
        )
    }

    fn update_input_state(&self, event: &KeyEvent) {
        let mut state = self.input_state.lock();
        state.last_key_time = event.timestamp;
        
        // Update key sequence for pattern detection
        state.key_sequence.push_back(event.clone());
        while state.key_sequence.len() > state.max_sequence_length {
            state.key_sequence.pop_front();
        }

        // Update cursor position tracking
        match event.key {
            Key::Left => state.cursor_position = state.cursor_position.saturating_sub(1),
            Key::Right => state.cursor_position += 1,
            Key::Home => {
                state.cursor_position = 0;
                state.line_start = true;
            },
            Key::End => state.line_start = false,
            Key::Enter => {
                state.cursor_position = 0;
                state.line_start = true;
            },
            Key::Backspace if state.cursor_position > 0 => {
                state.cursor_position -= 1;
                if state.cursor_position == 0 {
                    state.line_start = true;
                }
            },
            Key::Char(_) => {
                if state.line_start && state.cursor_position == 0 {
                    state.line_start = false;
                }
                state.cursor_position += 1;
            },
            _ => {}
        }

        // Auto-detect shell mode if not set
        if matches!(state.shell_mode, ShellMode::Auto) {
            state.shell_mode = self.detect_shell_mode();
        }
    }

    fn detect_paste_mode(&self, event: &KeyEvent) -> bool {
        // Detect bracketed paste sequences
        if let Key::Char(c) = event.key {
            if c == '\x1b' && event.text.as_ref().map_or(false, |t| t.starts_with("\x1b[200~")) {
                return true;
            }
        }
        false
    }

    async fn handle_paste_mode(&mut self, event: KeyEvent) -> Result<(), InputError> {
        let mut state = self.input_state.lock();
        state.in_paste_mode = true;
        drop(state);

        // In paste mode, collect all characters until end sequence
        if let Some(text) = event.text {
            if text.ends_with("\x1b[201~") {
                let paste_content = text.trim_start_matches("\x1b[200~").trim_end_matches("\x1b[201~");
                
                // Reset paste mode
                self.input_state.lock().in_paste_mode = false;
                
                // Send paste content as regular text
                self.execute_action(InputAction::SendToTerminal(paste_content.to_string())).await?;
            } else {
                // Continue collecting paste content
                self.execute_action(InputAction::SendToTerminal(text)).await?;
            }
        }

        Ok(())
    }

    fn check_prefix_activation(&self, event: &KeyEvent) -> Result<Option<InputAction>, InputError> {
        let keymap = self.keymap_config.read();
        let prefix_char = keymap.prefix.chars().next().unwrap_or('p');
        
        // Check if we're in escape mode first
        {
            let mut prefix_state = self.prefix_state.lock();
            if prefix_state.escape_mode {
                if matches!(event.key, Key::Char(c) if c == prefix_char) {
                    // Send literal prefix character
                    prefix_state.escape_mode = false;
                    return Ok(Some(InputAction::SendToTerminal(prefix_char.to_string())));
                } else {
                    // Send escape character and continue processing
                    prefix_state.escape_mode = false;
                    // Fall through to normal processing
                }
            }
        }

        // Check for escape sequence start
        if matches!(event.key, Key::Char('\\')) && self.is_at_line_start() {
            self.prefix_state.lock().escape_mode = true;
            return Ok(None); // Consume the backslash
        }

        // Check for prefix activation
        if !self.is_prefix_active() && self.is_at_line_start() {
            if let Key::Char(c) = event.key {
                if c == prefix_char && event.modifiers.is_empty() {
                    let mut prefix_state = self.prefix_state.lock();
                    prefix_state.detected = true;
                    prefix_state.start_time = Some(event.timestamp);
                    
                    // Increment statistics
                    self.stats.lock().prefix_activations += 1;
                    
                    return Ok(None); // Consume the prefix character
                }
            }
        }

        // Handle commands in prefix mode
        if self.is_prefix_active() {
            match event.key {
                Key::Enter => {
                    let command = {
                        let mut prefix_state = self.prefix_state.lock();
                        let cmd = prefix_state.buffer.clone();
                        prefix_state.detected = false;
                        prefix_state.buffer.clear();
                        prefix_state.start_time = None;
                        cmd
                    };
                    
                    if !command.is_empty() {
                        match self.command_parser.write().parse(&command) {
                            Ok(parsed) => return Ok(Some(InputAction::ExecuteParsedCommand(parsed))),
                            Err(e) => {
                                let error_msg = format!("Command error: {}\n", e);
                                return Ok(Some(InputAction::SendToTerminal(error_msg)));
                            }
                        }
                    }
                }
                Key::Escape => {
                    // Cancel command
                    let mut prefix_state = self.prefix_state.lock();
                    prefix_state.detected = false;
                    prefix_state.buffer.clear();
                    prefix_state.start_time = None;
                    return Ok(None);
                }
                Key::Backspace => {
                    let mut prefix_state = self.prefix_state.lock();
                    if !prefix_state.buffer.is_empty() {
                        prefix_state.buffer.pop();
                    } else {
                        // Exit prefix mode if buffer is empty
                        prefix_state.detected = false;
                        prefix_state.start_time = None;
                    }
                    return Ok(None);
                }
                Key::Tab => {
                    // TODO: Implement command completion
                    return Ok(None);
                }
                Key::Char(c) => {
                    self.prefix_state.lock().buffer.push(c);
                    return Ok(None);
                }
                _ => {
                    // Ignore other keys in prefix mode
                    return Ok(None);
                }
            }
        }

        // Check for prefix timeout
        {
            let mut prefix_state = self.prefix_state.lock();
            if let Some(start_time) = prefix_state.start_time {
                if event.timestamp.duration_since(start_time).as_millis() > prefix_state.timeout_ms as u128 {
                    prefix_state.detected = false;
                    prefix_state.buffer.clear();
                    prefix_state.start_time = None;
                }
            }
        }

        Ok(None)
    }

    fn resolve_keybinding(&self, event: &KeyEvent) -> Result<Option<InputAction>, InputError> {
        let binding = KeyBinding {
            key: event.key,
            modifiers: event.modifiers.clone(),
            context: self.get_current_context(),
        };

        // Check cache first for O(1) performance
        {
            let cache = self.key_lookup_cache.lock();
            if let Some(cached_result) = cache.get(&binding) {
                if cached_result.is_some() {
                    self.stats.lock().cache_hits += 1;
                } else {
                    self.stats.lock().cache_misses += 1;
                }
                return Ok(cached_result.as_ref().map(|action| action.action.clone()));
            }
        }

        // Look up in keybindings
        let keybindings = self.keybindings.read();
        
        // Try exact match first
        if let Some(action) = keybindings.get(&binding) {
            // Cache the result
            self.key_lookup_cache.lock().insert(binding, Some(action.clone()));
            return Ok(Some(action.action.clone()));
        }

        // Try context-agnostic match (Global context)
        let global_binding = KeyBinding {
            context: KeyBindingContext::Global,
            ..binding.clone()
        };
        
        if let Some(action) = keybindings.get(&global_binding) {
            // Cache the result
            self.key_lookup_cache.lock().insert(binding, Some(action.clone()));
            return Ok(Some(action.action.clone()));
        }

        // Handle conflicts by priority
        let mut matching_bindings: Vec<_> = keybindings.iter()
            .filter(|(kb, _)| kb.key == binding.key && kb.modifiers == binding.modifiers)
            .collect();

        if !matching_bindings.is_empty() {
            self.stats.lock().conflicts_resolved += 1;
            
            // Sort by priority (higher priority first)
            matching_bindings.sort_by(|a, b| b.1.priority.cmp(&a.1.priority));
            
            let highest_priority_action = matching_bindings[0].1.clone();
            
            // Cache the result
            self.key_lookup_cache.lock().insert(binding, Some(highest_priority_action.clone()));
            return Ok(Some(highest_priority_action.action));
        }

        // No match found - cache the negative result
        self.key_lookup_cache.lock().insert(binding, None);
        Ok(None)
    }

    fn get_current_context(&self) -> KeyBindingContext {
        let input_state = self.input_state.lock();
        
        if self.is_prefix_active() {
            return KeyBindingContext::Agent;
        }
        
        match input_state.shell_mode {
            ShellMode::Vi => KeyBindingContext::Vi,
            ShellMode::Emacs => KeyBindingContext::Emacs,
            ShellMode::Auto => KeyBindingContext::Shell,
        }
    }

    fn detect_shell_mode(&self) -> ShellMode {
        // Try to detect shell mode from environment
        if let Ok(editor) = std::env::var("EDITOR") {
            if editor.contains("vi") || editor.contains("vim") {
                return ShellMode::Vi;
            }
        }
        
        if let Ok(inputrc) = std::env::var("INPUTRC") {
            if let Ok(content) = std::fs::read_to_string(inputrc) {
                if content.contains("set editing-mode vi") {
                    return ShellMode::Vi;
                } else if content.contains("set editing-mode emacs") {
                    return ShellMode::Emacs;
                }
            }
        }

        // Default to emacs mode
        ShellMode::Emacs
    }

    async fn handle_regular_input(&mut self, event: KeyEvent) -> Result<(), InputError> {
        match event.key {
            Key::Char(c) => {
                self.execute_action(InputAction::SendToTerminal(c.to_string())).await?;
            }
            Key::Enter => {
                self.execute_action(InputAction::SendToTerminal("\n".to_string())).await?;
            }
            Key::Tab => {
                self.execute_action(InputAction::SendToTerminal("\t".to_string())).await?;
            }
            Key::Backspace => {
                self.execute_action(InputAction::SendToTerminal("\x08".to_string())).await?;
            }
            Key::Delete => {
                self.execute_action(InputAction::SendToTerminal("\x7f".to_string())).await?;
            }
            Key::Escape => {
                self.execute_action(InputAction::SendToTerminal("\x1b".to_string())).await?;
            }
            Key::Up => {
                self.execute_action(InputAction::SendToTerminal("\x1b[A".to_string())).await?;
            }
            Key::Down => {
                self.execute_action(InputAction::SendToTerminal("\x1b[B".to_string())).await?;
            }
            Key::Right => {
                self.execute_action(InputAction::SendToTerminal("\x1b[C".to_string())).await?;
            }
            Key::Left => {
                self.execute_action(InputAction::SendToTerminal("\x1b[D".to_string())).await?;
            }
            Key::Home => {
                self.execute_action(InputAction::SendToTerminal("\x1b[H".to_string())).await?;
            }
            Key::End => {
                self.execute_action(InputAction::SendToTerminal("\x1b[F".to_string())).await?;
            }
            Key::PageUp => {
                self.execute_action(InputAction::SendToTerminal("\x1b[5~".to_string())).await?;
            }
            Key::PageDown => {
                self.execute_action(InputAction::SendToTerminal("\x1b[6~".to_string())).await?;
            }
            Key::Insert => {
                self.execute_action(InputAction::SendToTerminal("\x1b[2~".to_string())).await?;
            }
            Key::F1 => self.execute_action(InputAction::SendToTerminal("\x1bOP".to_string())).await?,
            Key::F2 => self.execute_action(InputAction::SendToTerminal("\x1bOQ".to_string())).await?,
            Key::F3 => self.execute_action(InputAction::SendToTerminal("\x1bOR".to_string())).await?,
            Key::F4 => self.execute_action(InputAction::SendToTerminal("\x1bOS".to_string())).await?,
            Key::F5 => self.execute_action(InputAction::SendToTerminal("\x1b[15~".to_string())).await?,
            Key::F6 => self.execute_action(InputAction::SendToTerminal("\x1b[17~".to_string())).await?,
            Key::F7 => self.execute_action(InputAction::SendToTerminal("\x1b[18~".to_string())).await?,
            Key::F8 => self.execute_action(InputAction::SendToTerminal("\x1b[19~".to_string())).await?,
            Key::F9 => self.execute_action(InputAction::SendToTerminal("\x1b[20~".to_string())).await?,
            Key::F10 => self.execute_action(InputAction::SendToTerminal("\x1b[21~".to_string())).await?,
            Key::F11 => self.execute_action(InputAction::SendToTerminal("\x1b[23~".to_string())).await?,
            Key::F12 => self.execute_action(InputAction::SendToTerminal("\x1b[24~".to_string())).await?,
            _ => {
                // Handle other special keys or use the text representation
                if let Some(text) = event.text {
                    self.execute_action(InputAction::SendToTerminal(text)).await?;
                }
            }
        }
        Ok(())
    }

    async fn execute_action(&mut self, action: InputAction) -> Result<(), InputError> {
        self.action_sender
            .send(action)
            .await
            .map_err(|e| InputError::Channel(format!("Failed to send action: {}", e)))
    }

    fn is_at_line_start(&self) -> bool {
        self.input_state.lock().line_start || self.input_state.lock().cursor_position == 0
    }

    fn is_prefix_active(&self) -> bool {
        self.prefix_state.lock().detected
    }

    // Configuration management
    pub async fn load_keybindings_from_config(&mut self) -> Result<(), InputError> {
        let config = self.config_manager.get_config();
        let mut new_bindings = Self::build_default_keybindings();
        
        // Add user-defined bindings from config
        for (key_str, action_str) in &config.keymap.bindings {
            if let Ok(key_binding) = Self::parse_key_binding(key_str, KeyBindingContext::Global) {
                if let Some(action) = self.string_to_action(action_str) {
                    new_bindings.insert(key_binding, KeyBindingAction {
                        action,
                        priority: 100, // User bindings get highest priority
                        condition: None,
                    });
                }
            }
        }

        // Update keybindings and clear cache
        *self.keybindings.write() = new_bindings;
        self.key_lookup_cache.lock().clear();
        
        // Update keymap config
        *self.keymap_config.write() = config.keymap;
        
        Ok(())
    }

    pub fn add_custom_keybinding(
        &mut self,
        key_str: &str,
        action: InputAction,
        context: KeyBindingContext,
        priority: u8,
    ) -> Result<(), InputError> {
        let key_binding = Self::parse_key_binding(key_str, context)?;
        
        self.keybindings.write().insert(key_binding, KeyBindingAction {
            action,
            priority,
            condition: None,
        });
        
        // Clear cache to ensure new binding is recognized
        self.key_lookup_cache.lock().clear();
        
        Ok(())
    }

    pub fn remove_keybinding(&mut self, key_str: &str, context: KeyBindingContext) -> Result<bool, InputError> {
        let key_binding = Self::parse_key_binding(key_str, context)?;
        let removed = self.keybindings.write().remove(&key_binding).is_some();
        
        if removed {
            self.key_lookup_cache.lock().clear();
        }
        
        Ok(removed)
    }

    fn string_to_action(&self, action_str: &str) -> Option<InputAction> {
        match action_str.trim() {
            // Control actions
            "interrupt" => Some(InputAction::Interrupt),
            "eof" => Some(InputAction::Eof),
            "suspend" => Some(InputAction::Suspend),
            "resume" => Some(InputAction::Resume),
            
            // Clipboard actions
            "copy" => Some(InputAction::Copy),
            "paste" => Some(InputAction::Paste),
            "cut" => Some(InputAction::Cut),
            "select_all" => Some(InputAction::SelectAll),
            
            // Screen actions
            "clear" => Some(InputAction::Clear),
            "clear_line" => Some(InputAction::ClearLine),
            
            // Scrolling actions
            "scroll_up" => Some(InputAction::ScrollUp),
            "scroll_down" => Some(InputAction::ScrollDown),
            "scroll_page_up" => Some(InputAction::ScrollPageUp),
            "scroll_page_down" => Some(InputAction::ScrollPageDown),
            "scroll_to_top" => Some(InputAction::ScrollToTop),
            "scroll_to_bottom" => Some(InputAction::ScrollToBottom),
            
            // Navigation actions
            "word_back" => Some(InputAction::WordBack),
            "word_forward" => Some(InputAction::WordForward),
            "line_start" => Some(InputAction::LineStart),
            "line_end" => Some(InputAction::LineEnd),
            
            // Deletion actions
            "delete_word" => Some(InputAction::DeleteWord),
            "delete_to_end" => Some(InputAction::DeleteToEnd),
            "delete_to_start" => Some(InputAction::DeleteToStart),
            
            // History actions
            "history_prev" => Some(InputAction::HistoryPrev),
            "history_next" => Some(InputAction::HistoryNext),
            "history_search" => Some(InputAction::HistorySearch),
            
            // Window management
            "new_window" => Some(InputAction::NewWindow),
            "close_window" => Some(InputAction::CloseWindow),
            "next_window" => Some(InputAction::NextWindow),
            "prev_window" => Some(InputAction::PrevWindow),
            
            // Custom actions with parameters
            _ if action_str.starts_with("custom:") => {
                let parts: Vec<&str> = action_str.strip_prefix("custom:").unwrap().split(':').collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(InputAction::Custom(
                        parts[0].to_string(),
                        parts[1..].iter().map(|s| s.to_string()).collect(),
                    ))
                }
            },
            _ => {
                eprintln!("Warning: Unknown action '{}', treating as custom", action_str);
                Some(InputAction::Custom(action_str.to_string(), vec![]))
            }
        }
    }

    // Public API methods
    pub async fn receive_action(&mut self) -> Option<InputAction> {
        self.action_receiver.recv().await
    }

    pub fn get_command_buffer(&self) -> String {
        self.prefix_state.lock().buffer.clone()
    }

    pub fn is_prefix_mode(&self) -> bool {
        self.is_prefix_active()
    }

    pub fn cancel_command(&mut self) {
        let mut prefix_state = self.prefix_state.lock();
        prefix_state.detected = false;
        prefix_state.buffer.clear();
        prefix_state.escape_mode = false;
        prefix_state.start_time = None;
    }

    pub fn set_shell_mode(&mut self, mode: ShellMode) {
        self.input_state.lock().shell_mode = mode;
        // Clear keybinding cache since context affects resolution
        self.key_lookup_cache.lock().clear();
    }

    pub fn get_shell_mode(&self) -> ShellMode {
        self.input_state.lock().shell_mode.clone()
    }

    pub fn get_input_stats(&self) -> InputStats {
        self.stats.lock().clone()
    }

    pub fn reset_stats(&mut self) {
        *self.stats.lock() = InputStats::default();
    }

    pub fn clear_cache(&mut self) {
        self.key_lookup_cache.lock().clear();
    }

    pub fn get_cache_size(&self) -> usize {
        self.key_lookup_cache.lock().len()
    }

    // Utility methods for testing and debugging
    pub fn simulate_key_event(&mut self, key: Key, modifiers: Vec<Modifier>, text: Option<String>) -> KeyEvent {
        KeyEvent {
            key,
            modifiers: modifiers.into_iter().collect(),
            text,
            repeat: false,
            timestamp: Instant::now(),
            key_code: None,
        }
    }

    pub fn set_prefix_timeout(&mut self, timeout_ms: u64) {
        self.prefix_state.lock().timeout_ms = timeout_ms;
    }

    pub fn list_active_keybindings(&self) -> Vec<(String, String)> {
        let keybindings = self.keybindings.read();
        let context = self.get_current_context();
        
        keybindings
            .iter()
            .filter(|(kb, _)| kb.context == context || kb.context == KeyBindingContext::Global)
            .map(|(kb, action)| {
                (
                    self.keybinding_to_string(kb),
                    format!("{:?}", action.action)
                )
            })
            .collect()
    }

    fn keybinding_to_string(&self, kb: &KeyBinding) -> String {
        let mut parts = Vec::new();
        
        // Add modifiers in consistent order
        if kb.modifiers.contains(&Modifier::Ctrl) { parts.push("ctrl"); }
        if kb.modifiers.contains(&Modifier::Alt) { parts.push("alt"); }
        if kb.modifiers.contains(&Modifier::Shift) { parts.push("shift"); }
        if kb.modifiers.contains(&Modifier::Super) { parts.push("super"); }
        if kb.modifiers.contains(&Modifier::Meta) { parts.push("meta"); }
        if kb.modifiers.contains(&Modifier::Hyper) { parts.push("hyper"); }

        // Add key
        let key_str = match kb.key {
            Key::Char(c) => c.to_string(),
            Key::Space => "space".to_string(),
            Key::Enter => "enter".to_string(),
            Key::Tab => "tab".to_string(),
            Key::Backspace => "backspace".to_string(),
            Key::Delete => "delete".to_string(),
            Key::Escape => "escape".to_string(),
            Key::Up => "up".to_string(),
            Key::Down => "down".to_string(),
            Key::Left => "left".to_string(),
            Key::Right => "right".to_string(),
            Key::Home => "home".to_string(),
            Key::End => "end".to_string(),
            Key::PageUp => "pageup".to_string(),
            Key::PageDown => "pagedown".to_string(),
            Key::Insert => "insert".to_string(),
            Key::F1 => "f1".to_string(), Key::F2 => "f2".to_string(),
            Key::F3 => "f3".to_string(), Key::F4 => "f4".to_string(),
            Key::F5 => "f5".to_string(), Key::F6 => "f6".to_string(),
            Key::F7 => "f7".to_string(), Key::F8 => "f8".to_string(),
            Key::F9 => "f9".to_string(), Key::F10 => "f10".to_string(),
            Key::F11 => "f11".to_string(), Key::F12 => "f12".to_string(),
            Key::F13 => "f13".to_string(), Key::F14 => "f14".to_string(),
            Key::F15 => "f15".to_string(), Key::F16 => "f16".to_string(),
            Key::F17 => "f17".to_string(), Key::F18 => "f18".to_string(),
            Key::F19 => "f19".to_string(), Key::F20 => "f20".to_string(),
            Key::F21 => "f21".to_string(), Key::F22 => "f22".to_string(),
            Key::F23 => "f23".to_string(), Key::F24 => "f24".to_string(),
            _ => format!("{:?}", kb.key).to_lowercase(),
        };

        parts.push(&key_str);
        parts.join("+")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn create_test_processor() -> InputProcessor {
        let keymap_config = Arc::new(RwLock::new(crate::config::KeymapConfig::default()));
        let command_parser = Arc::new(RwLock::new(CommandParser::new("p".to_string())));
        let config_manager = Arc::new(ConfigManager::new().unwrap());
        
        InputProcessor::new(keymap_config, command_parser, config_manager)
    }

    #[test]
    fn test_key_binding_parsing() {
        assert!(InputProcessor::parse_key_binding("ctrl+c", KeyBindingContext::Global).is_ok());
        assert!(InputProcessor::parse_key_binding("ctrl+shift+f", KeyBindingContext::Global).is_ok());
        assert!(InputProcessor::parse_key_binding("alt+f1", KeyBindingContext::Global).is_ok());
        assert!(InputProcessor::parse_key_binding("super+space", KeyBindingContext::Global).is_ok());
        assert!(InputProcessor::parse_key_binding("invalid+key", KeyBindingContext::Global).is_err());
    }

    #[test]
    fn test_keybinding_context_resolution() {
        let processor = create_test_processor();
        
        // Test that global context is default
        assert!(matches!(processor.get_current_context(), KeyBindingContext::Shell | KeyBindingContext::Emacs));
    }

    #[tokio::test]
    async fn test_prefix_detection() {
        let mut processor = create_test_processor();

        // Send prefix character at line start
        let event = processor.simulate_key_event(Key::Char('p'), vec![], Some("p".to_string()));
        
        processor.process_key_event(event).await.unwrap();
        assert!(processor.is_prefix_mode());
        assert_eq!(processor.get_command_buffer(), "");
    }

    #[tokio::test]
    async fn test_command_execution() {
        let mut processor = create_test_processor();

        // Enter prefix mode
        let prefix_event = processor.simulate_key_event(Key::Char('f'), vec![], Some("f".to_string()));
        processor.process_key_event(prefix_event).await.unwrap();

        // Type command
        let h_event = processor.simulate_key_event(Key::Char('h'), vec![], Some("h".to_string()));
        processor.process_key_event(h_event).await.unwrap();

        let e_event = processor.simulate_key_event(Key::Char('e'), vec![], Some("e".to_string()));
        processor.process_key_event(e_event).await.unwrap();

        assert_eq!(processor.get_command_buffer(), "he");

        // Execute command
        let enter_event = processor.simulate_key_event(Key::Enter, vec![], None);
        processor.process_key_event(enter_event).await.unwrap();

        assert!(!processor.is_prefix_mode());
        assert_eq!(processor.get_command_buffer(), "");
    }

    #[tokio::test]
    async fn test_escape_sequence() {
        let mut processor = create_test_processor();

        // Start escape sequence at line start
        let backslash_event = processor.simulate_key_event(Key::Char('\\'), vec![], Some("\\".to_string()));
        processor.process_key_event(backslash_event).await.unwrap();

        // Send prefix character (should be literal)
        let p_event = processor.simulate_key_event(Key::Char('p'), vec![], Some("p".to_string()));
        processor.process_key_event(p_event).await.unwrap();

        assert!(!processor.is_prefix_mode());
    }

    #[tokio::test]
    async fn test_keybinding_priority() {
        let mut processor = create_test_processor();
        
        // Add conflicting keybindings with different priorities
        processor.add_custom_keybinding(
            "ctrl+t", 
            InputAction::Copy, 
            KeyBindingContext::Global, 
            50
        ).unwrap();
        
        processor.add_custom_keybinding(
            "ctrl+t", 
            InputAction::Paste, 
            KeyBindingContext::Emacs, 
            100
        ).unwrap();

        let event = processor.simulate_key_event(
            Key::Char('t'), 
            vec![Modifier::Ctrl], 
            None
        );
        
        // Higher priority should win
        if let Some(action) = processor.resolve_keybinding(&event).unwrap() {
            assert!(matches!(action, InputAction::Paste));
        }
    }

    #[test]
    fn test_performance_key_lookup() {
        let processor = create_test_processor();
        let event = processor.simulate_key_event(
            Key::Char('c'), 
            vec![Modifier::Ctrl], 
            None
        );

        // Measure lookup time
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = processor.resolve_keybinding(&event);
        }
        let duration = start.elapsed();

        // Should be well under 1ms for 1000 lookups (< 1μs per lookup)
        assert!(duration.as_millis() < 10, "Keybinding lookup too slow: {}ms", duration.as_millis());
    }

    #[test]
    fn test_cache_effectiveness() {
        let mut processor = create_test_processor();
        let event = processor.simulate_key_event(
            Key::Char('c'), 
            vec![Modifier::Ctrl], 
            None
        );

        // First lookup should miss cache
        let _ = processor.resolve_keybinding(&event);
        let stats = processor.get_input_stats();
        assert!(stats.cache_misses > 0);

        // Second lookup should hit cache
        let _ = processor.resolve_keybinding(&event);
        let stats_after = processor.get_input_stats();
        assert!(stats_after.cache_hits > stats.cache_hits);
    }

    #[test]
    fn test_shell_mode_detection() {
        std::env::set_var("EDITOR", "vim");
        let processor = create_test_processor();
        
        let detected_mode = processor.detect_shell_mode();
        assert!(matches!(detected_mode, ShellMode::Vi));
        
        std::env::remove_var("EDITOR");
    }

    #[tokio::test]
    async fn test_paste_mode_handling() {
        let mut processor = create_test_processor();
        
        let paste_start = processor.simulate_key_event(
            Key::Char('\x1b'), 
            vec![], 
            Some("\x1b[200~hello world\x1b[201~".to_string())
        );
        
        processor.process_key_event(paste_start).await.unwrap();
        
        // Paste mode should be reset after handling
        assert!(!processor.input_state.lock().in_paste_mode);
    }

    #[test]
    fn test_input_stats() {
        let mut processor = create_test_processor();
        
        let initial_stats = processor.get_input_stats();
        assert_eq!(initial_stats.total_keys_processed, 0);
        
        processor.reset_stats();
        let reset_stats = processor.get_input_stats();
        assert_eq!(reset_stats.total_keys_processed, 0);
    }

    #[test]
    fn test_custom_keybinding_management() {
        let mut processor = create_test_processor();
        
        // Add custom keybinding
        processor.add_custom_keybinding(
            "ctrl+shift+t", 
            InputAction::NewWindow, 
            KeyBindingContext::Global, 
            90
        ).unwrap();
        
        // Remove keybinding
        let removed = processor.remove_keybinding("ctrl+shift+t", KeyBindingContext::Global).unwrap();
        assert!(removed);
        
        // Try to remove non-existent keybinding
        let not_removed = processor.remove_keybinding("ctrl+shift+z", KeyBindingContext::Global).unwrap();
        assert!(!not_removed);
    }
}
