use memmap2::{MmapMut, MmapOptions};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::mem;
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, info, warn};

#[derive(Error, Debug)]
pub enum SharedMemoryError {
    #[error("Memory allocation failed: {0}")]
    Allocation(String),
    #[error("Memory mapping failed: {0}")]
    Mapping(String),
    #[error("Synchronization error: {0}")]
    Sync(String),
    #[error("Buffer overflow: {0}")]
    Overflow(String),
    #[error("Invalid access: {0}")]
    InvalidAccess(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u32, actual: u32 },
    #[error("Buffer not found: {id}")]
    BufferNotFound { id: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BufferHeader {
    pub magic: u32,
    pub version: u32,
    pub total_size: u64,
    pub command_ring_offset: u64,
    pub command_ring_size: u64,
    pub file_ring_offset: u64,
    pub file_ring_size: u64,
    pub scratch_offset: u64,
    pub scratch_size: u64,
    pub checksum: u32,
}

impl BufferHeader {
    pub const MAGIC: u32 = 0x50414348; // "PACH" in ASCII
    pub const VERSION: u32 = 1;

    pub fn new(
        total_size: u64,
        cmd_ring_size: u64,
        file_ring_size: u64,
        scratch_size: u64,
    ) -> Self {
        let mut header = Self {
            magic: Self::MAGIC,
            version: Self::VERSION,
            total_size,
            command_ring_offset: mem::size_of::<BufferHeader>() as u64,
            command_ring_size: cmd_ring_size,
            file_ring_offset: mem::size_of::<BufferHeader>() as u64 + cmd_ring_size,
            file_ring_size,
            scratch_offset: mem::size_of::<BufferHeader>() as u64 + cmd_ring_size + file_ring_size,
            scratch_size,
            checksum: 0,
        };

        // Calculate checksum
        header.checksum = header.calculate_checksum();
        header
    }

    pub fn calculate_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&self.magic.to_le_bytes());
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&self.total_size.to_le_bytes());
        hasher.update(&self.command_ring_offset.to_le_bytes());
        hasher.update(&self.command_ring_size.to_le_bytes());
        hasher.update(&self.file_ring_offset.to_le_bytes());
        hasher.update(&self.file_ring_size.to_le_bytes());
        hasher.update(&self.scratch_offset.to_le_bytes());
        hasher.update(&self.scratch_size.to_le_bytes());
        hasher.finalize()
    }

    pub fn validate(&self) -> Result<(), SharedMemoryError> {
        if self.magic != Self::MAGIC {
            return Err(SharedMemoryError::InvalidAccess(
                "Invalid magic number".to_string(),
            ));
        }
        if self.version != Self::VERSION {
            return Err(SharedMemoryError::InvalidAccess(format!(
                "Version mismatch: expected {}, got {}",
                Self::VERSION,
                self.version
            )));
        }
        if self.checksum != self.calculate_checksum() {
            return Err(SharedMemoryError::ChecksumMismatch {
                expected: self.calculate_checksum(),
                actual: self.checksum,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct RingBufferHeader {
    pub write_pos: AtomicU64,
    pub read_pos: AtomicU64,
    pub capacity: u64,
    pub entry_count: AtomicU64,
    pub checksum: AtomicU32,
}

impl RingBufferHeader {
    pub fn new(capacity: u64) -> Self {
        Self {
            write_pos: AtomicU64::new(0),
            read_pos: AtomicU64::new(0),
            capacity,
            entry_count: AtomicU64::new(0),
            checksum: AtomicU32::new(0),
        }
    }

    pub fn update_checksum(&self, data: &[u8]) {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(data);
        self.checksum.store(hasher.finalize(), Ordering::Release);
    }

    pub fn validate_checksum(&self, data: &[u8]) -> bool {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(data);
        let expected = hasher.finalize();
        self.checksum.load(Ordering::Acquire) == expected
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEntry {
    pub timestamp: u64,
    pub command: String,
    pub working_dir: String,
    pub exit_code: Option<i32>,
    pub output_length: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSliceEntry {
    pub timestamp: u64,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
}

pub struct SharedMemoryBuffer {
    pub id: String,
    file: File,
    mapping: MmapMut,
    header: *mut BufferHeader,
    command_ring: *mut RingBufferHeader,
    file_ring: *mut RingBufferHeader,
    scratch_area: *mut u8,
    is_readonly: bool,
}

impl SharedMemoryBuffer {
    pub fn new(
        id: String,
        cmd_ring_entries: usize,
        file_ring_entries: usize,
        scratch_size: usize,
    ) -> Result<Self, SharedMemoryError> {
        let cmd_ring_size = (mem::size_of::<RingBufferHeader>()
            + cmd_ring_entries * mem::size_of::<CommandEntry>()) as u64;
        let file_ring_size = (mem::size_of::<RingBufferHeader>()
            + file_ring_entries * mem::size_of::<FileSliceEntry>())
            as u64;
        let scratch_size = scratch_size as u64;

        let total_size =
            mem::size_of::<BufferHeader>() as u64 + cmd_ring_size + file_ring_size + scratch_size;

        // Align to 4KB page size
        let aligned_size = ((total_size + 4095) / 4096) * 4096;

        // Try to create shared memory file
        let file = Self::create_shared_memory_file(&id, aligned_size)?;

        // Map the memory
        let mut mapping = unsafe { MmapOptions::new().map_mut(&file)? };

        // Initialize header
        let header_ptr = mapping.as_mut_ptr() as *mut BufferHeader;
        let header = BufferHeader::new(aligned_size, cmd_ring_size, file_ring_size, scratch_size);
        unsafe {
            *header_ptr = header;
        }

        // Initialize ring buffers
        let cmd_ring_offset = header.command_ring_offset as usize;
        let file_ring_offset = header.file_ring_offset as usize;
        let scratch_offset = header.scratch_offset as usize;

        let command_ring =
            unsafe { mapping.as_mut_ptr().add(cmd_ring_offset) as *mut RingBufferHeader };
        let file_ring =
            unsafe { mapping.as_mut_ptr().add(file_ring_offset) as *mut RingBufferHeader };
        let scratch_area = unsafe { mapping.as_mut_ptr().add(scratch_offset) as *mut u8 };

        unsafe {
            *command_ring = RingBufferHeader::new(cmd_ring_size);
            *file_ring = RingBufferHeader::new(file_ring_size);
        }

        info!(
            "Created shared memory buffer: {} ({} bytes)",
            id, aligned_size
        );

        Ok(Self {
            id,
            file,
            mapping,
            header: header_ptr,
            command_ring,
            file_ring,
            scratch_area,
            is_readonly: false,
        })
    }

    pub fn open_readonly(id: &str) -> Result<Self, SharedMemoryError> {
        let file = Self::open_shared_memory_file(id)?;
        let mapping = unsafe { MmapOptions::new().map_mut(&file)? };

        // Validate header
        let header_ptr = mapping.as_ptr() as *const BufferHeader;
        let header = unsafe { &*header_ptr };
        header.validate()?;

        let cmd_ring_offset = header.command_ring_offset as usize;
        let file_ring_offset = header.file_ring_offset as usize;
        let scratch_offset = header.scratch_offset as usize;

        let command_ring =
            unsafe { mapping.as_ptr().add(cmd_ring_offset) as *mut RingBufferHeader };
        let file_ring = unsafe { mapping.as_ptr().add(file_ring_offset) as *mut RingBufferHeader };
        let scratch_area = unsafe { mapping.as_ptr().add(scratch_offset) as *mut u8 };

        info!("Opened shared memory buffer (readonly): {}", id);

        Ok(Self {
            id: id.to_string(),
            file,
            mapping,
            header: header_ptr as *mut BufferHeader,
            command_ring,
            file_ring,
            scratch_area,
            is_readonly: true,
        })
    }

    fn create_shared_memory_file(id: &str, size: u64) -> Result<File, SharedMemoryError> {
        // Try /dev/shm first
        let shm_path = PathBuf::from("/dev/shm").join(format!("ferroterm-{}", id));

        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&shm_path)
        {
            Ok(file) => {
                file.set_len(size)?;
                return Ok(file);
            }
            Err(_) => {
                // Fallback to memfd_create
                Self::create_memfd(id, size)
            }
        }
    }

    fn create_memfd(id: &str, size: u64) -> Result<File, SharedMemoryError> {
        #[cfg(target_os = "linux")]
        {
            // Use memfd_create syscall on Linux
            let name = std::ffi::CString::new(format!("ferroterm-{}", id)).unwrap();

            let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };

            if fd == -1 {
                return Err(SharedMemoryError::Allocation(
                    "memfd_create failed".to_string(),
                ));
            }

            let file = unsafe { File::from_raw_fd(fd) };
            file.set_len(size)?;
            Ok(file)
        }
        #[cfg(target_os = "macos")]
        {
            // On macOS, use temporary files as memfd alternative
            use std::env;
            use tempfile::NamedTempFile;

            let mut temp_file = NamedTempFile::new()
                .map_err(|e| SharedMemoryError::Allocation(format!("Failed to create temp file: {}", e)))?;

            temp_file.as_file().set_len(size)
                .map_err(|e| SharedMemoryError::Allocation(format!("Failed to set file size: {}", e)))?;

            // Keep the file around by not calling close() immediately
            let file = temp_file.into_file();
            Ok(file)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            // Fallback for other platforms
            Err(SharedMemoryError::Allocation(
                "Shared memory not supported on this platform".to_string(),
            ))
        }
    }

    fn open_shared_memory_file(id: &str) -> Result<File, SharedMemoryError> {
        #[cfg(target_os = "linux")]
        {
            // Try /dev/shm first
            let shm_path = PathBuf::from("/dev/shm").join(format!("ferroterm-{}", id));

            match OpenOptions::new().read(true).write(true).open(&shm_path) {
                Ok(file) => Ok(file),
                Err(_) => {
                    // For memfd, we can't reopen by name, so this is an error
                    Err(SharedMemoryError::BufferNotFound { id: id.to_string() })
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            // On macOS, we can't reopen temporary files by name
            // This is expected behavior since we use anonymous temp files
            Err(SharedMemoryError::BufferNotFound { id: id.to_string() })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(SharedMemoryError::BufferNotFound { id: id.to_string() })
        }
    }

    pub fn write_command(&self, command: CommandEntry) -> Result<(), SharedMemoryError> {
        if self.is_readonly {
            return Err(SharedMemoryError::InvalidAccess(
                "Buffer is readonly".to_string(),
            ));
        }

        let cmd_size = mem::size_of::<CommandEntry>();
        let ring_header = unsafe { &*self.command_ring };

        let write_pos = ring_header.write_pos.load(Ordering::Acquire);
        let capacity = ring_header.capacity;

        // Check if we have space
        let next_pos = (write_pos + cmd_size as u64) % capacity;
        if next_pos == ring_header.read_pos.load(Ordering::Acquire)
            && ring_header.entry_count.load(Ordering::Acquire) > 0
        {
            // Buffer full, advance read position (overwrite oldest)
            let read_pos = ring_header.read_pos.load(Ordering::Acquire);
            let new_read_pos = (read_pos + cmd_size as u64) % capacity;
            ring_header.read_pos.store(new_read_pos, Ordering::Release);
            ring_header.entry_count.fetch_sub(1, Ordering::Release);
        }

        // Write the command
        let cmd_bytes = unsafe {
            std::slice::from_raw_parts(&command as *const CommandEntry as *const u8, cmd_size)
        };

        let ring_start = unsafe { self.command_ring.add(1) as *mut u8 };
        let entry_ptr = unsafe { ring_start.add(write_pos as usize) };

        unsafe {
            std::ptr::copy_nonoverlapping(cmd_bytes.as_ptr(), entry_ptr, cmd_size);
        }

        // Update checksum
        ring_header.update_checksum(cmd_bytes);

        // Update positions
        ring_header.write_pos.store(next_pos, Ordering::Release);
        ring_header.entry_count.fetch_add(1, Ordering::Release);

        Ok(())
    }

    pub fn read_commands(
        &self,
        max_entries: usize,
    ) -> Result<Vec<CommandEntry>, SharedMemoryError> {
        let ring_header = unsafe { &*self.command_ring };
        let cmd_size = mem::size_of::<CommandEntry>();
        let mut commands = Vec::new();

        let mut read_pos = ring_header.read_pos.load(Ordering::Acquire);
        let write_pos = ring_header.write_pos.load(Ordering::Acquire);
        let capacity = ring_header.capacity;

        while commands.len() < max_entries && read_pos != write_pos {
            let entry_ptr = unsafe {
                let ring_start = self.command_ring.add(1) as *const u8;
                ring_start.add(read_pos as usize) as *const CommandEntry
            };

            let command = unsafe { &*entry_ptr }.clone();

            // Validate checksum
            let cmd_bytes = unsafe {
                std::slice::from_raw_parts(&command as *const CommandEntry as *const u8, cmd_size)
            };

            if !ring_header.validate_checksum(cmd_bytes) {
                warn!("Checksum validation failed for command entry");
                break;
            }

            commands.push(command);

            read_pos = (read_pos + cmd_size as u64) % capacity;
        }

        Ok(commands)
    }

    pub fn write_file_slice(&self, file_slice: FileSliceEntry) -> Result<(), SharedMemoryError> {
        if self.is_readonly {
            return Err(SharedMemoryError::InvalidAccess(
                "Buffer is readonly".to_string(),
            ));
        }

        let slice_size = mem::size_of::<FileSliceEntry>();
        let ring_header = unsafe { &*self.file_ring };

        let write_pos = ring_header.write_pos.load(Ordering::Acquire);
        let capacity = ring_header.capacity;

        // Check if we have space
        let next_pos = (write_pos + slice_size as u64) % capacity;
        if next_pos == ring_header.read_pos.load(Ordering::Acquire)
            && ring_header.entry_count.load(Ordering::Acquire) > 0
        {
            // Buffer full, advance read position (overwrite oldest)
            let read_pos = ring_header.read_pos.load(Ordering::Acquire);
            let new_read_pos = (read_pos + slice_size as u64) % capacity;
            ring_header.read_pos.store(new_read_pos, Ordering::Release);
            ring_header.entry_count.fetch_sub(1, Ordering::Release);
        }

        // Write the file slice
        let slice_bytes = unsafe {
            std::slice::from_raw_parts(
                &file_slice as *const FileSliceEntry as *const u8,
                slice_size,
            )
        };

        let ring_start = unsafe { self.file_ring.add(1) as *mut u8 };
        let entry_ptr = unsafe { ring_start.add(write_pos as usize) };

        unsafe {
            std::ptr::copy_nonoverlapping(slice_bytes.as_ptr(), entry_ptr, slice_size);
        }

        // Update checksum
        ring_header.update_checksum(slice_bytes);

        // Update positions
        ring_header.write_pos.store(next_pos, Ordering::Release);
        ring_header.entry_count.fetch_add(1, Ordering::Release);

        Ok(())
    }

    pub fn read_file_slices(
        &self,
        max_entries: usize,
    ) -> Result<Vec<FileSliceEntry>, SharedMemoryError> {
        let ring_header = unsafe { &*self.file_ring };
        let slice_size = mem::size_of::<FileSliceEntry>();
        let mut slices = Vec::new();

        let mut read_pos = ring_header.read_pos.load(Ordering::Acquire);
        let write_pos = ring_header.write_pos.load(Ordering::Acquire);
        let capacity = ring_header.capacity;

        while slices.len() < max_entries && read_pos != write_pos {
            let entry_ptr = unsafe {
                let ring_start = self.file_ring.add(1) as *const u8;
                ring_start.add(read_pos as usize) as *const FileSliceEntry
            };

            let file_slice = unsafe { &*entry_ptr }.clone();

            // Validate checksum
            let slice_bytes = unsafe {
                std::slice::from_raw_parts(
                    &file_slice as *const FileSliceEntry as *const u8,
                    slice_size,
                )
            };

            if !ring_header.validate_checksum(slice_bytes) {
                warn!("Checksum validation failed for file slice entry");
                break;
            }

            slices.push(file_slice);

            read_pos = (read_pos + slice_size as u64) % capacity;
        }

        Ok(slices)
    }

    pub fn write_scratch(&self, data: &[u8]) -> Result<(), SharedMemoryError> {
        if self.is_readonly {
            return Err(SharedMemoryError::InvalidAccess(
                "Buffer is readonly".to_string(),
            ));
        }

        let header = unsafe { &*self.header };
        if data.len() > header.scratch_size as usize {
            return Err(SharedMemoryError::Overflow(
                "Data too large for scratch area".to_string(),
            ));
        }

        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.scratch_area, data.len());
        }

        Ok(())
    }

    pub fn read_scratch(&self, buffer: &mut [u8]) -> Result<usize, SharedMemoryError> {
        let header = unsafe { &*self.header };
        let available = header.scratch_size as usize;
        let to_read = buffer.len().min(available);

        unsafe {
            std::ptr::copy_nonoverlapping(self.scratch_area, buffer.as_mut_ptr(), to_read);
        }

        Ok(to_read)
    }

    pub fn get_stats(&self) -> BufferStats {
        let header = unsafe { &*self.header };
        let cmd_ring = unsafe { &*self.command_ring };
        let file_ring = unsafe { &*self.file_ring };

        BufferStats {
            total_size: header.total_size,
            command_entries: cmd_ring.entry_count.load(Ordering::Acquire),
            file_entries: file_ring.entry_count.load(Ordering::Acquire),
            is_readonly: self.is_readonly,
        }
    }

    pub fn flush(&self) -> Result<(), SharedMemoryError> {
        if self.is_readonly {
            return Ok(());
        }

        self.mapping.flush()?;
        Ok(())
    }
}

impl Drop for SharedMemoryBuffer {
    fn drop(&mut self) {
        if !self.is_readonly {
            let _ = self.flush();
        }

        // Clean up shared memory file if it exists
        let shm_path = PathBuf::from("/dev/shm").join(format!("ferroterm-{}", self.id));
        if shm_path.exists() {
            let _ = std::fs::remove_file(shm_path);
        }
    }
}

#[derive(Debug, Clone)]
pub struct BufferStats {
    pub total_size: u64,
    pub command_entries: u64,
    pub file_entries: u64,
    pub is_readonly: bool,
}

pub struct SharedMemoryManager {
    buffers: Arc<RwLock<HashMap<String, Arc<SharedMemoryBuffer>>>>,
    default_cmd_ring_entries: usize,
    default_file_ring_entries: usize,
    default_scratch_size: usize,
}

impl SharedMemoryManager {
    pub fn new() -> Self {
        Self {
            buffers: Arc::new(RwLock::new(HashMap::new())),
            default_cmd_ring_entries: 10000,
            default_file_ring_entries: 2000,
            default_scratch_size: 1024 * 1024, // 1MB
        }
    }

    pub fn create_buffer(&self, id: String) -> Result<Arc<SharedMemoryBuffer>, SharedMemoryError> {
        let buffer = Arc::new(SharedMemoryBuffer::new(
            id.clone(),
            self.default_cmd_ring_entries,
            self.default_file_ring_entries,
            self.default_scratch_size,
        )?);

        self.buffers.write().insert(id, Arc::clone(&buffer));
        Ok(buffer)
    }

    pub fn open_buffer(&self, id: &str) -> Result<Arc<SharedMemoryBuffer>, SharedMemoryError> {
        if let Some(buffer) = self.buffers.read().get(id) {
            return Ok(Arc::clone(buffer));
        }

        let buffer = Arc::new(SharedMemoryBuffer::open_readonly(id)?);
        self.buffers
            .write()
            .insert(id.to_string(), Arc::clone(&buffer));
        Ok(buffer)
    }

    pub fn get_buffer(&self, id: &str) -> Option<Arc<SharedMemoryBuffer>> {
        self.buffers.read().get(id).cloned()
    }

    pub fn remove_buffer(&self, id: &str) -> bool {
        self.buffers.write().remove(id).is_some()
    }

    pub fn list_buffers(&self) -> Vec<String> {
        self.buffers.read().keys().cloned().collect()
    }

    pub fn cleanup(&self) {
        let mut buffers = self.buffers.write();
        buffers.clear();
    }

    pub fn set_defaults(&mut self, cmd_entries: usize, file_entries: usize, scratch_size: usize) {
        self.default_cmd_ring_entries = cmd_entries;
        self.default_file_ring_entries = file_entries;
        self.default_scratch_size = scratch_size;
    }
}

impl Default for SharedMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_buffer_header_creation() {
        let header = BufferHeader::new(1024 * 1024, 64 * 1024, 32 * 1024, 128 * 1024);
        assert_eq!(header.magic, BufferHeader::MAGIC);
        assert_eq!(header.version, BufferHeader::VERSION);
        assert!(header.checksum != 0);
    }

    #[test]
    fn test_buffer_header_validation() {
        let header = BufferHeader::new(1024 * 1024, 64 * 1024, 32 * 1024, 128 * 1024);
        assert!(header.validate().is_ok());

        let mut invalid_header = header;
        invalid_header.magic = 0x12345678;
        assert!(invalid_header.validate().is_err());
    }

    #[test]
    fn test_command_entry_creation() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = CommandEntry {
            timestamp,
            command: "ls -la".to_string(),
            working_dir: "/home/user".to_string(),
            exit_code: Some(0),
            output_length: 1024,
        };

        assert_eq!(entry.command, "ls -la");
        assert_eq!(entry.exit_code, Some(0));
    }

    #[test]
    fn test_file_slice_entry_creation() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = FileSliceEntry {
            timestamp,
            file_path: "/etc/passwd".to_string(),
            start_line: 1,
            end_line: 10,
            content: "root:x:0:0:root:/root:/bin/bash".to_string(),
        };

        assert_eq!(entry.file_path, "/etc/passwd");
        assert_eq!(entry.start_line, 1);
        assert_eq!(entry.end_line, 10);
    }

    #[test]
    fn test_ring_buffer_header() {
        let header = RingBufferHeader::new(1024);
        assert_eq!(header.capacity, 1024);
        assert_eq!(header.write_pos.load(Ordering::Relaxed), 0);
        assert_eq!(header.read_pos.load(Ordering::Relaxed), 0);
        assert_eq!(header.entry_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_shared_memory_manager() {
        let manager = SharedMemoryManager::new();
        assert!(manager.list_buffers().is_empty());

        manager.set_defaults(5000, 1000, 512 * 1024);
        assert_eq!(manager.default_cmd_ring_entries, 5000);
        assert_eq!(manager.default_file_ring_entries, 1000);
        assert_eq!(manager.default_scratch_size, 512 * 1024);
    }

    #[test]
    fn test_buffer_stats() {
        let stats = BufferStats {
            total_size: 1024 * 1024,
            command_entries: 42,
            file_entries: 15,
            is_readonly: false,
        };

        assert_eq!(stats.total_size, 1024 * 1024);
        assert_eq!(stats.command_entries, 42);
        assert_eq!(stats.file_entries, 15);
        assert!(!stats.is_readonly);
    }
}
