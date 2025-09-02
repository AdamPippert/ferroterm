use parking_lot::RwLock as ParkingRwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::ConfigManager;
use crate::input::{InputError, InputProcessor, Key, KeyEvent, Modifier};
use crate::renderer::{GpuRenderer, RendererError};
use crate::tty::{PtyConfig, TtyEngine, TtyError};

#[derive(Error, Debug)]
pub enum MultiplexerError {
    #[error("TTY error: {0}")]
    Tty(#[from] TtyError),
    #[error("Renderer error: {0}")]
    Renderer(#[from] RendererError),
    #[error("Input error: {0}")]
    Input(#[from] InputError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Pane not found: {id}")]
    PaneNotFound { id: u64 },
    #[error("Window not found: {id}")]
    WindowNotFound { id: u64 },
    #[error("Session not found: {name}")]
    SessionNotFound { name: String },
    #[error("Invalid layout: {reason}")]
    InvalidLayout { reason: String },
    #[error("Maximum panes exceeded: {max}")]
    MaxPanesExceeded { max: usize },
    #[error("Command not found: {cmd}")]
    CommandNotFound { cmd: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutAlgorithm {
    Tiled,
    EvenHorizontal,
    EvenVertical,
    MainVertical,
    MainHorizontal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneLayout {
    pub id: u64,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub split_direction: Option<SplitDirection>,
    pub children: Vec<PaneLayout>,
}

impl PaneLayout {
    pub fn new(id: u64, x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            id,
            x,
            y,
            width,
            height,
            split_direction: None,
            children: Vec::new(),
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    pub fn find_pane(&self, id: u64) -> Option<&PaneLayout> {
        if self.id == id {
            return Some(self);
        }
        for child in &self.children {
            if let Some(pane) = child.find_pane(id) {
                return Some(pane);
            }
        }
        None
    }

    pub fn find_pane_mut(&mut self, id: u64) -> Option<&mut PaneLayout> {
        if self.id == id {
            return Some(self);
        }
        for child in &mut self.children {
            if let Some(pane) = child.find_pane_mut(id) {
                return Some(pane);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: u64,
    pub pty_id: u64,
    pub layout: PaneLayout,
    pub is_active: bool,
    pub title: String,
    pub created_at: u64,
    pub last_activity: u64,
}

impl Pane {
    pub fn new(id: u64, pty_id: u64, x: u32, y: u32, width: u32, height: u32) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let layout = PaneLayout::new(id, x, y, width, height);
        Self {
            id,
            pty_id,
            layout,
            is_active: false,
            title: format!("Pane {}", id),
            created_at: now,
            last_activity: now,
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    pub fn resize(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.layout.x = x;
        self.layout.y = y;
        self.layout.width = width;
        self.layout.height = height;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    pub id: u64,
    pub name: String,
    pub panes: HashMap<u64, Pane>,
    pub active_pane_id: Option<u64>,
    pub layout: PaneLayout,
    pub layout_algorithm: LayoutAlgorithm,
    pub created_at: u64,
}

impl Window {
    pub fn new(id: u64, name: String, width: u32, height: u32) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let layout = PaneLayout::new(0, 0, 0, width, height);
        Self {
            id,
            name,
            panes: HashMap::new(),
            active_pane_id: None,
            layout,
            layout_algorithm: LayoutAlgorithm::Tiled,
            created_at: now,
        }
    }

    pub fn add_pane(&mut self, pane: Pane) {
        if self.panes.is_empty() {
            self.active_pane_id = Some(pane.id);
        }
        self.panes.insert(pane.id, pane);
        self.recalculate_layout();
    }

    pub fn remove_pane(&mut self, pane_id: u64) -> Option<Pane> {
        let removed = self.panes.remove(&pane_id);
        if self.active_pane_id == Some(pane_id) {
            self.active_pane_id = self.panes.keys().next().copied();
        }
        if removed.is_some() {
            self.recalculate_layout();
        }
        removed
    }

    pub fn get_active_pane(&self) -> Option<&Pane> {
        self.active_pane_id.and_then(|id| self.panes.get(&id))
    }

    pub fn get_active_pane_mut(&mut self) -> Option<&mut Pane> {
        if let Some(id) = self.active_pane_id {
            self.panes.get_mut(&id)
        } else {
            None
        }
    }

    pub fn set_active_pane(&mut self, pane_id: u64) -> Result<(), MultiplexerError> {
        if self.panes.contains_key(&pane_id) {
            self.active_pane_id = Some(pane_id);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.is_active = true;
                pane.update_activity();
            }
            // Deactivate other panes
            for (id, pane) in &mut self.panes {
                if *id != pane_id {
                    pane.is_active = false;
                }
            }
            Ok(())
        } else {
            Err(MultiplexerError::PaneNotFound { id: pane_id })
        }
    }

    pub fn recalculate_layout(&mut self) {
        if self.panes.is_empty() {
            return;
        }

        let pane_ids: Vec<u64> = self.panes.keys().copied().collect();
        self.layout.children.clear();

        match self.layout_algorithm {
            LayoutAlgorithm::Tiled => self.layout_tiled(&pane_ids),
            LayoutAlgorithm::EvenHorizontal => self.layout_even_horizontal(&pane_ids),
            LayoutAlgorithm::EvenVertical => self.layout_even_vertical(&pane_ids),
            LayoutAlgorithm::MainVertical => self.layout_main_vertical(&pane_ids),
            LayoutAlgorithm::MainHorizontal => self.layout_main_horizontal(&pane_ids),
        }
    }

    fn layout_tiled(&mut self, pane_ids: &[u64]) {
        let count = pane_ids.len();
        if count == 0 {
            return;
        }

        let cols = ((count as f32).sqrt()).ceil() as u32;
        let rows = (count + cols as usize - 1) / cols as usize;

        for (i, &pane_id) in pane_ids.iter().enumerate() {
            let row = i / cols as usize;
            let col = i % cols as usize;

            let width = self.layout.width / cols;
            let height = self.layout.height / rows as u32;
            let x = col as u32 * width;
            let y = row as u32 * height;

            let pane_layout = PaneLayout::new(pane_id, x, y, width, height);
            self.layout.children.push(pane_layout);
        }
    }

    fn layout_even_horizontal(&mut self, pane_ids: &[u64]) {
        let count = pane_ids.len() as u32;
        if count == 0 {
            return;
        }

        let height = self.layout.height / count;
        for (i, &pane_id) in pane_ids.iter().enumerate() {
            let y = i as u32 * height;
            let pane_layout = PaneLayout::new(pane_id, 0, y, self.layout.width, height);
            self.layout.children.push(pane_layout);
        }
    }

    fn layout_even_vertical(&mut self, pane_ids: &[u64]) {
        let count = pane_ids.len() as u32;
        if count == 0 {
            return;
        }

        let width = self.layout.width / count;
        for (i, &pane_id) in pane_ids.iter().enumerate() {
            let x = i as u32 * width;
            let pane_layout = PaneLayout::new(pane_id, x, 0, width, self.layout.height);
            self.layout.children.push(pane_layout);
        }
    }

    fn layout_main_vertical(&mut self, pane_ids: &[u64]) {
        if pane_ids.is_empty() {
            return;
        }

        // Main pane takes 2/3 of the space
        let main_width = (self.layout.width * 2) / 3;
        let side_width = self.layout.width - main_width;

        // Main pane
        let main_layout = PaneLayout::new(pane_ids[0], 0, 0, main_width, self.layout.height);
        self.layout.children.push(main_layout);

        // Side panes
        if pane_ids.len() > 1 {
            let side_height = self.layout.height / (pane_ids.len() - 1) as u32;
            for (i, &pane_id) in pane_ids.iter().enumerate().skip(1) {
                let y = (i - 1) as u32 * side_height;
                let side_layout = PaneLayout::new(pane_id, main_width, y, side_width, side_height);
                self.layout.children.push(side_layout);
            }
        }
    }

    fn layout_main_horizontal(&mut self, pane_ids: &[u64]) {
        if pane_ids.is_empty() {
            return;
        }

        // Main pane takes 2/3 of the space
        let main_height = (self.layout.height * 2) / 3;
        let side_height = self.layout.height - main_height;

        // Main pane
        let main_layout = PaneLayout::new(pane_ids[0], 0, 0, self.layout.width, main_height);
        self.layout.children.push(main_layout);

        // Side panes
        if pane_ids.len() > 1 {
            let side_width = self.layout.width / (pane_ids.len() - 1) as u32;
            for (i, &pane_id) in pane_ids.iter().enumerate().skip(1) {
                let x = (i - 1) as u32 * side_width;
                let side_layout = PaneLayout::new(pane_id, x, main_height, side_width, side_height);
                self.layout.children.push(side_layout);
            }
        }
    }

    pub fn split_pane(
        &mut self,
        pane_id: u64,
        direction: SplitDirection,
    ) -> Result<u64, MultiplexerError> {
        let layout = {
            let pane = self
                .panes
                .get(&pane_id)
                .ok_or(MultiplexerError::PaneNotFound { id: pane_id })?;
            pane.layout.clone()
        };

        // Create new pane
        let new_pane_id = self.panes.keys().max().unwrap_or(&0) + 1;
        let (new_x, new_y, new_width, new_height) = match direction {
            SplitDirection::Horizontal => {
                let half_height = layout.height / 2;
                (layout.x, layout.y + half_height, layout.width, half_height)
            }
            SplitDirection::Vertical => {
                let half_width = layout.width / 2;
                (layout.x + half_width, layout.y, half_width, layout.height)
            }
        };

        let mut new_pane = Pane::new(new_pane_id, 0, new_x, new_y, new_width, new_height);
        new_pane.title = format!("Pane {}", new_pane_id);

        // Resize existing pane
        let existing_pane = self.panes.get_mut(&pane_id).unwrap();
        match direction {
            SplitDirection::Horizontal => {
                existing_pane.resize(layout.x, layout.y, layout.width, layout.height / 2);
            }
            SplitDirection::Vertical => {
                existing_pane.resize(layout.x, layout.y, layout.width / 2, layout.height);
            }
        }

        self.add_pane(new_pane);
        Ok(new_pane_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub windows: HashMap<u64, Window>,
    pub active_window_id: Option<u64>,
    pub created_at: u64,
    pub last_activity: u64,
    pub is_detached: bool,
    pub working_directory: PathBuf,
}

impl Session {
    pub fn new(name: String, working_directory: PathBuf) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            name,
            windows: HashMap::new(),
            active_window_id: None,
            created_at: now,
            last_activity: now,
            is_detached: false,
            working_directory,
        }
    }

    pub fn add_window(&mut self, window: Window) {
        if self.windows.is_empty() {
            self.active_window_id = Some(window.id);
        }
        self.windows.insert(window.id, window);
    }

    pub fn remove_window(&mut self, window_id: u64) -> Option<Window> {
        let removed = self.windows.remove(&window_id);
        if self.active_window_id == Some(window_id) {
            self.active_window_id = self.windows.keys().next().copied();
        }
        removed
    }

    pub fn get_active_window(&self) -> Option<&Window> {
        self.active_window_id.and_then(|id| self.windows.get(&id))
    }

    pub fn get_active_window_mut(&mut self) -> Option<&mut Window> {
        if let Some(id) = self.active_window_id {
            self.windows.get_mut(&id)
        } else {
            None
        }
    }

    pub fn set_active_window(&mut self, window_id: u64) -> Result<(), MultiplexerError> {
        if self.windows.contains_key(&window_id) {
            self.active_window_id = Some(window_id);
            self.last_activity = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            Ok(())
        } else {
            Err(MultiplexerError::WindowNotFound { id: window_id })
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

#[derive(Debug, Clone)]
pub struct MultiplexerConfig {
    pub max_panes_per_window: usize,
    pub max_windows_per_session: usize,
    pub default_layout: LayoutAlgorithm,
    pub session_directory: PathBuf,
    pub prefix_key: Key,
    pub prefix_modifier: Option<Modifier>,
}

impl Default for MultiplexerConfig {
    fn default() -> Self {
        Self {
            max_panes_per_window: 16,
            max_windows_per_session: 10,
            default_layout: LayoutAlgorithm::Tiled,
            session_directory: PathBuf::from("~/.pachyterm/sessions"),
            prefix_key: Key::Char('b'),
            prefix_modifier: Some(Modifier::Ctrl),
        }
    }
}

pub struct Multiplexer {
    tty_engine: Arc<TtyEngine>,
    renderer: Arc<RwLock<GpuRenderer>>,
    input_processor: Arc<RwLock<InputProcessor>>,
    config_manager: Arc<ConfigManager>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    active_session: Arc<RwLock<Option<String>>>,
    config: MultiplexerConfig,
    next_pane_id: Arc<Mutex<u64>>,
    next_window_id: Arc<Mutex<u64>>,
    prefix_mode: Arc<RwLock<bool>>,
    command_buffer: Arc<RwLock<String>>,
}

impl Multiplexer {
    pub fn new(
        tty_engine: Arc<TtyEngine>,
        renderer: Arc<RwLock<GpuRenderer>>,
        input_processor: Arc<RwLock<InputProcessor>>,
        config_manager: Arc<ConfigManager>,
    ) -> Self {
        let config = MultiplexerConfig::default();
        Self {
            tty_engine,
            renderer,
            input_processor,
            config_manager,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            active_session: Arc::new(RwLock::new(None)),
            config,
            next_pane_id: Arc::new(Mutex::new(1)),
            next_window_id: Arc::new(Mutex::new(1)),
            prefix_mode: Arc::new(RwLock::new(false)),
            command_buffer: Arc::new(RwLock::new(String::new())),
        }
    }

    pub async fn create_session(&self, name: String) -> Result<(), MultiplexerError> {
        let working_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut session = Session::new(name.clone(), working_directory);

        // Create initial window
        let window_id = self.get_next_window_id();
        let mut window = Window::new(window_id, "Window 1".to_string(), 80, 24);
        window.layout_algorithm = self.config.default_layout;

        // Create initial pane
        let pane_id = self.get_next_pane_id();
        let pty_config = PtyConfig::default();
        let pty_id = self.tty_engine.create_pty(pty_config).await?;
        let pane = Pane::new(pane_id, pty_id, 0, 0, 80, 24);
        window.add_pane(pane);

        session.add_window(window);
        self.sessions.write().await.insert(name.clone(), session);
        *self.active_session.write().await = Some(name);

        info!("Created new session with initial window and pane");
        Ok(())
    }

    pub async fn attach_session(&self, name: &str) -> Result<(), MultiplexerError> {
        if self.sessions.read().await.contains_key(name) {
            *self.active_session.write().await = Some(name.to_string());
            if let Some(session) = self.sessions.write().await.get_mut(name) {
                session.is_detached = false;
                session.update_activity();
            }
            info!("Attached to session: {}", name);
            Ok(())
        } else {
            Err(MultiplexerError::SessionNotFound {
                name: name.to_string(),
            })
        }
    }

    pub async fn detach_session(&self, name: &str) -> Result<(), MultiplexerError> {
        if let Some(session) = self.sessions.write().await.get_mut(name) {
            session.is_detached = true;
            info!("Detached from session: {}", name);
            Ok(())
        } else {
            Err(MultiplexerError::SessionNotFound {
                name: name.to_string(),
            })
        }
    }

    pub async fn save_session(&self, name: &str) -> Result<(), MultiplexerError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(name) {
            let session_path = self.config.session_directory.join(format!("{}.json", name));
            std::fs::create_dir_all(&self.config.session_directory)?;

            let serialized = serde_json::to_string_pretty(session)?;
            std::fs::write(session_path, serialized)?;

            info!("Saved session: {}", name);
            Ok(())
        } else {
            Err(MultiplexerError::SessionNotFound {
                name: name.to_string(),
            })
        }
    }

    pub async fn load_session(&self, name: &str) -> Result<(), MultiplexerError> {
        let session_path = self.config.session_directory.join(format!("{}.json", name));

        if !session_path.exists() {
            return Err(MultiplexerError::SessionNotFound {
                name: name.to_string(),
            });
        }

        let content = std::fs::read_to_string(session_path)?;
        let mut session: Session = serde_json::from_str(&content)?;

        // Recreate PTYs for all panes
        for window in session.windows.values_mut() {
            for pane in window.panes.values_mut() {
                let pty_config = PtyConfig::default();
                pane.pty_id = self.tty_engine.create_pty(pty_config).await?;
            }
        }

        self.sessions
            .write()
            .await
            .insert(name.to_string(), session);
        info!("Loaded session: {}", name);
        Ok(())
    }

    pub async fn split_pane(&self, direction: SplitDirection) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_name) {
            if let Some(window) = session.get_active_window_mut() {
                if let Some(active_pane) = window.get_active_pane() {
                    if window.panes.len() >= self.config.max_panes_per_window {
                        return Err(MultiplexerError::MaxPanesExceeded {
                            max: self.config.max_panes_per_window,
                        });
                    }

                    let active_pane_id = active_pane.id;
                    let new_pane_id = window.split_pane(active_pane_id, direction)?;

                    // Create PTY for new pane
                    let pty_config = PtyConfig::default();
                    let pty_id = self.tty_engine.create_pty(pty_config).await?;

                    if let Some(new_pane) = window.panes.get_mut(&new_pane_id) {
                        new_pane.pty_id = pty_id;
                    }

                    info!("Split pane {} in direction {:?}", active_pane_id, direction);
                }
            }
        }

        Ok(())
    }

    pub async fn switch_pane(&self, direction: PaneDirection) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_name) {
            if let Some(window) = session.get_active_window_mut() {
                if let Some(active_pane) = window.get_active_pane() {
                    let next_pane_id = self.find_adjacent_pane(window, active_pane.id, direction);
                    if let Some(next_id) = next_pane_id {
                        window.set_active_pane(next_id)?;
                        info!("Switched to pane {}", next_id);
                    }
                }
            }
        }

        Ok(())
    }

    fn find_adjacent_pane(
        &self,
        window: &Window,
        current_pane_id: u64,
        direction: PaneDirection,
    ) -> Option<u64> {
        let current_pane = window.panes.get(&current_pane_id)?;
        let current_layout = &current_pane.layout;

        let mut closest_pane = None;
        let mut min_distance = f32::INFINITY;

        for (pane_id, pane) in &window.panes {
            if *pane_id == current_pane_id {
                continue;
            }

            let layout = &pane.layout;
            let distance = match direction {
                PaneDirection::Up => {
                    if layout.y < current_layout.y {
                        (current_layout.y - layout.y) as f32
                            + (current_layout.x as f32 - layout.x as f32).abs()
                    } else {
                        f32::INFINITY
                    }
                }
                PaneDirection::Down => {
                    if layout.y > current_layout.y {
                        (layout.y - current_layout.y) as f32
                            + (current_layout.x as f32 - layout.x as f32).abs()
                    } else {
                        f32::INFINITY
                    }
                }
                PaneDirection::Left => {
                    if layout.x < current_layout.x {
                        (current_layout.x - layout.x) as f32
                            + (current_layout.y as f32 - layout.y as f32).abs()
                    } else {
                        f32::INFINITY
                    }
                }
                PaneDirection::Right => {
                    if layout.x > current_layout.x {
                        (layout.x - current_layout.x) as f32
                            + (current_layout.y as f32 - layout.y as f32).abs()
                    } else {
                        f32::INFINITY
                    }
                }
            };

            if distance < min_distance {
                min_distance = distance;
                closest_pane = Some(*pane_id);
            }
        }

        closest_pane
    }

    pub async fn create_window(&self, name: Option<String>) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let window_name = name.unwrap_or_else(|| format!("Window {}", self.get_next_window_id()));
        let window_id = self.get_next_window_id();

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_name) {
            if session.windows.len() >= self.config.max_windows_per_session {
                return Err(MultiplexerError::MaxPanesExceeded {
                    max: self.config.max_windows_per_session,
                });
            }

            let mut window = Window::new(window_id, window_name, 80, 24);
            window.layout_algorithm = self.config.default_layout;

            // Create initial pane
            let pane_id = self.get_next_pane_id();
            let pty_config = PtyConfig::default();
            let pty_id = self.tty_engine.create_pty(pty_config).await?;
            let pane = Pane::new(pane_id, pty_id, 0, 0, 80, 24);
            window.add_pane(pane);

            session.add_window(window);
            info!("Created new window: {}", window_id);
        }

        Ok(())
    }

    pub async fn switch_window(&self, window_id: u64) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_name) {
            session.set_active_window(window_id)?;
            info!("Switched to window {}", window_id);
        }

        Ok(())
    }

    pub async fn handle_key_event(&self, event: KeyEvent) -> Result<(), MultiplexerError> {
        // Check for prefix key combination
        if self.is_prefix_key(&event) {
            *self.prefix_mode.write().await = true;
            *self.command_buffer.write().await = String::new();
            return Ok(());
        }

        // If in prefix mode, handle multiplexer commands
        if *self.prefix_mode.read().await {
            return self.handle_multiplexer_command(event).await;
        }

        // Otherwise, pass through to active pane
        self.send_to_active_pane(&event).await?;
        Ok(())
    }

    fn is_prefix_key(&self, event: &KeyEvent) -> bool {
        if event.key != self.config.prefix_key {
            return false;
        }

        match self.config.prefix_modifier {
            Some(modifier) => event.modifiers.contains(&modifier),
            None => event.modifiers.is_empty(),
        }
    }

    async fn handle_multiplexer_command(&self, event: KeyEvent) -> Result<(), MultiplexerError> {
        let mut command_buffer = self.command_buffer.write().await;

        match event.key {
            Key::Char('%') => {
                // Vertical split
                self.split_pane(SplitDirection::Vertical).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('"') => {
                // Horizontal split
                self.split_pane(SplitDirection::Horizontal).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('c') => {
                // Create window
                self.create_window(None).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('n') => {
                // Next window
                self.next_window().await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('p') => {
                // Previous window
                self.previous_window().await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Up => {
                // Switch to pane above
                self.switch_pane(PaneDirection::Up).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Down => {
                // Switch to pane below
                self.switch_pane(PaneDirection::Down).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Left => {
                // Switch to pane left
                self.switch_pane(PaneDirection::Left).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Right => {
                // Switch to pane right
                self.switch_pane(PaneDirection::Right).await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('d') => {
                // Detach session
                if let Some(session_name) = &*self.active_session.read().await {
                    self.detach_session(session_name).await?;
                }
                *self.prefix_mode.write().await = false;
            }
            Key::Char('z') => {
                // Zoom pane (toggle fullscreen)
                self.zoom_pane().await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('x') => {
                // Kill pane
                self.kill_pane().await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Char('?') => {
                // Show help
                self.show_help().await?;
                *self.prefix_mode.write().await = false;
            }
            Key::Escape => {
                // Cancel command
                *self.prefix_mode.write().await = false;
                command_buffer.clear();
            }
            _ => {
                // Unknown command, ignore
            }
        }

        Ok(())
    }

    async fn send_to_active_pane(&self, event: &KeyEvent) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone();
        if let Some(session_name) = session_name {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(&session_name) {
                if let Some(window) = session.get_active_window() {
                    if let Some(pane) = window.get_active_pane() {
                        // Convert KeyEvent to string and send to PTY
                        if let Some(text) = &event.text {
                            self.tty_engine
                                .write_to_pty(pane.pty_id, text.as_bytes())
                                .await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn next_window(&self) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&session_name) {
            let window_ids: Vec<u64> = session.windows.keys().copied().collect();
            if let Some(current_id) = session.active_window_id {
                if let Some(current_index) = window_ids.iter().position(|&id| id == current_id) {
                    let next_index = (current_index + 1) % window_ids.len();
                    let next_id = window_ids[next_index];
                    drop(sessions);
                    self.switch_window(next_id).await?;
                }
            }
        }
        Ok(())
    }

    async fn previous_window(&self) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&session_name) {
            let window_ids: Vec<u64> = session.windows.keys().copied().collect();
            if let Some(current_id) = session.active_window_id {
                if let Some(current_index) = window_ids.iter().position(|&id| id == current_id) {
                    let prev_index = if current_index == 0 {
                        window_ids.len() - 1
                    } else {
                        current_index - 1
                    };
                    let prev_id = window_ids[prev_index];
                    drop(sessions);
                    self.switch_window(prev_id).await?;
                }
            }
        }
        Ok(())
    }

    async fn zoom_pane(&self) -> Result<(), MultiplexerError> {
        // TODO: Implement pane zooming (fullscreen toggle)
        warn!("Pane zooming not yet implemented");
        Ok(())
    }

    async fn kill_pane(&self) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone().ok_or_else(|| {
            MultiplexerError::SessionNotFound {
                name: "no active session".to_string(),
            }
        })?;

        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&session_name) {
            if let Some(window) = session.get_active_window_mut() {
                if let Some(active_pane) = window.get_active_pane() {
                    let pane_id = active_pane.id;
                    let pty_id = active_pane.pty_id;

                    window.remove_pane(pane_id);
                    self.tty_engine.destroy_pty(pty_id).await?;

                    info!("Killed pane {}", pane_id);
                }
            }
        }
        Ok(())
    }

    async fn show_help(&self) -> Result<(), MultiplexerError> {
        // TODO: Implement help display
        warn!("Help display not yet implemented");
        Ok(())
    }

    pub async fn render(&self) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone();
        if let Some(session_name) = session_name {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(&session_name) {
                if let Some(window) = session.get_active_window() {
                    // Render all panes in the active window
                    for pane in window.panes.values() {
                        self.render_pane(pane).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn render_pane(&self, pane: &Pane) -> Result<(), MultiplexerError> {
        // Read data from PTY and update renderer
        let mut buffer = [0u8; 4096];
        match self
            .tty_engine
            .read_from_pty(pane.pty_id, &mut buffer)
            .await
        {
            Ok(bytes_read) if bytes_read > 0 => {
                let data = &buffer[..bytes_read];
                let text = String::from_utf8_lossy(data);

                // Update renderer grid for this pane's region
                let mut renderer = self.renderer.write().await;
                let grid = renderer.get_grid();

                // Simple text rendering - in practice, would need more sophisticated parsing
                let mut x = pane.layout.x;
                let mut y = pane.layout.y;

                for ch in text.chars() {
                    if ch == '\n' {
                        x = pane.layout.x;
                        y += 1;
                        if y >= pane.layout.y + pane.layout.height {
                            break;
                        }
                    } else if ch != '\r' {
                        if x < pane.layout.x + pane.layout.width
                            && y < pane.layout.y + pane.layout.height
                        {
                            // Update grid cell
                            let mut grid_write = grid.write();
                            grid_write.set_cell(
                                x,
                                y,
                                crate::renderer::TerminalCell {
                                    character: ch,
                                    foreground: [1.0, 1.0, 1.0, 1.0],
                                    background: [0.0, 0.0, 0.0, 1.0],
                                    bold: false,
                                    italic: false,
                                    underline: false,
                                },
                            );
                        }
                        x += 1;
                        if x >= pane.layout.x + pane.layout.width {
                            x = pane.layout.x;
                            y += 1;
                            if y >= pane.layout.y + pane.layout.height {
                                break;
                            }
                        }
                    }
                }
            }
            Ok(_) => {} // No data available
            Err(e) => {
                warn!("Failed to read from PTY {}: {}", pane.pty_id, e);
            }
        }

        Ok(())
    }

    pub async fn resize(&self, width: u32, height: u32) -> Result<(), MultiplexerError> {
        let session_name = self.active_session.read().await.clone();
        if let Some(session_name) = session_name {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&session_name) {
                for window in session.windows.values_mut() {
                    window.layout.width = width;
                    window.layout.height = height;
                    window.recalculate_layout();

                    // Resize all panes
                    for pane in window.panes.values() {
                        self.tty_engine.resize_pty(
                            pane.pty_id,
                            pane.layout.height as u16,
                            pane.layout.width as u16,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn list_sessions(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }

    pub async fn get_active_session(&self) -> Option<String> {
        self.active_session.read().await.clone()
    }

    fn get_next_pane_id(&self) -> u64 {
        let mut id = self.next_pane_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    fn get_next_window_id(&self) -> u64 {
        let mut id = self.next_window_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    pub async fn shutdown(&self) -> Result<(), MultiplexerError> {
        info!("Shutting down multiplexer");

        // Save all sessions
        let session_names: Vec<String> = self.sessions.read().await.keys().cloned().collect();
        for name in session_names {
            if let Err(e) = self.save_session(&name).await {
                error!("Failed to save session {}: {}", name, e);
            }
        }

        // Destroy all PTYs
        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            for window in session.windows.values() {
                for pane in window.panes.values() {
                    if let Err(e) = self.tty_engine.destroy_pty(pane.pty_id).await {
                        error!("Failed to destroy PTY {}: {}", pane.pty_id, e);
                    }
                }
            }
        }

        info!("Multiplexer shutdown complete");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PaneDirection {
    Up,
    Down,
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Mock implementations for testing
    // Note: In practice, would need proper mocks for TtyEngine, GpuRenderer, etc.

    #[tokio::test]
    async fn test_session_creation() {
        // This would require mocking the dependencies
        // For now, just test the data structures
        let session = Session::new("test".to_string(), PathBuf::from("/tmp"));
        assert_eq!(session.name, "test");
        assert!(session.windows.is_empty());
    }

    #[test]
    fn test_window_creation() {
        let window = Window::new(1, "Test Window".to_string(), 80, 24);
        assert_eq!(window.id, 1);
        assert_eq!(window.name, "Test Window");
        assert!(window.panes.is_empty());
    }

    #[test]
    fn test_pane_creation() {
        let pane = Pane::new(1, 100, 0, 0, 80, 24);
        assert_eq!(pane.id, 1);
        assert_eq!(pane.pty_id, 100);
        assert_eq!(pane.layout.width, 80);
        assert_eq!(pane.layout.height, 24);
    }

    #[test]
    fn test_layout_calculations() {
        let mut window = Window::new(1, "Test".to_string(), 80, 24);
        window.layout_algorithm = LayoutAlgorithm::EvenVertical;

        let pane1 = Pane::new(1, 100, 0, 0, 40, 24);
        let pane2 = Pane::new(2, 101, 40, 0, 40, 24);

        window.add_pane(pane1);
        window.add_pane(pane2);

        window.recalculate_layout();

        assert_eq!(window.layout.children.len(), 2);
        assert_eq!(window.layout.children[0].width, 40);
        assert_eq!(window.layout.children[1].width, 40);
    }
}
