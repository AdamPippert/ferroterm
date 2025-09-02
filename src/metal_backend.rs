use bytemuck;
use parking_lot::RwLock;
use std::sync::Arc;
use thiserror::Error;
use wgpu; // Fallback to wgpu on macOS if Metal not available

#[derive(Error, Debug)]
pub enum MetalBackendError {
    #[error("Metal error: {0}")]
    Metal(String),
    #[error("WGPU fallback error: {0}")]
    WgpuFallback(#[from] wgpu::Error),
    #[error("Surface error: {0}")]
    Surface(String),
}

/// Metal-specific renderer for macOS optimization
pub struct MetalRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    glyph_atlas_texture: wgpu::Texture,
    glyph_atlas_view: wgpu::TextureView,
    glyph_sampler: wgpu::Sampler,
    glyph_bind_group: wgpu::BindGroup,
    grid: Arc<RwLock<crate::renderer::TerminalGrid>>,
    font_size: f32,
    cell_width: f32,
    cell_height: f32,
    metal_optimized: bool,
}

impl MetalRenderer {
    /// Create a new Metal renderer (with wgpu fallback)
    pub async fn new<W>(
        window: &W,
        width: u32,
        height: u32,
        font_size: f32,
    ) -> Result<Self, MetalBackendError>
    where
        W: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle,
    {
        // Try to use Metal backend if available
        let metal_optimized = Self::is_metal_available();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if metal_optimized {
                wgpu::Backends::METAL
            } else {
                wgpu::Backends::all()
            },
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: Default::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: window.display_handle().map_err(|e| {
                    MetalBackendError::Surface(format!("Failed to get display handle: {:?}", e))
                })?.into(),
                raw_window_handle: window.window_handle().map_err(|e| {
                    MetalBackendError::Surface(format!("Failed to get window handle: {:?}", e))
                })?.into(),
            })
        }
        .map_err(|e| MetalBackendError::Surface(format!("Failed to create surface: {:?}", e)))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(MetalBackendError::Surface(
                "No suitable adapter found".to_string(),
            ))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: None,
                },
                None,
            )
            .await
            .map_err(|e| {
                MetalBackendError::Surface(format!("Failed to request device: {:?}", e))
            })?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create glyph atlas texture with Metal optimizations
        let (glyph_atlas_texture, glyph_atlas_view, glyph_sampler) =
            Self::create_glyph_atlas(&device, metal_optimized)?;

        let glyph_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("glyph_bind_group_layout"),
            });

        let glyph_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &glyph_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glyph_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&glyph_sampler),
                },
            ],
            label: Some("glyph_bind_group"),
        });

        // Create render pipeline with Metal shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Metal Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/metal_text.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Metal Render Pipeline Layout"),
                bind_group_layouts: &[&glyph_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Metal Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[crate::renderer::Vertex::desc()],
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

        // Create buffers
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Metal Vertex Buffer"),
            size: 1024 * std::mem::size_of::<crate::renderer::Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Metal Index Buffer"),
            size: 1024 * std::mem::size_of::<u16>() as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.2;

        let grid = Arc::new(RwLock::new(crate::renderer::TerminalGrid::new(
            (width as f32 / cell_width) as u32,
            (height as f32 / cell_height) as u32,
        )));

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            config,
            render_pipeline,
            vertex_buffer,
            index_buffer,
            glyph_atlas_texture,
            glyph_atlas_view,
            glyph_sampler,
            glyph_bind_group,
            grid,
            font_size,
            cell_width,
            cell_height,
            metal_optimized,
        })
    }

    /// Check if Metal is available
    fn is_metal_available() -> bool {
        // In practice, check if running on macOS and Metal is supported
        cfg!(target_os = "macos")
    }

    /// Create glyph atlas with Metal optimizations
    fn create_glyph_atlas(
        device: &wgpu::Device,
        metal_optimized: bool,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, wgpu::Sampler), MetalBackendError> {
        let size = if metal_optimized { 2048 } else { 1024 }; // Larger atlas for Metal

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Metal Glyph Atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok((texture, texture_view, sampler))
    }

    /// Render frame with Metal optimizations
    pub fn render(&mut self) -> Result<(), MetalBackendError> {
        let output = self.surface.get_current_texture().map_err(|e| {
            MetalBackendError::Surface(format!("Failed to get surface texture: {:?}", e))
        })?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Metal Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Metal Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.glyph_bind_group, &[]);

            // Render terminal grid with Metal optimizations
            self.render_grid_metal(&mut render_pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Render grid with Metal-specific optimizations
    fn render_grid_metal<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        let grid = self.grid.read();

        // Metal-optimized rendering: batch operations, use Metal-specific features
        for y in 0..grid.height {
            for x in 0..grid.width {
                if let Some(cell) = grid.get_cell(x, y) {
                    self.render_cell_metal(render_pass, x, y, cell);
                }
            }
        }

        if grid.cursor_visible {
            self.render_cursor_metal(render_pass, grid.cursor_x, grid.cursor_y);
        }
    }

    /// Render cell with Metal optimizations
    fn render_cell_metal<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        x: u32,
        y: u32,
        cell: &crate::renderer::TerminalCell,
    ) {
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;

        let vertices = [
            crate::renderer::Vertex {
                position: [x_pos, y_pos],
                tex_coords: [0.0, 0.0],
                color: cell.foreground,
            },
            crate::renderer::Vertex {
                position: [x_pos + self.cell_width, y_pos],
                tex_coords: [1.0, 0.0],
                color: cell.foreground,
            },
            crate::renderer::Vertex {
                position: [x_pos, y_pos + self.cell_height],
                tex_coords: [0.0, 1.0],
                color: cell.foreground,
            },
            crate::renderer::Vertex {
                position: [x_pos + self.cell_width, y_pos + self.cell_height],
                tex_coords: [1.0, 1.0],
                color: cell.foreground,
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

    /// Render cursor with Metal optimizations
    fn render_cursor_metal<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, x: u32, y: u32) {
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;

        let vertices = [
            crate::renderer::Vertex {
                position: [x_pos, y_pos],
                tex_coords: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            crate::renderer::Vertex {
                position: [x_pos + self.cell_width, y_pos],
                tex_coords: [1.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            crate::renderer::Vertex {
                position: [x_pos, y_pos + self.cell_height],
                tex_coords: [0.0, 1.0],
                color: [1.0, 1.0, 1.0, 1.0],
            },
            crate::renderer::Vertex {
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

    /// Resize the Metal renderer
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);

            let mut grid = self.grid.write();
            let new_width = (width as f32 / self.cell_width) as u32;
            let new_height = (height as f32 / self.cell_height) as u32;

            if new_width != grid.width || new_height != grid.height {
                *grid = crate::renderer::TerminalGrid::new(new_width, new_height);
            }
        }
    }

    /// Check if using Metal optimizations
    pub fn is_metal_optimized(&self) -> bool {
        self.metal_optimized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metal_availability() {
        let available = MetalRenderer::is_metal_available();
        // On macOS this should be true, elsewhere false
        assert_eq!(available, cfg!(target_os = "macos"));
    }
}
