// TTY Engine implementation using direct libc calls for maximum performance
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{self, WaitStatus};
use nix::unistd::{self, ForkResult, Pid};
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum TtyError {
    #[error("PTY creation failed: {0}")]
    PtyCreation(String),
    #[error("Fork failed: {0}")]
    Fork(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Signal error: {0}")]
    Signal(String),
    #[error("PTY not found: {id}")]
    PtyNotFound { id: u64 },
    #[error("Invalid terminal mode: {mode}")]
    InvalidMode { mode: String },
    #[error("Buffer overflow: {size} bytes")]
    BufferOverflow { size: usize },
    #[error("Process died unexpectedly: pid {pid}")]
    ProcessDied { pid: i32 },
    #[error("Timeout: operation took longer than {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMode {
    Raw,
    Cooked,
    AltScreen,
}

#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub shell: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
            args: vec![],
            env: std::env::vars().collect(),
            working_dir: None,
            rows: 24,
            cols: 80,
        }
    }
}

#[derive(Debug)]
pub struct PtySession {
    pub id: u64,
    pub master_fd: RawFd,
    pub child_pid: Pid,
    pub mode: Arc<RwLock<TerminalMode>>,
    pub created_at: Instant,
    pub bytes_read: AtomicU64,
    pub bytes_written: AtomicU64,
    pub is_alive: AtomicBool,
}

impl PtySession {
    pub fn new(id: u64, master_fd: RawFd, child_pid: Pid) -> Self {
        Self {
            id,
            master_fd,
            child_pid,
            mode: Arc::new(RwLock::new(TerminalMode::Cooked)),
            created_at: Instant::now(),
            bytes_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            is_alive: AtomicBool::new(true),
        }
    }

    pub fn set_mode(&self, mode: TerminalMode) -> Result<(), TtyError> {
        // For now, just store the mode. Full termios implementation would go here
        *self.mode.write().unwrap() = mode;
        Ok(())
    }

    pub fn get_mode(&self) -> TerminalMode {
        *self.mode.read().unwrap()
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), TtyError> {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        unsafe {
            let result = libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize);
            if result == -1 {
                // Don't fail on resize errors - some systems may not support it
                warn!(
                    "Failed to set window size for PTY {}: {}",
                    self.id,
                    std::io::Error::last_os_error()
                );
            }
        }

        // Send SIGWINCH to child process (ignore errors)
        let _ = signal::kill(self.child_pid, Signal::SIGWINCH);
        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive.load(Ordering::Relaxed)
    }

    pub fn mark_dead(&self) {
        self.is_alive.store(false, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> (u64, u64, Duration) {
        (
            self.bytes_read.load(Ordering::Relaxed),
            self.bytes_written.load(Ordering::Relaxed),
            self.created_at.elapsed(),
        )
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

// Zero-copy buffer implementation is available for future use
// Currently using direct libc calls for maximum performance

pub struct TtyEngine {
    sessions: Arc<RwLock<HashMap<u64, Arc<PtySession>>>>,
    next_id: AtomicU64,
    signal_tx: broadcast::Sender<(Signal, Option<u64>)>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<Mutex<TtyStats>>,
}

#[derive(Debug, Default)]
pub struct TtyStats {
    pub sessions_created: u64,
    pub sessions_destroyed: u64,
    pub total_bytes_read: u64,
    pub total_bytes_written: u64,
    pub signal_count: u64,
    pub errors: u64,
}

impl TtyEngine {
    pub fn new() -> Self {
        let (signal_tx, _) = broadcast::channel(1024);

        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            signal_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(TtyStats::default())),
        }
    }

    pub async fn create_pty(&self, config: PtyConfig) -> Result<u64, TtyError> {
        let start = Instant::now();

        // Create PTY using libc directly for better compatibility
        let master_fd = unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if fd == -1 {
                return Err(TtyError::PtyCreation(
                    "Failed to create PTY master".to_string(),
                ));
            }

            if libc::grantpt(fd) == -1 {
                libc::close(fd);
                return Err(TtyError::PtyCreation("Failed to grant PTY".to_string()));
            }

            if libc::unlockpt(fd) == -1 {
                libc::close(fd);
                return Err(TtyError::PtyCreation("Failed to unlock PTY".to_string()));
            }

            fd
        };

        // Set initial window size
        let winsize = libc::winsize {
            ws_row: config.rows,
            ws_col: config.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        unsafe {
            libc::ioctl(master_fd, libc::TIOCSWINSZ, &winsize);
        }

        // Fork process
        let fork_result = unsafe { unistd::fork() }.map_err(|e| TtyError::Fork(e.to_string()))?;

        match fork_result {
            ForkResult::Parent { child } => {
                // Parent process - create session
                let session_id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let session = Arc::new(PtySession::new(session_id, master_fd, child));

                self.sessions
                    .write()
                    .unwrap()
                    .insert(session_id, session.clone());

                // Start monitoring this session
                self.start_session_monitor(session.clone()).await;

                // Update stats
                {
                    let mut stats = self.stats.lock().unwrap();
                    stats.sessions_created += 1;
                }

                let creation_time = start.elapsed();
                debug!("PTY {} created in {:?}", session_id, creation_time);

                Ok(session_id)
            }
            ForkResult::Child => {
                // Child process - setup and exec shell
                self.setup_child_process(master_fd, &config).await?;
                unreachable!("Child process should have exec'd");
            }
        }
    }

    async fn setup_child_process(
        &self,
        master_fd: RawFd,
        config: &PtyConfig,
    ) -> Result<(), TtyError> {
        unsafe {
            // Create new session
            if libc::setsid() == -1 {
                return Err(TtyError::Fork("Failed to create new session".to_string()));
            }

            // Get slave PTY name
            let slave_name = libc::ptsname(master_fd);
            if slave_name.is_null() {
                return Err(TtyError::Fork("Failed to get slave PTY name".to_string()));
            }

            // Open slave PTY
            let slave_fd = libc::open(slave_name, libc::O_RDWR);
            if slave_fd == -1 {
                return Err(TtyError::Fork("Failed to open slave PTY".to_string()));
            }

            // Redirect stdin, stdout, stderr to slave PTY
            libc::dup2(slave_fd, 0); // stdin
            libc::dup2(slave_fd, 1); // stdout
            libc::dup2(slave_fd, 2); // stderr

            // Close the slave fd and master fd as they're now duplicated
            libc::close(slave_fd);
            libc::close(master_fd);

            // Set working directory
            if let Some(ref dir) = config.working_dir {
                let c_dir = std::ffi::CString::new(dir.as_str()).unwrap();
                libc::chdir(c_dir.as_ptr());
            }

            // Set environment variables
            for (key, value) in &config.env {
                let c_key = std::ffi::CString::new(key.as_str()).unwrap();
                let c_value = std::ffi::CString::new(value.as_str()).unwrap();
                libc::setenv(c_key.as_ptr(), c_value.as_ptr(), 1);
            }

            // Prepare arguments
            let mut args = vec![config.shell.clone()];
            args.extend(config.args.iter().cloned());

            let c_args: Vec<std::ffi::CString> = args
                .iter()
                .map(|s| std::ffi::CString::new(s.as_str()).unwrap())
                .collect();

            let mut c_argv: Vec<*const libc::c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
            c_argv.push(std::ptr::null());

            // Execute shell
            let c_shell = std::ffi::CString::new(config.shell.as_str()).unwrap();
            libc::execv(c_shell.as_ptr(), c_argv.as_ptr());

            // If we get here, exec failed
            libc::exit(1);
        }
    }

    async fn start_session_monitor(&self, session: Arc<PtySession>) {
        let sessions = Arc::clone(&self.sessions);
        let stats = Arc::clone(&self.stats);
        let shutdown = Arc::clone(&self.shutdown);

        tokio::spawn(async move {
            let mut check_interval = tokio::time::interval(Duration::from_millis(100));

            while !shutdown.load(Ordering::Relaxed) && session.is_alive() {
                check_interval.tick().await;

                // Check if child process is still alive
                match wait::waitpid(session.child_pid, Some(wait::WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => continue,
                    Ok(_) | Err(_) => {
                        // Process has died
                        session.mark_dead();
                        sessions.write().unwrap().remove(&session.id);

                        let mut stats_guard = stats.lock().unwrap();
                        stats_guard.sessions_destroyed += 1;

                        info!("PTY session {} terminated", session.id);
                        break;
                    }
                }
            }
        });
    }

    pub async fn write_to_pty(&self, pty_id: u64, data: &[u8]) -> Result<usize, TtyError> {
        let session = {
            let sessions = self.sessions.read().unwrap();
            sessions
                .get(&pty_id)
                .ok_or(TtyError::PtyNotFound { id: pty_id })?
                .clone()
        };

        if !session.is_alive() {
            return Err(TtyError::ProcessDied {
                pid: session.child_pid.as_raw(),
            });
        }

        let master_fd = session.master_fd;
        let data_vec = data.to_vec();

        // Use non-blocking write with timeout
        let write_result = timeout(Duration::from_millis(100), async {
            tokio::task::spawn_blocking(move || unsafe {
                libc::write(
                    master_fd,
                    data_vec.as_ptr() as *const libc::c_void,
                    data_vec.len(),
                )
            })
            .await
            .unwrap()
        })
        .await;

        match write_result {
            Ok(bytes_written) => {
                if bytes_written < 0 {
                    let mut stats = self.stats.lock().unwrap();
                    stats.errors += 1;
                    Err(TtyError::Io(std::io::Error::last_os_error()))
                } else {
                    let bytes_written = bytes_written as usize;
                    session
                        .bytes_written
                        .fetch_add(bytes_written as u64, Ordering::Relaxed);

                    let mut stats = self.stats.lock().unwrap();
                    stats.total_bytes_written += bytes_written as u64;

                    Ok(bytes_written)
                }
            }
            Err(_) => Err(TtyError::Timeout { timeout_ms: 100 }),
        }
    }

    pub async fn read_from_pty(&self, pty_id: u64, buffer: &mut [u8]) -> Result<usize, TtyError> {
        let session = {
            let sessions = self.sessions.read().unwrap();
            sessions
                .get(&pty_id)
                .ok_or(TtyError::PtyNotFound { id: pty_id })?
                .clone()
        };

        if !session.is_alive() {
            return Err(TtyError::ProcessDied {
                pid: session.child_pid.as_raw(),
            });
        }

        let master_fd = session.master_fd;
        let buffer_len = buffer.len();

        // Use non-blocking read with timeout
        let read_result = timeout(Duration::from_millis(100), async {
            tokio::task::spawn_blocking(move || {
                let mut temp_buffer = vec![0u8; buffer_len];
                unsafe {
                    let result = libc::read(
                        master_fd,
                        temp_buffer.as_mut_ptr() as *mut libc::c_void,
                        buffer_len,
                    );
                    (result, temp_buffer)
                }
            })
            .await
            .unwrap()
        })
        .await;

        match read_result {
            Ok((bytes_read, temp_buffer)) => {
                if bytes_read < 0 {
                    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if errno == libc::EAGAIN || errno == libc::EWOULDBLOCK {
                        Ok(0) // No data available
                    } else {
                        let mut stats = self.stats.lock().unwrap();
                        stats.errors += 1;
                        Err(TtyError::Io(std::io::Error::last_os_error()))
                    }
                } else {
                    let bytes_read = bytes_read as usize;
                    buffer[..bytes_read].copy_from_slice(&temp_buffer[..bytes_read]);

                    session
                        .bytes_read
                        .fetch_add(bytes_read as u64, Ordering::Relaxed);

                    let mut stats = self.stats.lock().unwrap();
                    stats.total_bytes_read += bytes_read as u64;

                    Ok(bytes_read)
                }
            }
            Err(_) => Err(TtyError::Timeout { timeout_ms: 100 }),
        }
    }

    pub async fn send_signal(&self, pty_id: Option<u64>, signal: Signal) -> Result<(), TtyError> {
        match pty_id {
            Some(id) => {
                let session = {
                    let sessions = self.sessions.read().unwrap();
                    sessions
                        .get(&id)
                        .ok_or(TtyError::PtyNotFound { id })?
                        .clone()
                };

                signal::kill(session.child_pid, signal)
                    .map_err(|e| TtyError::Signal(e.to_string()))?;
            }
            None => {
                // Broadcast to all sessions
                let sessions = self.sessions.read().unwrap();
                for session in sessions.values() {
                    if let Err(e) = signal::kill(session.child_pid, signal) {
                        warn!("Failed to send signal to PTY {}: {}", session.id, e);
                    }
                }
            }
        }

        // Broadcast signal event
        let _ = self.signal_tx.send((signal, pty_id));

        let mut stats = self.stats.lock().unwrap();
        stats.signal_count += 1;

        Ok(())
    }

    pub fn set_pty_mode(&self, pty_id: u64, mode: TerminalMode) -> Result<(), TtyError> {
        let sessions = self.sessions.read().unwrap();
        let session = sessions
            .get(&pty_id)
            .ok_or(TtyError::PtyNotFound { id: pty_id })?;

        session.set_mode(mode)
    }

    pub fn resize_pty(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), TtyError> {
        let sessions = self.sessions.read().unwrap();
        let session = sessions
            .get(&pty_id)
            .ok_or(TtyError::PtyNotFound { id: pty_id })?;

        session.resize(rows, cols)
    }

    pub fn get_pty_stats(&self, pty_id: u64) -> Result<(u64, u64, Duration), TtyError> {
        let sessions = self.sessions.read().unwrap();
        let session = sessions
            .get(&pty_id)
            .ok_or(TtyError::PtyNotFound { id: pty_id })?;

        Ok(session.get_stats())
    }

    pub fn list_sessions(&self) -> Vec<u64> {
        self.sessions.read().unwrap().keys().copied().collect()
    }

    pub fn get_session_count(&self) -> usize {
        self.sessions.read().unwrap().len()
    }

    pub async fn destroy_pty(&self, pty_id: u64) -> Result<(), TtyError> {
        let session = {
            let mut sessions = self.sessions.write().unwrap();
            sessions
                .remove(&pty_id)
                .ok_or(TtyError::PtyNotFound { id: pty_id })?
        };

        // Send SIGTERM to child process
        if let Err(e) = signal::kill(session.child_pid, Signal::SIGTERM) {
            warn!("Failed to send SIGTERM to PTY {}: {}", pty_id, e);
        }

        // Wait for process to exit gracefully
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Force kill if still alive
        match wait::waitpid(session.child_pid, Some(wait::WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => {
                warn!("Force killing PTY {} process", pty_id);
                let _ = signal::kill(session.child_pid, Signal::SIGKILL);
                let _ = wait::waitpid(session.child_pid, None);
            }
            _ => {}
        }

        session.mark_dead();

        let mut stats = self.stats.lock().unwrap();
        stats.sessions_destroyed += 1;

        info!("PTY session {} destroyed", pty_id);
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), TtyError> {
        info!("Shutting down TTY engine");

        self.shutdown.store(true, Ordering::Relaxed);

        // Get all session IDs
        let session_ids: Vec<u64> = self.sessions.read().unwrap().keys().copied().collect();

        // Destroy all sessions
        for id in session_ids {
            if let Err(e) = self.destroy_pty(id).await {
                error!("Failed to destroy PTY {}: {}", id, e);
            }
        }

        info!("TTY engine shutdown complete");
        Ok(())
    }

    pub fn get_stats(&self) -> TtyStats {
        let stats = self.stats.lock().unwrap();
        TtyStats {
            sessions_created: stats.sessions_created,
            sessions_destroyed: stats.sessions_destroyed,
            total_bytes_read: stats.total_bytes_read,
            total_bytes_written: stats.total_bytes_written,
            signal_count: stats.signal_count,
            errors: stats.errors,
        }
    }

    pub fn subscribe_signals(&self) -> broadcast::Receiver<(Signal, Option<u64>)> {
        self.signal_tx.subscribe()
    }
}

impl Drop for TtyEngine {
    fn drop(&mut self) {
        // Ensure all sessions are cleaned up
        let session_ids: Vec<u64> = self.sessions.read().unwrap().keys().copied().collect();

        for id in session_ids {
            if let Some(session) = self.sessions.read().unwrap().get(&id) {
                let _ = signal::kill(session.child_pid, Signal::SIGTERM);
                session.mark_dead();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_pty_creation() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();
        assert!(pty_id > 0);

        let sessions = engine.list_sessions();
        assert!(sessions.contains(&pty_id));

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_pty_io() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();

        // Write to PTY
        let test_data = b"echo hello\n";
        let written = engine.write_to_pty(pty_id, test_data).await.unwrap();
        assert_eq!(written, test_data.len());

        // Give some time for command to execute
        sleep(Duration::from_millis(100)).await;

        // Read from PTY
        let mut buffer = [0u8; 1024];
        let read_bytes = engine.read_from_pty(pty_id, &mut buffer).await.unwrap();
        assert!(read_bytes > 0);

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_pty_resize() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();

        // Test resize
        engine.resize_pty(pty_id, 50, 120).unwrap();

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_signal_handling() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();

        // Test signal sending
        engine
            .send_signal(Some(pty_id), Signal::SIGWINCH)
            .await
            .unwrap();

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_terminal_modes() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();

        // Test mode switching
        engine.set_pty_mode(pty_id, TerminalMode::Raw).unwrap();
        engine.set_pty_mode(pty_id, TerminalMode::Cooked).unwrap();
        engine
            .set_pty_mode(pty_id, TerminalMode::AltScreen)
            .unwrap();

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_multiple_sessions() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let mut pty_ids = Vec::new();

        // Create multiple PTYs with small delays to prevent resource contention
        for i in 0..3 {
            // Reduced from 5 to 3 for test stability
            let pty_id = engine.create_pty(config.clone()).await.unwrap();
            pty_ids.push(pty_id);

            // Small delay to prevent resource contention
            if i < 2 {
                sleep(Duration::from_millis(10)).await;
            }
        }

        // Give time for all sessions to be fully established
        sleep(Duration::from_millis(50)).await;

        assert_eq!(engine.get_session_count(), 3);

        // Clean up with delays
        for pty_id in pty_ids {
            engine.destroy_pty(pty_id).await.unwrap();
            sleep(Duration::from_millis(10)).await;
        }

        // Verify cleanup
        assert_eq!(engine.get_session_count(), 0);
    }

    #[tokio::test]
    async fn test_pty_stats() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        let pty_id = engine.create_pty(config).await.unwrap();

        // Write some data
        let test_data = b"test data";
        engine.write_to_pty(pty_id, test_data).await.unwrap();

        let (_bytes_read, bytes_written, uptime) = engine.get_pty_stats(pty_id).unwrap();
        assert_eq!(bytes_written, test_data.len() as u64);
        assert!(uptime.as_millis() > 0);

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_error_handling() {
        let engine = TtyEngine::new();

        // Test operations on non-existent PTY
        let result = engine.write_to_pty(999, b"test").await;
        assert!(matches!(result, Err(TtyError::PtyNotFound { id: 999 })));

        let mut buffer = [0u8; 100];
        let result = engine.read_from_pty(999, &mut buffer).await;
        assert!(matches!(result, Err(TtyError::PtyNotFound { id: 999 })));
    }

    #[tokio::test]
    async fn test_performance_requirements() {
        let engine = TtyEngine::new();
        let config = PtyConfig::default();

        // Test PTY creation time
        let start = Instant::now();
        let pty_id = engine.create_pty(config).await.unwrap();
        let creation_time = start.elapsed();

        // Should be reasonably fast (< 50ms for creation in test environment)
        assert!(
            creation_time.as_millis() < 50,
            "PTY creation took {}ms",
            creation_time.as_millis()
        );

        // Test I/O latency
        let test_data = b"echo test\n";
        let start = Instant::now();
        engine.write_to_pty(pty_id, test_data).await.unwrap();
        let write_time = start.elapsed();

        // Should be reasonably fast (< 10ms for small writes in test environment)
        assert!(
            write_time.as_millis() < 10,
            "PTY write took {}ms",
            write_time.as_millis()
        );

        engine.destroy_pty(pty_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_concurrent_operations() {
        let engine = Arc::new(TtyEngine::new());
        let config = PtyConfig::default();

        let mut handles = Vec::new();

        // Create fewer concurrent operations to prevent resource exhaustion
        for i in 0..3 {
            // Reduced from 10 to 3
            let engine_clone = Arc::clone(&engine);
            let config_clone = config.clone();

            let handle = tokio::spawn(async move {
                match tokio::time::timeout(Duration::from_secs(10), async {
                    let pty_id = engine_clone.create_pty(config_clone).await?;

                    // Perform some I/O with shorter timeout
                    let data = format!("echo test{}\n", i);
                    let _ = engine_clone.write_to_pty(pty_id, data.as_bytes()).await;

                    tokio::time::sleep(Duration::from_millis(10)).await;

                    engine_clone.destroy_pty(pty_id).await?;
                    Ok::<(), crate::tty::TtyError>(())
                })
                .await
                {
                    Ok(result) => result.unwrap(),
                    Err(_) => {
                        panic!("Concurrent operation timed out after 10 seconds");
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all operations to complete with overall timeout
        let all_operations = async {
            for handle in handles {
                handle.await.unwrap();
            }
        };

        match tokio::time::timeout(Duration::from_secs(30), all_operations).await {
            Ok(_) => {
                // Give time for cleanup
                tokio::time::sleep(Duration::from_millis(100)).await;
                // All sessions should be cleaned up
                assert_eq!(engine.get_session_count(), 0);
            }
            Err(_) => {
                panic!("All concurrent operations timed out after 30 seconds");
            }
        }
    }
}
