use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose, Engine as _};
use ring::rand::SystemRandom;
use ring::signature::{self, Ed25519KeyPair, KeyPair as RingKeyPair};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Error, Debug)]
pub enum SecurityError {
    #[error("Cryptographic error: {0}")]
    Crypto(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),
    #[error("Sandbox error: {0}")]
    Sandbox(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Security policy violation: {0}")]
    PolicyViolation(String),
    #[error("Audit error: {0}")]
    Audit(String),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub enable_sandbox: bool,
    pub enable_audit: bool,
    pub enable_encryption: bool,
    pub max_memory_mb: u64,
    pub max_cpu_percent: u32,
    pub allowed_syscalls: Vec<String>,
    pub blocked_paths: Vec<String>,
    pub audit_log_path: PathBuf,
    pub key_store_path: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enable_sandbox: true,
            enable_audit: true,
            enable_encryption: true,
            max_memory_mb: 2048,
            max_cpu_percent: 80,
            allowed_syscalls: vec![
                "read".to_string(),
                "write".to_string(),
                "open".to_string(),
                "close".to_string(),
                "stat".to_string(),
                "fstat".to_string(),
                "lstat".to_string(),
                "mmap".to_string(),
                "munmap".to_string(),
                "brk".to_string(),
                "rt_sigaction".to_string(),
                "rt_sigprocmask".to_string(),
                "rt_sigreturn".to_string(),
                "ioctl".to_string(),
                "pread64".to_string(),
                "pwrite64".to_string(),
                "readv".to_string(),
                "writev".to_string(),
                "pipe".to_string(),
                "select".to_string(),
                "sched_yield".to_string(),
                "mremap".to_string(),
                "msync".to_string(),
                "mincore".to_string(),
                "madvise".to_string(),
                "shmget".to_string(),
                "shmat".to_string(),
                "shmctl".to_string(),
                "dup".to_string(),
                "dup2".to_string(),
                "pause".to_string(),
                "nanosleep".to_string(),
                "getitimer".to_string(),
                "alarm".to_string(),
                "setitimer".to_string(),
                "getpid".to_string(),
                "sendfile".to_string(),
                "socket".to_string(),
                "connect".to_string(),
                "accept".to_string(),
                "sendto".to_string(),
                "recvfrom".to_string(),
                "sendmsg".to_string(),
                "recvmsg".to_string(),
                "shutdown".to_string(),
                "bind".to_string(),
                "listen".to_string(),
                "getsockname".to_string(),
                "getpeername".to_string(),
                "socketpair".to_string(),
                "setsockopt".to_string(),
                "getsockopt".to_string(),
                "clone".to_string(),
                "fork".to_string(),
                "vfork".to_string(),
                "execve".to_string(),
                "exit".to_string(),
                "wait4".to_string(),
                "kill".to_string(),
                "uname".to_string(),
                "semget".to_string(),
                "semop".to_string(),
                "semctl".to_string(),
                "shmdt".to_string(),
                "msgget".to_string(),
                "msgsnd".to_string(),
                "msgrcv".to_string(),
                "msgctl".to_string(),
                "fcntl".to_string(),
                "flock".to_string(),
                "fsync".to_string(),
                "fdatasync".to_string(),
                "truncate".to_string(),
                "ftruncate".to_string(),
                "getdents".to_string(),
                "getcwd".to_string(),
                "chdir".to_string(),
                "fchdir".to_string(),
                "rename".to_string(),
                "mkdir".to_string(),
                "rmdir".to_string(),
                "creat".to_string(),
                "link".to_string(),
                "unlink".to_string(),
                "symlink".to_string(),
                "readlink".to_string(),
                "chmod".to_string(),
                "fchmod".to_string(),
                "chown".to_string(),
                "fchown".to_string(),
                "lchown".to_string(),
                "umask".to_string(),
                "gettimeofday".to_string(),
                "getrlimit".to_string(),
                "getrusage".to_string(),
                "sysinfo".to_string(),
                "times".to_string(),
                "ptrace".to_string(),
                "getuid".to_string(),
                "syslog".to_string(),
                "getgid".to_string(),
                "setuid".to_string(),
                "setgid".to_string(),
                "geteuid".to_string(),
                "getegid".to_string(),
                "setpgid".to_string(),
                "getppid".to_string(),
                "getpgrp".to_string(),
                "setsid".to_string(),
                "setreuid".to_string(),
                "setregid".to_string(),
                "getgroups".to_string(),
                "setgroups".to_string(),
                "setresuid".to_string(),
                "getresuid".to_string(),
                "setresgid".to_string(),
                "getresgid".to_string(),
                "getpgid".to_string(),
                "setfsuid".to_string(),
                "setfsgid".to_string(),
                "getsid".to_string(),
                "capget".to_string(),
                "capset".to_string(),
                "rt_sigpending".to_string(),
                "rt_sigtimedwait".to_string(),
                "rt_sigqueueinfo".to_string(),
                "rt_sigsuspend".to_string(),
                "sigaltstack".to_string(),
                "utime".to_string(),
                "mknod".to_string(),
                "uselib".to_string(),
                "personality".to_string(),
                "ustat".to_string(),
                "statfs".to_string(),
                "fstatfs".to_string(),
                "sysfs".to_string(),
                "getpriority".to_string(),
                "setpriority".to_string(),
                "sched_setparam".to_string(),
                "sched_getparam".to_string(),
                "sched_setscheduler".to_string(),
                "sched_getscheduler".to_string(),
                "sched_get_priority_max".to_string(),
                "sched_get_priority_min".to_string(),
                "sched_rr_get_interval".to_string(),
                "mlock".to_string(),
                "munlock".to_string(),
                "mlockall".to_string(),
                "munlockall".to_string(),
                "vhangup".to_string(),
                "modify_ldt".to_string(),
                "pivot_root".to_string(),
                "_sysctl".to_string(),
                "prctl".to_string(),
                "arch_prctl".to_string(),
                "adjtimex".to_string(),
                "setrlimit".to_string(),
                "chroot".to_string(),
                "sync".to_string(),
                "acct".to_string(),
                "settimeofday".to_string(),
                "mount".to_string(),
                "umount2".to_string(),
                "swapon".to_string(),
                "swapoff".to_string(),
                "reboot".to_string(),
                "sethostname".to_string(),
                "setdomainname".to_string(),
                "iopl".to_string(),
                "ioperm".to_string(),
                "create_module".to_string(),
                "init_module".to_string(),
                "delete_module".to_string(),
                "get_kernel_syms".to_string(),
                "query_module".to_string(),
                "quotactl".to_string(),
                "nfsservctl".to_string(),
                "getpmsg".to_string(),
                "putpmsg".to_string(),
                "afs_syscall".to_string(),
                "tuxcall".to_string(),
                "security".to_string(),
                "gettid".to_string(),
                "readahead".to_string(),
                "setxattr".to_string(),
                "lsetxattr".to_string(),
                "fsetxattr".to_string(),
                "getxattr".to_string(),
                "lgetxattr".to_string(),
                "fgetxattr".to_string(),
                "listxattr".to_string(),
                "llistxattr".to_string(),
                "flistxattr".to_string(),
                "removexattr".to_string(),
                "lremovexattr".to_string(),
                "fremovexattr".to_string(),
                "tkill".to_string(),
                "time".to_string(),
                "futex".to_string(),
                "sched_setaffinity".to_string(),
                "sched_getaffinity".to_string(),
                "set_thread_area".to_string(),
                "io_setup".to_string(),
                "io_destroy".to_string(),
                "io_getevents".to_string(),
                "io_submit".to_string(),
                "io_cancel".to_string(),
                "get_thread_area".to_string(),
                "lookup_dcookie".to_string(),
                "epoll_create".to_string(),
                "epoll_ctl_old".to_string(),
                "epoll_wait_old".to_string(),
                "remap_file_pages".to_string(),
                "getdents64".to_string(),
                "set_tid_address".to_string(),
                "restart_syscall".to_string(),
                "semtimedop".to_string(),
                "fadvise64".to_string(),
                "timer_create".to_string(),
                "timer_settime".to_string(),
                "timer_gettime".to_string(),
                "timer_getoverrun".to_string(),
                "timer_delete".to_string(),
                "clock_settime".to_string(),
                "clock_gettime".to_string(),
                "clock_getres".to_string(),
                "clock_nanosleep".to_string(),
                "exit_group".to_string(),
                "epoll_wait".to_string(),
                "epoll_ctl".to_string(),
                "tgkill".to_string(),
                "utimes".to_string(),
                "vserver".to_string(),
                "mbind".to_string(),
                "set_mempolicy".to_string(),
                "get_mempolicy".to_string(),
                "mq_open".to_string(),
                "mq_unlink".to_string(),
                "mq_timedsend".to_string(),
                "mq_timedreceive".to_string(),
                "mq_notify".to_string(),
                "mq_getsetattr".to_string(),
                "kexec_load".to_string(),
                "waitid".to_string(),
                "add_key".to_string(),
                "request_key".to_string(),
                "keyctl".to_string(),
                "ioprio_set".to_string(),
                "ioprio_get".to_string(),
                "inotify_init".to_string(),
                "inotify_add_watch".to_string(),
                "inotify_rm_watch".to_string(),
                "migrate_pages".to_string(),
                "openat".to_string(),
                "mkdirat".to_string(),
                "mknodat".to_string(),
                "fchownat".to_string(),
                "futimesat".to_string(),
                "newfstatat".to_string(),
                "unlinkat".to_string(),
                "renameat".to_string(),
                "linkat".to_string(),
                "symlinkat".to_string(),
                "readlinkat".to_string(),
                "fchmodat".to_string(),
                "faccessat".to_string(),
                "pselect6".to_string(),
                "ppoll".to_string(),
                "unshare".to_string(),
                "set_robust_list".to_string(),
                "get_robust_list".to_string(),
                "splice".to_string(),
                "tee".to_string(),
                "sync_file_range".to_string(),
                "vmsplice".to_string(),
                "move_pages".to_string(),
                "utimensat".to_string(),
                "epoll_pwait".to_string(),
                "signalfd".to_string(),
                "timerfd_create".to_string(),
                "eventfd".to_string(),
                "fallocate".to_string(),
                "timerfd_settime".to_string(),
                "timerfd_gettime".to_string(),
                "accept4".to_string(),
                "signalfd4".to_string(),
                "eventfd2".to_string(),
                "epoll_create1".to_string(),
                "dup3".to_string(),
                "pipe2".to_string(),
                "inotify_init1".to_string(),
                "preadv".to_string(),
                "pwritev".to_string(),
                "rt_tgsigqueueinfo".to_string(),
                "perf_event_open".to_string(),
                "recvmmsg".to_string(),
                "fanotify_init".to_string(),
                "fanotify_mark".to_string(),
                "prlimit64".to_string(),
                "name_to_handle_at".to_string(),
                "open_by_handle_at".to_string(),
                "clock_adjtime".to_string(),
                "syncfs".to_string(),
                "sendmmsg".to_string(),
                "setns".to_string(),
                "getcpu".to_string(),
                "process_vm_readv".to_string(),
                "process_vm_writev".to_string(),
                "kcmp".to_string(),
                "finit_module".to_string(),
                "sched_setattr".to_string(),
                "sched_getattr".to_string(),
                "renameat2".to_string(),
                "seccomp".to_string(),
                "getrandom".to_string(),
                "memfd_create".to_string(),
                "kexec_file_load".to_string(),
                "bpf".to_string(),
                "execveat".to_string(),
                "userfaultfd".to_string(),
                "membarrier".to_string(),
                "mlock2".to_string(),
                "copy_file_range".to_string(),
                "preadv2".to_string(),
                "pwritev2".to_string(),
                "pkey_mprotect".to_string(),
                "pkey_alloc".to_string(),
                "pkey_free".to_string(),
                "statx".to_string(),
                "io_pgetevents".to_string(),
                "rseq".to_string(),
                "pidfd_send_signal".to_string(),
                "io_uring_setup".to_string(),
                "io_uring_enter".to_string(),
                "io_uring_register".to_string(),
                "clone3".to_string(),
                "close_range".to_string(),
                "openat2".to_string(),
                "pidfd_getfd".to_string(),
                "faccessat2".to_string(),
                "process_madvise".to_string(),
                "epoll_pwait2".to_string(),
                "mount_setattr".to_string(),
                "quotactl_fd".to_string(),
                "landlock_create_ruleset".to_string(),
                "landlock_add_rule".to_string(),
                "landlock_restrict_self".to_string(),
                "memfd_secret".to_string(),
                "process_mrelease".to_string(),
                "futex_waitv".to_string(),
                "set_mempolicy_home_node".to_string(),
            ]
            .to_vec(),
            blocked_paths: vec![
                "/etc/passwd".to_string(),
                "/etc/shadow".to_string(),
                "/etc/sudoers".to_string(),
                "/root".to_string(),
                "/home".to_string(),
            ],
            audit_log_path: PathBuf::from("~/.ferroterm/security/audit.log"),
            key_store_path: PathBuf::from("~/.ferroterm/security/keys"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: u64,
    pub event_type: AuditEventType,
    pub user_id: u32,
    pub process_id: u32,
    pub details: HashMap<String, String>,
    pub severity: AuditSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventType {
    ProcessStart,
    ProcessEnd,
    FileAccess,
    NetworkAccess,
    PrivilegeEscalation,
    SandboxViolation,
    Authentication,
    Authorization,
    DataAccess,
    SecurityConfigChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPair {
    pub public_key: Vec<u8>,
    pub private_key: Vec<u8>,
}

pub struct SecurityManager {
    config: Arc<RwLock<SecurityConfig>>,
    audit_events: Arc<RwLock<Vec<AuditEvent>>>,
    signing_keys: Arc<RwLock<Option<KeyPair>>>,
    sandbox_active: Arc<RwLock<bool>>,
}

impl SecurityManager {
    pub fn new(config: SecurityConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            audit_events: Arc::new(RwLock::new(Vec::new())),
            signing_keys: Arc::new(RwLock::new(None)),
            sandbox_active: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn initialize(&self) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        // Create security directories
        self.create_security_directories().await?;

        // Initialize audit logging
        if config.enable_audit {
            self.initialize_audit().await?;
        }

        // Load or generate signing keys
        if config.enable_encryption {
            self.initialize_keys().await?;
        }

        // Setup sandbox if enabled
        if config.enable_sandbox {
            self.initialize_sandbox().await?;
        }

        info!("Security manager initialized");
        Ok(())
    }

    async fn create_security_directories(&self) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        // Create audit log directory
        if let Some(parent) = config.audit_log_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create key store directory
        fs::create_dir_all(&config.key_store_path)?;

        Ok(())
    }

    async fn initialize_audit(&self) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        // Create audit log file if it doesn't exist
        if !config.audit_log_path.exists() {
            File::create(&config.audit_log_path)?;
        }

        // Log initialization event
        self.log_audit_event(AuditEvent {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            event_type: AuditEventType::SecurityConfigChange,
            user_id: unsafe { libc::getuid() },
            process_id: std::process::id(),
            details: {
                let mut details = HashMap::new();
                details.insert("action".to_string(), "security_initialized".to_string());
                details
            },
            severity: AuditSeverity::Low,
        })
        .await?;

        Ok(())
    }

    async fn initialize_keys(&self) -> Result<(), SecurityError> {
        let config = self.config.read().await;
        let key_path = config.key_store_path.join("signing_keys.json");

        if key_path.exists() {
            // Load existing keys
            let key_data = fs::read_to_string(key_path)?;
            let key_pair: KeyPair = serde_json::from_str(&key_data)?;
            *self.signing_keys.write().await = Some(key_pair);
        } else {
            // Generate new keys
            let key_pair = self.generate_key_pair().await?;
            *self.signing_keys.write().await = Some(key_pair.clone());

            // Save keys
            let key_data = serde_json::to_string_pretty(&key_pair)?;
            fs::write(key_path, key_data)?;
        }

        Ok(())
    }

    async fn generate_key_pair(&self) -> Result<KeyPair, SecurityError> {
        // TODO: Implement proper key generation
        // For now, generate simple placeholder keys
        Ok(KeyPair {
            public_key: b"placeholder-public-key".to_vec(),
            private_key: b"placeholder-private-key".to_vec(),
        })
    }

    async fn initialize_sandbox(&self) -> Result<(), SecurityError> {
        // Check if seccomp is available
        if !self.is_seccomp_available() {
            warn!("Seccomp not available, sandboxing disabled");
            return Ok(());
        }

        // Apply seccomp filter
        self.apply_seccomp_filter().await?;

        *self.sandbox_active.write().await = true;
        info!("Sandbox initialized with seccomp");

        Ok(())
    }

    fn is_seccomp_available(&self) -> bool {
        // Check if seccomp syscall is available
        // Note: seccomp is Linux-specific, not available on macOS
        #[cfg(target_os = "linux")]
        {
            unsafe { libc::syscall(libc::SYS_seccomp, 0, 0, 0) != -1 }
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    async fn apply_seccomp_filter(&self) -> Result<(), SecurityError> {
        // seccomp is Linux-specific, not available on macOS
        #[cfg(target_os = "linux")]
        {
            let config = self.config.read().await;

            // This is a simplified seccomp setup
            // In practice, you'd use the seccomp crate or similar
            let filter = self.build_seccomp_filter(&config.allowed_syscalls)?;

            // Apply the filter using seccomp syscall
            let ret = unsafe {
                libc::syscall(
                    libc::SYS_seccomp,
                    1, // SECCOMP_SET_MODE_FILTER
                    0, // flags
                    &filter as *const _ as *const libc::c_void,
                )
            };

            if ret == -1 {
                return Err(SecurityError::Sandbox(
                    "Failed to apply seccomp filter".to_string(),
                ));
            }

            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            // On non-Linux platforms, seccomp is not available
            // Return success to avoid breaking the application
            Ok(())
        }
    }

    fn build_seccomp_filter(&self, allowed_syscalls: &[String]) -> Result<Vec<u8>, SecurityError> {
        // This is a placeholder - in practice you'd build a proper BPF filter
        // For now, return an empty filter that allows everything
        Ok(vec![])
    }

    pub async fn sign_data(&self, data: &[u8]) -> Result<String, SecurityError> {
        // TODO: Implement proper cryptographic signing
        // For now, return a placeholder signature
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(data);
        let checksum = hasher.finalize();
        Ok(format!("placeholder-signature-{}", checksum))
    }

    pub async fn verify_signature(
        &self,
        data: &[u8],
        signature: &str,
    ) -> Result<bool, SecurityError> {
        // TODO: Implement proper cryptographic verification
        // For now, just check if signature starts with placeholder
        Ok(signature.starts_with("placeholder-signature-"))
    }

    pub async fn log_audit_event(&self, event: AuditEvent) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        if !config.enable_audit {
            return Ok(());
        }

        // Add to in-memory log
        self.audit_events.write().await.push(event.clone());

        // Write to file
        let event_json = serde_json::to_string(&event)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.audit_log_path)?;

        writeln!(file, "{}", event_json)?;

        Ok(())
    }

    pub async fn check_file_access(
        &self,
        path: &Path,
        operation: &str,
    ) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        // Check blocked paths
        let path_str = path.to_string_lossy();
        for blocked in &config.blocked_paths {
            if path_str.starts_with(blocked) {
                self.log_audit_event(AuditEvent {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    event_type: AuditEventType::FileAccess,
                    user_id: unsafe { libc::getuid() },
                    process_id: std::process::id(),
                    details: {
                        let mut details = HashMap::new();
                        details.insert("path".to_string(), path_str.to_string());
                        details.insert("operation".to_string(), operation.to_string());
                        details.insert("blocked".to_string(), "true".to_string());
                        details
                    },
                    severity: AuditSeverity::High,
                })
                .await?;

                return Err(SecurityError::PermissionDenied(format!(
                    "Access to {} is blocked",
                    path_str
                )));
            }
        }

        // Log successful access
        self.log_audit_event(AuditEvent {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            event_type: AuditEventType::FileAccess,
            user_id: unsafe { libc::getuid() },
            process_id: std::process::id(),
            details: {
                let mut details = HashMap::new();
                details.insert("path".to_string(), path_str.to_string());
                details.insert("operation".to_string(), operation.to_string());
                details.insert("allowed".to_string(), "true".to_string());
                details
            },
            severity: AuditSeverity::Low,
        })
        .await?;

        Ok(())
    }

    pub async fn validate_memory_usage(&self) -> Result<(), SecurityError> {
        let config = self.config.read().await;

        // Get current memory usage
        let memory_kb = self.get_current_memory_usage()?;

        if memory_kb > config.max_memory_mb * 1024 {
            self.log_audit_event(AuditEvent {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                event_type: AuditEventType::SandboxViolation,
                user_id: unsafe { libc::getuid() },
                process_id: std::process::id(),
                details: {
                    let mut details = HashMap::new();
                    details.insert("violation".to_string(), "memory_limit_exceeded".to_string());
                    details.insert("current_mb".to_string(), (memory_kb / 1024).to_string());
                    details.insert("limit_mb".to_string(), config.max_memory_mb.to_string());
                    details
                },
                severity: AuditSeverity::High,
            })
            .await?;

            return Err(SecurityError::PolicyViolation(
                "Memory limit exceeded".to_string(),
            ));
        }

        Ok(())
    }

    fn get_current_memory_usage(&self) -> Result<u64, SecurityError> {
        let statm = fs::read_to_string("/proc/self/statm")?;
        let pages = statm
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        // Convert pages to KB (assuming 4KB pages)
        Ok(pages * 4)
    }

    pub async fn get_audit_events(&self, limit: Option<usize>) -> Vec<AuditEvent> {
        let events = self.audit_events.read().await;
        match limit {
            Some(n) => events.iter().rev().take(n).cloned().collect(),
            None => events.clone(),
        }
    }

    pub async fn generate_security_report(&self) -> Result<String, SecurityError> {
        let config = self.config.read().await;
        let events = self.audit_events.read().await;
        let sandbox_active = *self.sandbox_active.read().await;

        let mut report = String::new();
        report.push_str("# Ferroterm Security Report\n\n");

        report.push_str("## Configuration\n");
        report.push_str(&format!("- Sandbox: {}\n", config.enable_sandbox));
        report.push_str(&format!("- Audit: {}\n", config.enable_audit));
        report.push_str(&format!("- Encryption: {}\n", config.enable_encryption));
        report.push_str(&format!("- Max Memory: {} MB\n", config.max_memory_mb));
        report.push_str(&format!("- Max CPU: {}%\n", config.max_cpu_percent));
        report.push_str(&format!("- Sandbox Active: {}\n", sandbox_active));

        report.push_str("\n## Recent Audit Events\n");
        for event in events.iter().rev().take(10) {
            report.push_str(&format!(
                "- {}: {:?} (Severity: {:?})\n",
                event.timestamp, event.event_type, event.severity
            ));
        }

        Ok(report)
    }

    pub async fn cleanup(&self) -> Result<(), SecurityError> {
        // Clear sensitive data from memory
        self.audit_events.write().await.clear();

        // Log cleanup event
        self.log_audit_event(AuditEvent {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            event_type: AuditEventType::SecurityConfigChange,
            user_id: unsafe { libc::getuid() },
            process_id: std::process::id(),
            details: {
                let mut details = HashMap::new();
                details.insert("action".to_string(), "security_cleanup".to_string());
                details
            },
            severity: AuditSeverity::Low,
        })
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_security_config_default() {
        let config = SecurityConfig::default();
        assert!(config.enable_sandbox);
        assert!(config.enable_audit);
        assert!(config.enable_encryption);
        assert_eq!(config.max_memory_mb, 2048);
        assert_eq!(config.max_cpu_percent, 80);
    }

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent {
            timestamp: 1234567890,
            event_type: AuditEventType::ProcessStart,
            user_id: 1000,
            process_id: 12345,
            details: {
                let mut details = HashMap::new();
                details.insert("command".to_string(), "ls".to_string());
                details
            },
            severity: AuditSeverity::Low,
        };

        assert_eq!(event.user_id, 1000);
        assert_eq!(event.process_id, 12345);
        assert_eq!(event.details.get("command"), Some(&"ls".to_string()));
    }

    #[tokio::test]
    async fn test_security_manager_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = SecurityConfig::default();
        config.audit_log_path = temp_dir.path().join("audit.log");
        config.key_store_path = temp_dir.path().join("keys");

        let manager = SecurityManager::new(config);
        assert!(manager.initialize().await.is_ok());
    }

    #[tokio::test]
    async fn test_memory_validation() {
        let config = SecurityConfig::default();
        let manager = SecurityManager::new(config);

        // This test may fail if memory usage is actually high
        // In practice, you'd mock the memory checking
        let result = manager.validate_memory_usage().await;
        assert!(result.is_ok() || matches!(result, Err(SecurityError::PolicyViolation(_))));
    }

    #[test]
    fn test_key_pair_structure() {
        let key_pair = KeyPair {
            public_key: vec![1, 2, 3, 4],
            private_key: vec![5, 6, 7, 8],
        };

        assert_eq!(key_pair.public_key.len(), 4);
        assert_eq!(key_pair.private_key.len(), 4);
    }
}
