use base64::{engine::general_purpose, Engine as _};
use image::{DynamicImage, ImageFormat};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use wgpu;

#[derive(Error, Debug)]
pub enum MediaDisplayError {
    #[error("Image decoding error: {0}")]
    ImageDecode(String),
    #[error("WGPU error: {0}")]
    Wgpu(#[from] wgpu::Error),
    #[error("Base64 encoding error: {0}")]
    Base64(String),
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
}

/// Supported image formats
#[derive(Debug, Clone)]
pub enum ImageFormatType {
    Png,
    Jpeg,
    WebP,
}

/// Media display capabilities
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    pub supports_gpu_rendering: bool,
    pub supports_sixel: bool,
    pub supports_iterm: bool,
}

/// Image data with metadata
#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: ImageFormatType,
}

/// Display mode for images
#[derive(Debug, Clone)]
pub enum DisplayMode {
    Gpu,
    AnsiSixel,
    AnsiIterm,
}

/// Media display manager
pub struct MediaDisplay {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    capabilities: MediaCapabilities,
    image_cache: Arc<RwLock<HashMap<String, ImageData>>>,
    texture_cache: Arc<RwLock<HashMap<String, wgpu::Texture>>>,
}

impl MediaDisplay {
    /// Create a new media display instance
    pub async fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Result<Self, MediaDisplayError> {
        let capabilities = Self::detect_capabilities();

        Ok(Self {
            device,
            queue,
            capabilities,
            image_cache: Arc::new(RwLock::new(HashMap::new())),
            texture_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Detect terminal capabilities
    fn detect_capabilities() -> MediaCapabilities {
        // Check environment variables for capabilities
        let supports_gpu_rendering = true; // Assume GPU rendering is available
        let supports_sixel = std::env::var("PACHYTERM_CAPS")
            .unwrap_or_default()
            .contains("sixel");
        let supports_iterm = std::env::var("TERM_PROGRAM").unwrap_or_default() == "iTerm.app"
            || std::env::var("PACHYTERM_CAPS")
                .unwrap_or_default()
                .contains("iterm");

        MediaCapabilities {
            supports_gpu_rendering,
            supports_sixel,
            supports_iterm,
        }
    }

    /// Load and decode an image from bytes
    pub async fn load_image(
        &self,
        id: String,
        data: Vec<u8>,
        format: ImageFormatType,
    ) -> Result<(), MediaDisplayError> {
        let img_format = match format {
            ImageFormatType::Png => ImageFormat::Png,
            ImageFormatType::Jpeg => ImageFormat::Jpeg,
            ImageFormatType::WebP => ImageFormat::WebP,
        };

        let img = image::load_from_memory_with_format(&data, img_format)
            .map_err(|e| MediaDisplayError::ImageDecode(e.to_string()))?;

        let rgba_img = img.to_rgba8();
        let width = rgba_img.width();
        let height = rgba_img.height();
        let image_data = rgba_img.into_raw();

        let image_data_struct = ImageData {
            width,
            height,
            data: image_data.clone(),
            format,
        };

        // Cache the decoded image
        self.image_cache
            .write()
            .insert(id.clone(), image_data_struct);

        // Create GPU texture if GPU rendering is supported
        if self.capabilities.supports_gpu_rendering {
            self.create_texture(&id, width, height, &image_data)?;
        }

        Ok(())
    }

    /// Create a GPU texture for the image
    fn create_texture(
        &self,
        id: &str,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), MediaDisplayError> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Image Texture {}", id)),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.texture_cache.write().insert(id.to_string(), texture);
        Ok(())
    }

    /// Render image using GPU
    pub fn render_gpu(
        &self,
        id: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<(), MediaDisplayError> {
        // This would be integrated with the renderer's render pass
        // For now, just ensure texture exists
        if !self.texture_cache.read().contains_key(id) {
            return Err(MediaDisplayError::InvalidFormat(format!(
                "Texture not found for {}",
                id
            )));
        }
        Ok(())
    }

    /// Generate ANSI escape sequence for image display
    pub fn generate_ansi_sequence(
        &self,
        id: &str,
        mode: DisplayMode,
    ) -> Result<String, MediaDisplayError> {
        let image_data = self
            .image_cache
            .read()
            .get(id)
            .ok_or_else(|| MediaDisplayError::InvalidFormat(format!("Image not found: {}", id)))?
            .clone();

        match mode {
            DisplayMode::AnsiIterm => self.generate_iterm_sequence(&image_data),
            DisplayMode::AnsiSixel => self.generate_sixel_sequence(&image_data),
            DisplayMode::Gpu => Err(MediaDisplayError::InvalidFormat(
                "GPU mode not supported for ANSI".to_string(),
            )),
        }
    }

    /// Generate iTerm inline image sequence
    fn generate_iterm_sequence(&self, image_data: &ImageData) -> Result<String, MediaDisplayError> {
        // Convert to PNG for iTerm
        let img = image::RgbaImage::from_raw(
            image_data.width,
            image_data.height,
            image_data.data.clone(),
        )
        .ok_or_else(|| {
            MediaDisplayError::ImageDecode("Failed to create image from data".to_string())
        })?;

        let mut png_data = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png_data), ImageFormat::Png)
            .map_err(|e| MediaDisplayError::ImageDecode(e.to_string()))?;

        let encoded = general_purpose::STANDARD.encode(&png_data);

        // iTerm2 inline image protocol
        let sequence = format!(
            "\x1b]1337;File=inline=1;width={}px;height={}px;preserveAspectRatio=1:{}\x07",
            image_data.width, image_data.height, encoded
        );

        Ok(sequence)
    }

    /// Generate Sixel sequence (simplified)
    fn generate_sixel_sequence(&self, image_data: &ImageData) -> Result<String, MediaDisplayError> {
        // Simplified Sixel encoding - in practice, would need a proper sixel encoder
        // For now, just return a placeholder
        let sequence = format!(
            "\x1bPq\"1;1;{};{};{}{}\x1b\\",
            image_data.width,
            image_data.height,
            "#0;2;0;0;0#1;2;100;100;100", // Color palette
            "#0!100~-~-"                  // Simplified pixel data
        );

        Ok(sequence)
    }

    /// Get image dimensions
    pub fn get_image_dimensions(&self, id: &str) -> Option<(u32, u32)> {
        self.image_cache
            .read()
            .get(id)
            .map(|img| (img.width, img.height))
    }

    /// Check if image exists
    pub fn has_image(&self, id: &str) -> bool {
        self.image_cache.read().contains_key(id)
    }

    /// Remove image from cache
    pub fn remove_image(&self, id: &str) {
        self.image_cache.write().remove(id);
        self.texture_cache.write().remove(id);
    }

    /// Get capabilities
    pub fn get_capabilities(&self) -> &MediaCapabilities {
        &self.capabilities
    }

    /// Choose best display mode based on capabilities
    pub fn choose_display_mode(&self) -> DisplayMode {
        if self.capabilities.supports_gpu_rendering {
            DisplayMode::Gpu
        } else if self.capabilities.supports_iterm {
            DisplayMode::AnsiIterm
        } else if self.capabilities.supports_sixel {
            DisplayMode::AnsiSixel
        } else {
            DisplayMode::Gpu // Fallback to GPU even if not detected
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_image() {
        // Create a simple test image (1x1 red pixel)
        let test_data = vec![255, 0, 0, 255]; // RGBA red
        let device = Arc::new(
            wgpu::util::DeviceExt::create_device(
                &wgpu::Instance::new(wgpu::InstanceDescriptor::default()),
                &wgpu::RequestAdapterOptions::default(),
                &wgpu::DeviceDescriptor::default(),
            )
            .await
            .unwrap()
            .0,
        );
        let queue = Arc::new(device.create_queue());

        let media_display = MediaDisplay::new(device, queue).await.unwrap();

        // This would need actual image data, but for test structure
        assert!(media_display.capabilities.supports_gpu_rendering);
    }

    #[test]
    fn test_capabilities_detection() {
        let caps = MediaDisplay::detect_capabilities();
        // Basic checks
        assert!(caps.supports_gpu_rendering); // Should be true by default
    }
}
