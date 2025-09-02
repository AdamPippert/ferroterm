use crate::renderer::{GpuRenderer, RendererError as GpuRendererError, TerminalGrid};
use parking_lot::RwLock;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DualRendererError {
    #[error("GPU renderer error: {0}")]
    Gpu(#[from] GpuRendererError),
    #[error("ANSI renderer error: {0}")]
    Ansi(String),
    #[error("Backend detection error: {0}")]
    Detection(String),
    #[error("Backend switch error: {0}")]
    Switch(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RenderBackend {
    Gpu,
    Ansi,
}

pub struct AnsiRenderer {
    grid: Arc<RwLock<TerminalGrid>>,
    output_buffer: String,
}

impl AnsiRenderer {
    pub fn new(grid: Arc<RwLock<TerminalGrid>>) -> Self {
        Self {
            grid,
            output_buffer: String::new(),
        }
    }

    pub fn render(&mut self) -> Result<String, DualRendererError> {
        let grid = self.grid.read();
        let mut output = String::new();

        // Clear screen
        output.push_str("\x1b[2J\x1b[H");

        // Render each cell
        for y in 0..grid.height {
            for x in 0..grid.width {
                if let Some(cell) = grid.get_cell(x, y) {
                    // Move cursor to position
                    output.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));

                    // Set colors (simplified - would need full ANSI color mapping)
                    let fg_color = self.color_to_ansi(&cell.foreground, true);
                    let bg_color = self.color_to_ansi(&cell.background, false);

                    if fg_color != "39" || bg_color != "49" {
                        output.push_str(&format!("\x1b[{};{}m", fg_color, bg_color));
                    }

                    // Add text attributes
                    if cell.bold {
                        output.push_str("\x1b[1m");
                    }
                    if cell.italic {
                        output.push_str("\x1b[3m");
                    }
                    if cell.underline {
                        output.push_str("\x1b[4m");
                    }

                    // Output character
                    output.push(cell.character);

                    // Reset attributes
                    output.push_str("\x1b[0m");
                }
            }
        }

        // Render cursor
        if grid.cursor_visible {
            output.push_str(&format!(
                "\x1b[{};{}H",
                grid.cursor_y + 1,
                grid.cursor_x + 1
            ));
            output.push_str("\x1b[?25h"); // Show cursor
        } else {
            output.push_str("\x1b[?25l"); // Hide cursor
        }

        Ok(output)
    }

    fn color_to_ansi(&self, color: &[f32; 4], foreground: bool) -> String {
        // Simplified ANSI color mapping
        // In a full implementation, this would support 256-color and true-color ANSI codes
        let r = (color[0] * 255.0) as u8;
        let g = (color[1] * 255.0) as u8;
        let b = (color[2] * 255.0) as u8;

        // Map to basic ANSI colors (simplified)
        match (r, g, b) {
            (0, 0, 0) => {
                if foreground {
                    "30"
                } else {
                    "40"
                }
            } // Black
            (255, 0, 0) => {
                if foreground {
                    "31"
                } else {
                    "41"
                }
            } // Red
            (0, 255, 0) => {
                if foreground {
                    "32"
                } else {
                    "42"
                }
            } // Green
            (255, 255, 0) => {
                if foreground {
                    "33"
                } else {
                    "43"
                }
            } // Yellow
            (0, 0, 255) => {
                if foreground {
                    "34"
                } else {
                    "44"
                }
            } // Blue
            (255, 0, 255) => {
                if foreground {
                    "35"
                } else {
                    "45"
                }
            } // Magenta
            (0, 255, 255) => {
                if foreground {
                    "36"
                } else {
                    "46"
                }
            } // Cyan
            (255, 255, 255) => {
                if foreground {
                    "37"
                } else {
                    "47"
                }
            } // White
            _ => {
                if foreground {
                    "39"
                } else {
                    "49"
                }
            } // Default
        }
        .to_string()
    }
}

pub struct DualRenderer {
    gpu_renderer: Option<GpuRenderer>,
    ansi_renderer: AnsiRenderer,
    current_backend: RenderBackend,
    grid: Arc<RwLock<TerminalGrid>>,
    gpu_available: bool,
    force_ansi: bool,
}

impl DualRenderer {
    pub async fn new<W>(
        window: Option<&W>,
        width: u32,
        height: u32,
        font_size: f32,
        force_ansi: bool,
    ) -> Result<Self, DualRendererError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        let grid = Arc::new(RwLock::new(TerminalGrid::new(
            (width as f32 / (font_size * 0.6)) as u32,
            (height as f32 / (font_size * 1.2)) as u32,
        )));

        let ansi_renderer = AnsiRenderer::new(Arc::clone(&grid));

        let mut gpu_renderer = None;
        let mut gpu_available = false;

        if !force_ansi {
            if let Some(window) = window {
                match GpuRenderer::new(window, width, height, font_size).await {
                    Ok(renderer) => {
                        gpu_renderer = Some(renderer);
                        gpu_available = true;
                    }
                    Err(e) => {
                        eprintln!(
                            "GPU renderer initialization failed, falling back to ANSI: {:?}",
                            e
                        );
                    }
                }
            }
        }

        let current_backend = if gpu_available && !force_ansi {
            RenderBackend::Gpu
        } else {
            RenderBackend::Ansi
        };

        Ok(Self {
            gpu_renderer,
            ansi_renderer,
            current_backend,
            grid,
            gpu_available,
            force_ansi,
        })
    }

    pub fn detect_backend_capabilities() -> Result<BackendCapabilities, DualRendererError> {
        // Check for GPU availability
        let gpu_available = Self::check_gpu_availability();

        // Check for ANSI terminal support
        let ansi_available = Self::check_ansi_support();

        Ok(BackendCapabilities {
            gpu_available,
            ansi_available,
            vulkan_supported: Self::check_vulkan_support(),
            metal_supported: Self::check_metal_support(),
            opengl_supported: Self::check_opengl_support(),
        })
    }

    fn check_gpu_availability() -> bool {
        // This would check for GPU drivers and wgpu compatibility
        // For now, return true on most systems
        true
    }

    fn check_ansi_support() -> bool {
        // Check if we're running in a compatible terminal
        std::env::var("TERM").is_ok()
    }

    fn check_vulkan_support() -> bool {
        // Check for Vulkan installation
        // This would query the system for Vulkan libraries
        true
    }

    fn check_metal_support() -> bool {
        // Check for Metal support (macOS only)
        cfg!(target_os = "macos")
    }

    fn check_opengl_support() -> bool {
        // Check for OpenGL support
        true
    }

    pub fn get_current_backend(&self) -> RenderBackend {
        self.current_backend
    }

    pub fn switch_backend(&mut self, backend: RenderBackend) -> Result<(), DualRendererError> {
        match backend {
            RenderBackend::Gpu => {
                if !self.gpu_available {
                    return Err(DualRendererError::Switch(
                        "GPU backend not available".to_string(),
                    ));
                }
                if self.gpu_renderer.is_none() {
                    return Err(DualRendererError::Switch(
                        "GPU renderer not initialized".to_string(),
                    ));
                }
            }
            RenderBackend::Ansi => {
                // ANSI is always available
            }
        }

        self.current_backend = backend;
        Ok(())
    }

    pub fn render(&mut self) -> Result<Option<String>, DualRendererError> {
        match self.current_backend {
            RenderBackend::Gpu => {
                if let Some(ref mut gpu_renderer) = self.gpu_renderer {
                    gpu_renderer.render()?;
                    Ok(None) // GPU rendering doesn't return text output
                } else {
                    Err(DualRendererError::Switch(
                        "GPU renderer not available".to_string(),
                    ))
                }
            }
            RenderBackend::Ansi => {
                let output = self.ansi_renderer.render()?;
                Ok(Some(output))
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(ref mut gpu_renderer) = self.gpu_renderer {
            gpu_renderer.resize(width, height);
        }

        // Update grid size
        let mut grid = self.grid.write();
        let font_size = 12.0; // Default font size
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.2;

        let new_width = (width as f32 / cell_width) as u32;
        let new_height = (height as f32 / cell_height) as u32;

        if new_width != grid.width || new_height != grid.height {
            *grid = TerminalGrid::new(new_width, new_height);
        }
    }

    pub fn update_grid<F>(&self, updater: F)
    where
        F: FnOnce(&mut TerminalGrid),
    {
        let mut grid = self.grid.write();
        updater(&mut grid);
    }

    pub fn get_grid(&self) -> Arc<RwLock<TerminalGrid>> {
        Arc::clone(&self.grid)
    }

    pub fn is_gpu_available(&self) -> bool {
        self.gpu_available
    }

    pub fn get_backend_info(&self) -> BackendInfo {
        BackendInfo {
            current_backend: self.current_backend,
            gpu_available: self.gpu_available,
            force_ansi: self.force_ansi,
        }
    }
}

#[derive(Debug)]
pub struct BackendCapabilities {
    pub gpu_available: bool,
    pub ansi_available: bool,
    pub vulkan_supported: bool,
    pub metal_supported: bool,
    pub opengl_supported: bool,
}

#[derive(Debug)]
pub struct BackendInfo {
    pub current_backend: RenderBackend,
    pub gpu_available: bool,
    pub force_ansi: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_detection() {
        let caps = DualRenderer::detect_backend_capabilities().unwrap();
        assert!(caps.ansi_available); // ANSI should always be available in test environment
    }

    #[test]
    fn test_ansi_renderer() {
        let grid = Arc::new(RwLock::new(TerminalGrid::new(10, 5)));
        let mut renderer = AnsiRenderer::new(Arc::clone(&grid));

        // Add some test content
        {
            let mut grid = grid.write();
            grid.set_cell(
                0,
                0,
                crate::renderer::TerminalCell {
                    character: 'H',
                    foreground: [1.0, 1.0, 1.0, 1.0],
                    background: [0.0, 0.0, 0.0, 1.0],
                    bold: false,
                    italic: false,
                    underline: false,
                },
            );
        }

        let output = renderer.render().unwrap();
        assert!(output.contains("\x1b[2J")); // Clear screen
        assert!(output.contains("\x1b[H")); // Home cursor
        assert!(output.contains('H')); // Our test character
    }

    #[test]
    fn test_color_to_ansi() {
        let grid = Arc::new(RwLock::new(TerminalGrid::new(1, 1)));
        let renderer = AnsiRenderer::new(grid);

        // Test black foreground
        assert_eq!(renderer.color_to_ansi(&[0.0, 0.0, 0.0, 1.0], true), "30");
        // Test white foreground
        assert_eq!(renderer.color_to_ansi(&[1.0, 1.0, 1.0, 1.0], true), "37");
        // Test red background
        assert_eq!(renderer.color_to_ansi(&[1.0, 0.0, 0.0, 1.0], false), "41");
    }
}
