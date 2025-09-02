use bytemuck;
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping, CacheKey, SwashCache, TextArea, LayoutRun, Color as CosmicColor};
use font_kit::{
    family_name::FamilyName, font::Font, properties::Properties, source::SystemSource,
};
use glyph_brush::{ab_glyph::FontArc, GlyphBrush, GlyphBrushBuilder, Section, Text};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use swash::{FontRef, CacheKey as SwashCacheKey};
use thiserror::Error;
use tokio::sync::mpsc;
use unicode_segmentation::UnicodeSegmentation;
use wgpu;

use crate::markdown_renderer::{MarkdownTerminalRenderer, MarkdownError, RenderContext};

#[derive(Error, Debug)]
pub enum RendererError {
    #[error("WGPU error: {0}")]
    Wgpu(#[from] wgpu::Error),
    #[error("Surface error: {0}")]
    Surface(String),
    #[error("Font loading error: {0}")]
    Font(String),
    #[error("Glyph rendering error: {0}")]
    Glyph(String),
    #[error("Atlas error: {0}")]
    Atlas(String),
    #[error("Shader compilation error: {0}")]
    Shader(String),
    #[error("Performance error: {0}")]
    Performance(String),
    #[error("Markdown error: {0}")]
    Markdown(#[from] MarkdownError),
}

#[derive(Debug, Clone)]
pub struct TerminalCell {
    pub character: char,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub dim: bool,
    pub reverse: bool,
    pub blink: bool,
    pub wide: bool, // For double-width characters
    pub double_height: bool,
    pub dirty: bool, // For dirty region tracking
}

pub struct TerminalGrid {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<TerminalCell>,
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub cursor_visible: bool,
}

impl TerminalGrid {
    pub fn new(width: u32, height: u32) -> Self {
        let mut cells = Vec::with_capacity((width * height) as usize);
        for _ in 0..(width * height) {
            cells.push(TerminalCell {
                character: ' ',
                foreground: [1.0, 1.0, 1.0, 1.0], // White
                background: [0.0, 0.0, 0.0, 1.0], // Black
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

        Self {
            width,
            height,
            cells,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
        }
    }

    pub fn get_cell(&self, x: u32, y: u32) -> Option<&TerminalCell> {
        if x < self.width && y < self.height {
            self.cells.get((y * self.width + x) as usize)
        } else {
            None
        }
    }

    pub fn set_cell(&mut self, x: u32, y: u32, cell: TerminalCell) {
        if x < self.width && y < self.height {
            let index = (y * self.width + x) as usize;
            if index < self.cells.len() {
                self.cells[index] = cell;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorStyle {
    Block,
    Beam,
    Underline,
}

#[derive(Debug, Clone)]
pub struct SelectionRange {
    pub start_x: u32,
    pub start_y: u32,
    pub end_x: u32,
    pub end_y: u32,
}

#[derive(Debug, Clone)]
pub struct DirtyRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub timestamp: Instant,
}

#[derive(Debug)]
pub struct PerformanceMetrics {
    pub frame_time: Duration,
    pub cpu_usage: f32,
    pub gpu_memory: u64,
    pub atlas_usage: f32,
    pub dirty_regions: usize,
    pub glyph_cache_hits: u64,
    pub glyph_cache_misses: u64,
}

pub struct GlyphAtlas {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub size: u32,
    pub layer_count: u32,
    pub current_layer: u32,
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub line_height: u32,
    pub glyph_map: HashMap<CacheKey, GlyphLocation>,
    pub usage_stats: HashMap<CacheKey, u64>,
}

#[derive(Debug, Clone)]
pub struct GlyphLocation {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub layer: u32,
    pub left: i32,
    pub top: i32,
    pub advance_width: f32,
}

pub struct FontManager {
    pub system_source: SystemSource,
    pub primary_font: Option<FontArc>,
    pub fallback_fonts: Vec<FontArc>,
    pub emoji_font: Option<FontArc>,
    pub font_size: f32,
    pub line_height: f32,
    pub character_width: f32,
    pub properties: Properties,
}

impl FontManager {
    pub fn new(font_size: f32) -> Result<Self, RendererError> {
        let system_source = SystemSource::new();
        
        // Load primary monospace font
        let primary_font = system_source
            .select_best_match(
                &[FamilyName::Monospace, FamilyName::SansSerif],
                &Properties::default(),
            )
            .and_then(|handle| handle.load())
            .ok()
            .and_then(|font| FontArc::try_from_vec(font.copy_font_data().unwrap_or_default()).ok());
        
        // Load fallback fonts
        let mut fallback_fonts = Vec::new();
        for family in &[
            FamilyName::Title("Menlo".to_string()),
            FamilyName::Title("Consolas".to_string()),
            FamilyName::Title("Monaco".to_string()),
            FamilyName::Title("DejaVu Sans Mono".to_string()),
        ] {
            if let Ok(handle) = system_source.select_best_match(&[family.clone()], &Properties::default()) {
                if let Ok(font) = handle.load() {
                    if let Ok(font_arc) = FontArc::try_from_vec(font.copy_font_data().unwrap_or_default()) {
                        fallback_fonts.push(font_arc);
                    }
                }
            }
        }
        
        // Try to load emoji font
        let emoji_font = system_source
            .select_best_match(
                &[FamilyName::Title("Apple Color Emoji".to_string())],
                &Properties::default(),
            )
            .and_then(|handle| handle.load())
            .ok()
            .and_then(|font| FontArc::try_from_vec(font.copy_font_data().unwrap_or_default()).ok());
        
        let line_height = font_size * 1.2;
        let character_width = font_size * 0.6;
        
        Ok(Self {
            system_source,
            primary_font,
            fallback_fonts,
            emoji_font,
            font_size,
            line_height,
            character_width,
            properties: Properties::default(),
        })
    }
    
    pub fn get_best_font(&self, character: char) -> Option<&FontArc> {
        // Check if character is emoji
        if character.is_emoji() {
            if let Some(ref emoji_font) = self.emoji_font {
                return Some(emoji_font);
            }
        }
        
        // Check primary font first
        if let Some(ref primary) = self.primary_font {
            if primary.glyph_id(character).0 != 0 {
                return Some(primary);
            }
        }
        
        // Check fallback fonts
        for font in &self.fallback_fonts {
            if font.glyph_id(character).0 != 0 {
                return Some(font);
            }
        }
        
        // Default to primary font even if glyph is missing
        self.primary_font.as_ref()
    }
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device, size: u32, layer_count: u32) -> Result<Self, RendererError> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm, // Single channel for text
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        
        Ok(Self {
            texture,
            texture_view,
            size,
            layer_count,
            current_layer: 0,
            cursor_x: 0,
            cursor_y: 0,
            line_height: 0,
            glyph_map: HashMap::new(),
            usage_stats: HashMap::new(),
        })
    }
    
    pub fn allocate_glyph(&mut self, width: u32, height: u32) -> Option<GlyphLocation> {
        // Simple allocation strategy - can be improved with better packing
        if self.cursor_x + width > self.size {
            self.cursor_x = 0;
            self.cursor_y += self.line_height;
            self.line_height = 0;
        }
        
        if self.cursor_y + height > self.size {
            // Move to next layer
            self.current_layer += 1;
            if self.current_layer >= self.layer_count {
                return None; // Atlas is full
            }
            self.cursor_x = 0;
            self.cursor_y = 0;
            self.line_height = 0;
        }
        
        let location = GlyphLocation {
            x: self.cursor_x,
            y: self.cursor_y,
            width,
            height,
            layer: self.current_layer,
            left: 0,
            top: 0,
            advance_width: width as f32,
        };
        
        self.cursor_x += width;
        self.line_height = self.line_height.max(height);
        
        Some(location)
    }
    
    pub fn evict_lru(&mut self, count: usize) {
        // Find least recently used glyphs
        let mut sorted_glyphs: Vec<_> = self.usage_stats.iter().collect();
        sorted_glyphs.sort_by_key(|(_, usage)| *usage);
        
        for (cache_key, _) in sorted_glyphs.iter().take(count) {
            self.glyph_map.remove(cache_key);
            self.usage_stats.remove(cache_key);
        }
    }
}

impl Vertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

pub struct GpuRenderer {
    // Core wgpu components
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    
    // Rendering pipeline
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    
    // Font and text rendering
    font_system: FontSystem,
    swash_cache: SwashCache,
    glyph_brush: GlyphBrush<()>,
    font_manager: FontManager,
    glyph_atlas: GlyphAtlas,
    font_sampler: wgpu::Sampler,
    font_bind_group: wgpu::BindGroup,
    font_bind_group_layout: wgpu::BindGroupLayout,
    
    // Terminal state
    grid: Arc<RwLock<TerminalGrid>>,
    cell_width: f32,
    cell_height: f32,
    
    // Rendering state
    cursor_style: CursorStyle,
    cursor_blink_timer: Instant,
    cursor_visible: bool,
    selection: Option<SelectionRange>,
    dirty_regions: Vec<DirtyRegion>,
    
    // Performance and features
    ligatures_enabled: bool,
    triple_buffering: bool,
    vsync_enabled: bool,
    target_fps: u32,
    last_frame_time: Instant,
    performance_metrics: PerformanceMetrics,
    
    // Streaming support
    streaming_sessions: Arc<RwLock<HashMap<String, StreamingSession>>>,
    stream_update_tx: mpsc::UnboundedSender<(String, StreamUpdate)>,
    stream_update_rx: mpsc::UnboundedReceiver<(String, StreamUpdate)>,
    markdown_renderer: Option<MarkdownTerminalRenderer>,
    
    // GPU memory management
    gpu_memory_usage: u64,
    max_gpu_memory: u64,
}

impl GpuRenderer {
    pub async fn new<W>(
        window: &W,
        width: u32,
        height: u32,
        font_size: f32,
    ) -> Result<Self, RendererError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        // Create WGPU instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: Default::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });

        // Create surface
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: window.display_handle().map_err(|e| {
                    RendererError::Surface(format!("Failed to get display handle: {:?}", e))
                })?.into(),
                raw_window_handle: window.window_handle().map_err(|e| {
                    RendererError::Surface(format!("Failed to get window handle: {:?}", e))
                })?.into(),
            })
        }
        .map_err(|e| RendererError::Surface(format!("Failed to create surface: {:?}", e)))?;

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RendererError::Surface(
                "No suitable adapter found".to_string(),
            ))?;

        // Request device with enhanced features
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::TEXTURE_BINDING_ARRAY
                        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
                        | wgpu::Features::MULTI_DRAW_INDIRECT,
                    required_limits: wgpu::Limits {
                        max_bind_groups: 8,
                        max_texture_array_layers: 256,
                        max_storage_buffer_binding_size: 134217728, // 128MB
                        ..wgpu::Limits::default()
                    },
                    label: Some("Ferroterm GPU Device"),
                },
                None,
            )
            .await
            .map_err(|e| RendererError::Surface(format!("Failed to request device: {:?}", e)))?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Configure for 144 FPS target with triple buffering
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox // Triple buffering for high FPS
        } else if surface_caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate // No vsync for maximum performance
        } else {
            wgpu::PresentMode::Fifo // Fallback to vsync
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 3, // Triple buffering
        };
        surface.configure(&device, &config);

        // Initialize advanced font system
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let font_manager = FontManager::new(font_size)?;
        
        // Initialize glyph atlas with layered texture for better memory management
        let glyph_atlas = GlyphAtlas::new(&device, 2048, 16)?;
        
        // Create high-performance glyph brush
        let glyph_brush = if let Some(ref primary_font) = font_manager.primary_font {
            GlyphBrushBuilder::using_font(primary_font.clone())
                .initial_cache_size((2048, 2048))
                .multithread(true)
                .build()
        } else {
            return Err(RendererError::Font("No suitable font found".to_string()));
        };

        // Create optimized sampler for text rendering
        let font_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
            label: Some("font_sampler"),
        });

        // Create enhanced bind group layout for text rendering
        let font_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    // Glyph atlas texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    // Sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // Uniform buffer for rendering parameters
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("font_bind_group_layout"),
            });

        // Create uniform buffer for rendering parameters
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: 256, // Enough for transformation matrices and rendering parameters
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let font_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &font_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glyph_atlas.texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&font_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("font_bind_group"),
        });

        // Load appropriate shader based on platform
        let shader_source = if cfg!(target_os = "macos") && device.features().contains(wgpu::Features::SHADER_F16) {
            include_str!("shaders/metal_text.wgsl")
        } else {
            include_str!("shaders/text.wgsl")
        };
        
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Text Render Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&font_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        // Create larger buffers for high-performance rendering
        let max_quads = (width * height / 16) as usize; // Estimate max glyphs per frame
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: (max_quads * 4 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Index Buffer"),
            size: (max_quads * 6 * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Calculate cell dimensions from font metrics
        let cell_width = font_manager.character_width;
        let cell_height = font_manager.line_height;

        let grid = Arc::new(RwLock::new(TerminalGrid::new(
            (width as f32 / cell_width) as u32,
            (height as f32 / cell_height) as u32,
        )));

        // Initialize streaming components
        let streaming_sessions = Arc::new(RwLock::new(HashMap::new()));
        let (stream_update_tx, stream_update_rx) = mpsc::unbounded_channel();

        // Initialize markdown renderer
        let grid_width = (width as f32 / cell_width) as usize;
        let grid_height = (height as f32 / cell_height) as usize;
        let markdown_context = RenderContext {
            terminal_width: grid_width,
            supports_truecolor: true,
            supports_256color: true,
            supports_unicode: true,
            tab_width: 4,
            code_theme: "base16-ocean.dark".to_string(),
            wrap_code: true,
            show_line_numbers: false,
        };
        let markdown_renderer = MarkdownTerminalRenderer::new(markdown_context, grid_width, grid_height).ok();

        Ok(Self {
            // Core wgpu components
            device,
            queue,
            surface,
            config,
            
            // Rendering pipeline
            render_pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            
            // Font and text rendering
            font_system,
            swash_cache,
            glyph_brush,
            font_manager,
            glyph_atlas,
            font_sampler,
            font_bind_group,
            font_bind_group_layout,
            
            // Terminal state
            grid,
            cell_width,
            cell_height,
            
            // Rendering state
            cursor_style: CursorStyle::Block,
            cursor_blink_timer: now,
            cursor_visible: true,
            selection: None,
            dirty_regions: Vec::new(),
            
            // Performance and features
            ligatures_enabled: true,
            triple_buffering: present_mode == wgpu::PresentMode::Mailbox,
            vsync_enabled: present_mode == wgpu::PresentMode::Fifo,
            target_fps: 144,
            last_frame_time: now,
            performance_metrics,
            
            // Streaming support
            streaming_sessions,
            stream_update_tx,
            stream_update_rx,
            markdown_renderer,
            
            // GPU memory management
            gpu_memory_usage: 0,
            max_gpu_memory: 200 * 1024 * 1024, // 200MB limit
        })
    }

    /// Detect hardware capabilities and optimize settings
    pub fn detect_hardware_capabilities(&mut self) -> Result<(), RendererError> {
        let adapter_info = self.device.adapter_info();
        
        // Adjust settings based on GPU capabilities
        match adapter_info.backend {
            wgpu::Backend::Vulkan => {
                // Optimize for Vulkan
                self.target_fps = 144;
            },
            wgpu::Backend::Metal => {
                // Optimize for Metal on macOS
                self.target_fps = 120; // Match ProMotion displays
            },
            wgpu::Backend::Dx12 => {
                // Optimize for DirectX 12
                self.target_fps = 144;
            },
            _ => {
                // Conservative settings for other backends
                self.target_fps = 60;
            }
        }
        
        // Estimate available GPU memory
        self.max_gpu_memory = match adapter_info.device {
            wgpu::DeviceType::DiscreteGpu => 200 * 1024 * 1024, // 200MB for discrete GPU
            wgpu::DeviceType::IntegratedGpu => 100 * 1024 * 1024, // 100MB for integrated GPU
            _ => 50 * 1024 * 1024, // 50MB for other types
        };
        
        Ok(())
    }
    
    /// Update dirty regions for efficient rendering
    pub fn mark_dirty_region(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let region = DirtyRegion {
            x,
            y,
            width,
            height,
            timestamp: Instant::now(),
        };
        
        // Merge overlapping regions
        let mut merged = false;
        for existing_region in &mut self.dirty_regions {
            if self.regions_overlap(existing_region, &region) {
                *existing_region = self.merge_regions(existing_region, &region);
                merged = true;
                break;
            }
        }
        
        if !merged {
            self.dirty_regions.push(region);
        }
        
        // Limit number of dirty regions to prevent performance issues
        if self.dirty_regions.len() > 32 {
            // Merge all regions into one
            let mut min_x = u32::MAX;
            let mut min_y = u32::MAX;
            let mut max_x = 0;
            let mut max_y = 0;
            
            for region in &self.dirty_regions {
                min_x = min_x.min(region.x);
                min_y = min_y.min(region.y);
                max_x = max_x.max(region.x + region.width);
                max_y = max_y.max(region.y + region.height);
            }
            
            self.dirty_regions.clear();
            self.dirty_regions.push(DirtyRegion {
                x: min_x,
                y: min_y,
                width: max_x - min_x,
                height: max_y - min_y,
                timestamp: Instant::now(),
            });
        }
    }
    
    fn regions_overlap(&self, a: &DirtyRegion, b: &DirtyRegion) -> bool {
        a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
    }
    
    fn merge_regions(&self, a: &DirtyRegion, b: &DirtyRegion) -> DirtyRegion {
        let min_x = a.x.min(b.x);
        let min_y = a.y.min(b.y);
        let max_x = (a.x + a.width).max(b.x + b.width);
        let max_y = (a.y + a.height).max(b.y + b.height);
        
        DirtyRegion {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
            timestamp: Instant::now(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);

            // Update grid size
            let mut grid = self.grid.write();
            let new_width = (width as f32 / self.cell_width) as u32;
            let new_height = (height as f32 / self.cell_height) as u32;

            if new_width != grid.width || new_height != grid.height {
                *grid = TerminalGrid::new(new_width, new_height);
                
                // Update markdown renderer size
                if let Some(ref mut markdown_renderer) = self.markdown_renderer {
                    markdown_renderer.resize(new_width as usize, new_height as usize);
                }
            }
        }
    }

    pub fn render(&mut self) -> Result<(), RendererError> {
        let frame_start = Instant::now();
        
        // Update performance metrics
        self.update_performance_metrics();
        
        // Skip frame if we're running too fast (frame rate limiting)
        let target_frame_time = Duration::from_nanos(1_000_000_000 / self.target_fps as u64);
        let elapsed = frame_start.duration_since(self.last_frame_time);
        if elapsed < target_frame_time {
            // Early return to maintain target frame rate
            return Ok(());
        }
        
        // Process streaming updates
        self.process_stream_updates()?;
        
        // Update cursor blink state
        if frame_start.duration_since(self.cursor_blink_timer) > Duration::from_millis(500) {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_timer = frame_start;
        }
        
        // Get surface texture with error recovery
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Timeout | wgpu::SurfaceError::Outdated) => {
                // Surface needs reconfiguration
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture().map_err(|e| {
                    RendererError::Surface(format!("Failed to get surface texture after reconfigure: {:?}", e))
                })?
            }
            Err(e) => {
                return Err(RendererError::Surface(format!("Surface error: {:?}", e)));
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Update uniform buffer with current frame parameters
        self.update_uniform_buffer();

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if self.dirty_regions.is_empty() {
                            wgpu::LoadOp::Load // Don't clear if no changes
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.font_bind_group, &[]);

            // Render only dirty regions for better performance
            if self.dirty_regions.is_empty() {
                // Full render if no dirty regions specified
                self.render_grid(&mut render_pass)?;
            } else {
                // Render only dirty regions
                for region in &self.dirty_regions.clone() {
                    self.render_grid_region(&mut render_pass, region)?;
                }
                self.dirty_regions.clear();
            }
            
            // Render cursor
            if self.cursor_visible {
                self.render_cursor(&mut render_pass)?;
            }
            
            // Render selection
            if let Some(ref selection) = self.selection {
                self.render_selection(&mut render_pass, selection)?;
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        
        // Update timing
        self.last_frame_time = frame_start;
        self.performance_metrics.frame_time = frame_start.elapsed();

        Ok(())
    }
    
    fn update_performance_metrics(&mut self) {
        // Update performance counters
        let gpu_memory = self.estimate_gpu_memory_usage();
        self.performance_metrics.gpu_memory = gpu_memory;
        self.performance_metrics.atlas_usage = self.glyph_atlas.glyph_map.len() as f32 / 
            (self.glyph_atlas.size * self.glyph_atlas.size * self.glyph_atlas.layer_count) as f32;
        self.performance_metrics.dirty_regions = self.dirty_regions.len();
    }
    
    fn estimate_gpu_memory_usage(&self) -> u64 {
        let vertex_buffer_size = self.vertex_buffer.size();
        let index_buffer_size = self.index_buffer.size();
        let uniform_buffer_size = self.uniform_buffer.size();
        let atlas_size = (self.glyph_atlas.size * self.glyph_atlas.size * self.glyph_atlas.layer_count * 4) as u64;
        
        vertex_buffer_size + index_buffer_size + uniform_buffer_size + atlas_size
    }
    
    fn update_uniform_buffer(&mut self) {
        // Update uniform buffer with current rendering parameters
        let uniform_data = [
            self.config.width as f32, self.config.height as f32,
            self.cell_width, self.cell_height,
            self.font_manager.font_size, 0.0, 0.0, 0.0, // Padding for alignment
        ];
        
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&uniform_data),
        );
    }

    fn render_grid<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        let grid = self.grid.read();

        // For each cell in the grid, render the character
        for y in 0..grid.height {
            for x in 0..grid.width {
                if let Some(cell) = grid.get_cell(x, y) {
                    self.render_cell(render_pass, x, y, cell);
                }
            }
        }

        // Render cursor if visible
        if grid.cursor_visible {
            self.render_cursor(render_pass, grid.cursor_x, grid.cursor_y);
        }
    }

    fn render_cell<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        x: u32,
        y: u32,
        cell: &TerminalCell,
    ) {
        // Calculate position
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;

        // Create vertices for the character quad
        let vertices = [
            Vertex {
                position: [x_pos, y_pos],
                tex_coords: [0.0, 0.0],
                color: cell.foreground,
            },
            Vertex {
                position: [x_pos + self.cell_width, y_pos],
                tex_coords: [1.0, 0.0],
                color: cell.foreground,
            },
            Vertex {
                position: [x_pos, y_pos + self.cell_height],
                tex_coords: [0.0, 1.0],
                color: cell.foreground,
            },
            Vertex {
                position: [x_pos + self.cell_width, y_pos + self.cell_height],
                tex_coords: [1.0, 1.0],
                color: cell.foreground,
            },
        ];

        let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];

        // Update buffers
        self.queue
            .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        self.queue
            .write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }

    fn render_cursor<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, x: u32, y: u32) {
        // Simple cursor as a filled rectangle
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;

        let vertices = [
            Vertex {
                position: [x_pos, y_pos],
                tex_coords: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0], // White cursor
            },
            Vertex {
                position: [x_pos + self.cell_width, y_pos],
                tex_coords: [1.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            Vertex {
                position: [x_pos, y_pos + self.cell_height],
                tex_coords: [0.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            Vertex {
                position: [x_pos + self.cell_width, y_pos + self.cell_height],
                tex_coords: [1.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
        ];

        let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];

        self.queue
            .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        self.queue
            .write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
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

    /// Start a new streaming session
    pub fn start_streaming_session(
        &self,
        session_id: String,
        start_line: u32,
        markdown_mode: bool,
    ) -> Result<(), RendererError> {
        let mut sessions = self.streaming_sessions.write();
        let session = StreamingSession {
            id: session_id.clone(),
            start_line,
            current_line: start_line,
            content: String::new(),
            is_active: true,
            markdown_mode,
        };
        sessions.insert(session_id, session);
        Ok(())
    }

    /// Update a streaming session with new content
    pub fn update_streaming_session(
        &self,
        session_id: &str,
        update: StreamUpdate,
    ) -> Result<(), RendererError> {
        let mut sessions = self.streaming_sessions.write();
        if let Some(session) = sessions.get_mut(session_id) {
            match update {
                StreamUpdate::Text(text) | StreamUpdate::Markdown(text) => {
                    session.content.push_str(&text);
                    // In a real implementation, this would render the text to the grid
                    // For now, we'll just update the session state
                }
                StreamUpdate::Finish => {
                    session.is_active = false;
                }
                StreamUpdate::Interrupt => {
                    session.is_active = false;
                    session.content.push_str(" [INTERRUPTED]");
                }
            }
        }
        Ok(())
    }

    /// End a streaming session
    pub fn end_streaming_session(&self, session_id: &str) -> Result<(), RendererError> {
        let mut sessions = self.streaming_sessions.write();
        if let Some(session) = sessions.get_mut(session_id) {
            session.is_active = false;
        }
        sessions.remove(session_id);
        Ok(())
    }

    /// Get streaming session info
    pub fn get_streaming_session(&self, session_id: &str) -> Option<StreamingSession> {
        let sessions = self.streaming_sessions.read();
        sessions.get(session_id).cloned()
    }

    /// Get all active streaming sessions
    pub fn get_active_streaming_sessions(&self) -> Vec<StreamingSession> {
        let sessions = self.streaming_sessions.read();
        sessions.values().filter(|s| s.is_active).cloned().collect()
    }

    /// Render markdown content to terminal cells
    fn render_markdown_to_cells(
        &self,
        content: &str,
        start_x: u32,
        start_y: u32,
        max_width: u32,
    ) -> Vec<(u32, u32, TerminalCell)> {
        let mut cells = Vec::new();
        let mut x = start_x;
        let mut y = start_y;

        // Simple markdown parser (basic implementation)
        for line in content.lines() {
            if line.starts_with("# ") {
                // Header
                for (i, ch) in line.chars().enumerate() {
                    if x + i as u32 >= max_width {
                        break;
                    }
                    let mut cell = TerminalCell {
                        character: ch,
                        foreground: [1.0, 1.0, 1.0, 1.0], // White
                        background: [0.0, 0.0, 0.0, 1.0], // Black
                        bold: true,
                        italic: false,
                        underline: false,
                    };
                    cells.push((x + i as u32, y, cell));
                }
            } else if line.starts_with("```") {
                // Code block
                for (i, ch) in line.chars().enumerate() {
                    if x + i as u32 >= max_width {
                        break;
                    }
                    let cell = TerminalCell {
                        character: ch,
                        foreground: [0.7, 0.7, 0.7, 1.0], // Gray
                        background: [0.1, 0.1, 0.1, 1.0], // Dark gray
                        bold: false,
                        italic: false,
                        underline: false,
                    };
                    cells.push((x + i as u32, y, cell));
                }
            } else {
                // Regular text
                for (i, ch) in line.chars().enumerate() {
                    if x + i as u32 >= max_width {
                        break;
                    }
                    let cell = TerminalCell {
                        character: ch,
                        foreground: [1.0, 1.0, 1.0, 1.0], // White
                        background: [0.0, 0.0, 0.0, 1.0], // Black
                        bold: false,
                        italic: false,
                        underline: false,
                    };
                    cells.push((x + i as u32, y, cell));
                }
            }
            y += 1;
            x = start_x;
        }

        cells
    }

    /// Process streaming updates (call this in the render loop)
    pub fn process_stream_updates(&mut self) -> Result<(), RendererError> {
        while let Ok((session_id, update)) = self.stream_update_rx.try_recv() {
            self.update_streaming_session(&session_id, update)?;
        }
        Ok(())
    }

    /// Get stream update sender for external components
    pub fn get_stream_update_sender(&self) -> mpsc::UnboundedSender<(String, StreamUpdate)> {
        self.stream_update_tx.clone()
    }

    /// Enable or disable ligatures
    pub fn set_ligatures_enabled(&mut self, enabled: bool) {
        self.ligatures_enabled = enabled;
    }

    /// Get ligature status
    pub fn get_ligatures_enabled(&self) -> bool {
        self.ligatures_enabled
    }

    /// Render text with advanced font features using cosmic-text
    pub fn render_text_cosmic(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        attrs: Attrs,
    ) -> Result<(), RendererError> {
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(self.font_size, self.font_size * 1.2),
        );

        buffer.set_text(&mut self.font_system, text, attrs, Shaping::Advanced);

        // Shape and layout the text
        buffer.shape_until_scroll(&mut self.font_system);

        // Render glyphs to texture atlas
        // This is a simplified implementation - in practice, would need to integrate with wgpu texture
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                // Update font texture with glyph data
                // For now, just placeholder
            }
        }

        Ok(())
    }

    /// Get font system for advanced operations
    pub fn get_font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// Render markdown content to the terminal grid
    pub fn render_markdown(&mut self, content: &str) -> Result<(), RendererError> {
        if let Some(ref mut markdown_renderer) = self.markdown_renderer {
            let mut grid = self.grid.write();
            markdown_renderer.render_to_grid(content, &mut grid)?;
        }
        Ok(())
    }

    /// Render streaming markdown content
    pub fn render_markdown_streaming(&mut self, chunk: &str) -> Result<(), RendererError> {
        if let Some(ref mut markdown_renderer) = self.markdown_renderer {
            let mut grid = self.grid.write();
            markdown_renderer.render_streaming(chunk, &mut grid)?;
        }
        Ok(())
    }

    /// Scroll markdown content
    pub fn scroll_markdown(&mut self, delta: i32) -> Result<(), RendererError> {
        if let Some(ref mut markdown_renderer) = self.markdown_renderer {
            markdown_renderer.scroll(delta);
        }
        Ok(())
    }

    /// Update markdown renderer context (for theme changes, terminal resize, etc.)
    pub fn update_markdown_context(&mut self, context: RenderContext) -> Result<(), RendererError> {
        if let Some(ref mut markdown_renderer) = self.markdown_renderer {
            markdown_renderer.update_context(context)?;
        }
        Ok(())
    }

    /// Get markdown renderer performance stats
    pub fn get_markdown_performance_stats(&self) -> Option<&crate::markdown_renderer::PerformanceStats> {
        self.markdown_renderer.as_ref().map(|r| r.get_performance_stats())
    }

    /// Enable/disable markdown renderer
    pub fn set_markdown_enabled(&mut self, enabled: bool) -> Result<(), RendererError> {
        if enabled && self.markdown_renderer.is_none() {
            // Initialize markdown renderer
            let grid = self.grid.read();
            let grid_width = grid.width as usize;
            let grid_height = grid.height as usize;
            drop(grid);

            let markdown_context = RenderContext {
                terminal_width: grid_width,
                supports_truecolor: true,
                supports_256color: true,
                supports_unicode: true,
                tab_width: 4,
                code_theme: "base16-ocean.dark".to_string(),
                wrap_code: true,
                show_line_numbers: false,
            };
            self.markdown_renderer = MarkdownTerminalRenderer::new(markdown_context, grid_width, grid_height).ok();
        } else if !enabled {
            self.markdown_renderer = None;
        }
        Ok(())
    }

    /// Check if markdown rendering is enabled
    pub fn is_markdown_enabled(&self) -> bool {
        self.markdown_renderer.is_some()
    }
    
    // Enhanced rendering helper methods
    fn add_background_quad(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        vertex_offset: &mut u32,
        x: u32,
        y: u32,
        cell: &TerminalCell,
    ) {
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;
        
        // Convert screen coordinates to normalized device coordinates
        let ndc_x = (x_pos / self.config.width as f32) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y_pos / self.config.height as f32) * 2.0;
        let ndc_w = (self.cell_width / self.config.width as f32) * 2.0;
        let ndc_h = (self.cell_height / self.config.height as f32) * 2.0;

        // Add quad vertices
        vertices.extend_from_slice(&[
            Vertex {
                position: [ndc_x, ndc_y],
                tex_coords: [0.0, 0.0],
                color: cell.background,
            },
            Vertex {
                position: [ndc_x + ndc_w, ndc_y],
                tex_coords: [1.0, 0.0],
                color: cell.background,
            },
            Vertex {
                position: [ndc_x, ndc_y - ndc_h],
                tex_coords: [0.0, 1.0],
                color: cell.background,
            },
            Vertex {
                position: [ndc_x + ndc_w, ndc_y - ndc_h],
                tex_coords: [1.0, 1.0],
                color: cell.background,
            },
        ]);

        // Add quad indices
        let base = *vertex_offset;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        *vertex_offset += 4;
    }
    
    fn add_text_quad(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        vertex_offset: &mut u32,
        x: u32,
        y: u32,
        cell: &TerminalCell,
    ) -> Result<(), RendererError> {
        // Get glyph from atlas (simplified for now)
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;
        
        // Convert screen coordinates to normalized device coordinates
        let ndc_x = (x_pos / self.config.width as f32) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y_pos / self.config.height as f32) * 2.0;
        let ndc_w = (self.cell_width / self.config.width as f32) * 2.0;
        let ndc_h = (self.cell_height / self.config.height as f32) * 2.0;
        
        // Use placeholder texture coordinates (would be from glyph atlas)
        let tex_x = 0.0;
        let tex_y = 0.0;
        let tex_w = 1.0;
        let tex_h = 1.0;

        vertices.extend_from_slice(&[
            Vertex {
                position: [ndc_x, ndc_y],
                tex_coords: [tex_x, tex_y],
                color: cell.foreground,
            },
            Vertex {
                position: [ndc_x + ndc_w, ndc_y],
                tex_coords: [tex_x + tex_w, tex_y],
                color: cell.foreground,
            },
            Vertex {
                position: [ndc_x, ndc_y - ndc_h],
                tex_coords: [tex_x, tex_y + tex_h],
                color: cell.foreground,
            },
            Vertex {
                position: [ndc_x + ndc_w, ndc_y - ndc_h],
                tex_coords: [tex_x + tex_w, tex_y + tex_h],
                color: cell.foreground,
            },
        ]);

        let base = *vertex_offset;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        *vertex_offset += 4;
        
        Ok(())
    }
    
    /// Enhanced cursor rendering with different styles
    fn render_cursor<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) -> Result<(), RendererError> {
        let grid = self.grid.read();
        let cursor_x = grid.cursor_x;
        let cursor_y = grid.cursor_y;
        
        let x_pos = cursor_x as f32 * self.cell_width;
        let y_pos = cursor_y as f32 * self.cell_height;
        
        let ndc_x = (x_pos / self.config.width as f32) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y_pos / self.config.height as f32) * 2.0;
        let ndc_w = (self.cell_width / self.config.width as f32) * 2.0;
        let ndc_h = (self.cell_height / self.config.height as f32) * 2.0;
        
        let cursor_color = [1.0, 1.0, 1.0, 1.0]; // White cursor

        let vertices = match self.cursor_style {
            CursorStyle::Block => vec![
                Vertex { position: [ndc_x, ndc_y], tex_coords: [0.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w, ndc_y], tex_coords: [1.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x, ndc_y - ndc_h], tex_coords: [0.0, 1.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w, ndc_y - ndc_h], tex_coords: [1.0, 1.0], color: cursor_color },
            ],
            CursorStyle::Beam => vec![
                Vertex { position: [ndc_x, ndc_y], tex_coords: [0.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w * 0.1, ndc_y], tex_coords: [1.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x, ndc_y - ndc_h], tex_coords: [0.0, 1.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w * 0.1, ndc_y - ndc_h], tex_coords: [1.0, 1.0], color: cursor_color },
            ],
            CursorStyle::Underline => vec![
                Vertex { position: [ndc_x, ndc_y - ndc_h * 0.9], tex_coords: [0.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w, ndc_y - ndc_h * 0.9], tex_coords: [1.0, 0.0], color: cursor_color },
                Vertex { position: [ndc_x, ndc_y - ndc_h], tex_coords: [0.0, 1.0], color: cursor_color },
                Vertex { position: [ndc_x + ndc_w, ndc_y - ndc_h], tex_coords: [1.0, 1.0], color: cursor_color },
            ],
        };

        let indices: Vec<u32> = vec![0, 1, 2, 2, 1, 3];

        self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        self.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..6, 0, 0..1);
        
        Ok(())
    }
    
    fn render_selection<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        selection: &SelectionRange,
    ) -> Result<(), RendererError> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut vertex_offset = 0u32;
        
        let selection_color = [0.3, 0.5, 1.0, 0.3]; // Semi-transparent blue
        
        // Render selection as background highlights
        let start_y = selection.start_y.min(selection.end_y);
        let end_y = selection.start_y.max(selection.end_y);
        
        for y in start_y..=end_y {
            let (start_x, end_x) = if y == start_y && y == end_y {
                (selection.start_x.min(selection.end_x), selection.start_x.max(selection.end_x))
            } else if y == start_y {
                (selection.start_x, u32::MAX) // To end of line
            } else if y == end_y {
                (0, selection.end_x)
            } else {
                (0, u32::MAX) // Full line
            };
            
            let grid = self.grid.read();
            let actual_end_x = if end_x == u32::MAX { grid.width } else { end_x.min(grid.width) };
            
            for x in start_x..actual_end_x {
                let x_pos = x as f32 * self.cell_width;
                let y_pos = y as f32 * self.cell_height;
                
                let ndc_x = (x_pos / self.config.width as f32) * 2.0 - 1.0;
                let ndc_y = 1.0 - (y_pos / self.config.height as f32) * 2.0;
                let ndc_w = (self.cell_width / self.config.width as f32) * 2.0;
                let ndc_h = (self.cell_height / self.config.height as f32) * 2.0;
                
                vertices.extend_from_slice(&[
                    Vertex { position: [ndc_x, ndc_y], tex_coords: [0.0, 0.0], color: selection_color },
                    Vertex { position: [ndc_x + ndc_w, ndc_y], tex_coords: [1.0, 0.0], color: selection_color },
                    Vertex { position: [ndc_x, ndc_y - ndc_h], tex_coords: [0.0, 1.0], color: selection_color },
                    Vertex { position: [ndc_x + ndc_w, ndc_y - ndc_h], tex_coords: [1.0, 1.0], color: selection_color },
                ]);
                
                let base = vertex_offset;
                indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
                vertex_offset += 4;
            }
        }
        
        if !vertices.is_empty() {
            self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }
        
        Ok(())
    }
    
    /// Set cursor style
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }
    
    /// Set selection range
    pub fn set_selection(&mut self, selection: Option<SelectionRange>) {
        self.selection = selection;
    }
    
    /// Get performance metrics
    pub fn get_performance_metrics(&self) -> &PerformanceMetrics {
        &self.performance_metrics
    }
    
    /// Set target FPS
    pub fn set_target_fps(&mut self, fps: u32) {
        self.target_fps = fps.clamp(30, 240); // Reasonable bounds
    }
}
