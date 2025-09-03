use crate::terminal::{TerminalState, TerminalCell};
use std::sync::Arc;
use parking_lot::RwLock;
use wgpu;
use winit::window::Window;
use bytemuck::{Pod, Zeroable};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RendererError {
    #[error("WGPU error: {0}")]
    Wgpu(String),
    #[error("Surface error: {0}")]
    Surface(String),
    #[error("Shader error: {0}")]
    Shader(String),
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
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

pub struct SimpleRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    terminal_state: Arc<RwLock<TerminalState>>,
    cell_width: f32,
    cell_height: f32,
}

impl SimpleRenderer {
    pub async fn new(
        window: Arc<Window>, 
        terminal_state: Arc<RwLock<TerminalState>>
    ) -> Result<Self, RendererError> {
        let size = window.inner_size();
        
        // Create WGPU instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: Default::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });

        // Create surface
        let surface = instance.create_surface(window.clone())
            .map_err(|e| RendererError::Surface(format!("Failed to create surface: {:?}", e)))?;

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(RendererError::Surface("No suitable adapter found".to_string()))?;

        // Request device
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
            .map_err(|e| RendererError::Surface(format!("Failed to request device: {:?}", e)))?;

        // Configure surface
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
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create shader
        let shader_source = r#"
            struct VertexInput {
                @location(0) position: vec2<f32>,
                @location(1) tex_coords: vec2<f32>,
                @location(2) color: vec4<f32>,
            }

            struct VertexOutput {
                @builtin(position) clip_position: vec4<f32>,
                @location(0) color: vec4<f32>,
            }

            @vertex
            fn vs_main(model: VertexInput) -> VertexOutput {
                var out: VertexOutput;
                out.color = model.color;
                out.clip_position = vec4<f32>(model.position, 0.0, 1.0);
                return out;
            }

            @fragment
            fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
                return in.color;
            }
        "#;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Simple Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[],
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
                    blend: Some(wgpu::BlendState::REPLACE),
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

        // Create vertex buffer
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: 1024 * 1024, // 1MB buffer
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create index buffer
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Index Buffer"),
            size: 512 * 1024, // 512KB buffer
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Calculate cell dimensions
        let terminal = terminal_state.read();
        let cell_width = size.width as f32 / terminal.width as f32;
        let cell_height = size.height as f32 / terminal.height as f32;
        drop(terminal);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            render_pipeline,
            vertex_buffer,
            index_buffer,
            terminal_state,
            cell_width,
            cell_height,
        })
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            
            // Update cell dimensions
            let terminal = self.terminal_state.read();
            self.cell_width = new_size.width as f32 / terminal.width as f32;
            self.cell_height = new_size.height as f32 / terminal.height as f32;
        }
    }

    pub fn render(&mut self) -> Result<(), RendererError> {
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err(RendererError::Surface("Out of memory".to_string()));
            }
            Err(e) => {
                return Err(RendererError::Surface(format!("Surface error: {:?}", e)));
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        // Build vertex and index data
        let (vertices, indices) = self.build_render_data();

        // Update buffers
        if !vertices.is_empty() {
            self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
        if !indices.is_empty() {
            self.queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

            if !indices.is_empty() {
                render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    fn build_render_data(&self) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut vertex_index = 0u32;

        let terminal = self.terminal_state.read();

        // Render terminal cells
        for y in 0..terminal.height {
            for x in 0..terminal.width {
                if let Some(cell) = terminal.get_cell(x, y) {
                    // Only render non-empty cells or cells with non-default background
                    if cell.character != ' ' || cell.background != [0.0, 0.0, 0.0, 1.0] {
                        self.add_cell_quad(&mut vertices, &mut indices, &mut vertex_index, x, y, cell);
                    }
                }
            }
        }

        // Render cursor
        if terminal.cursor_visible {
            self.add_cursor_quad(&mut vertices, &mut indices, &mut vertex_index, terminal.cursor_x, terminal.cursor_y);
        }

        (vertices, indices)
    }

    fn add_cell_quad(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        vertex_index: &mut u32,
        x: u32,
        y: u32,
        cell: &TerminalCell,
    ) {
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;
        let cell_w = self.cell_width;
        let cell_h = self.cell_height;

        // Convert screen coordinates to normalized device coordinates
        let left = (x_pos / self.config.width as f32) * 2.0 - 1.0;
        let right = ((x_pos + cell_w) / self.config.width as f32) * 2.0 - 1.0;
        let top = 1.0 - (y_pos / self.config.height as f32) * 2.0;
        let bottom = 1.0 - ((y_pos + cell_h) / self.config.height as f32) * 2.0;

        // Add background quad if background is not default black
        if cell.background != [0.0, 0.0, 0.0, 1.0] {
            vertices.extend_from_slice(&[
                Vertex {
                    position: [left, top],
                    tex_coords: [0.0, 0.0],
                    color: cell.background,
                },
                Vertex {
                    position: [right, top],
                    tex_coords: [1.0, 0.0],
                    color: cell.background,
                },
                Vertex {
                    position: [right, bottom],
                    tex_coords: [1.0, 1.0],
                    color: cell.background,
                },
                Vertex {
                    position: [left, bottom],
                    tex_coords: [0.0, 1.0],
                    color: cell.background,
                },
            ]);

            indices.extend_from_slice(&[
                *vertex_index, *vertex_index + 1, *vertex_index + 2,
                *vertex_index, *vertex_index + 2, *vertex_index + 3,
            ]);
            *vertex_index += 4;
        }

        // Add character quad (simplified - just render as colored rectangle for now)
        if cell.character != ' ' {
            // For now, just render characters as small rectangles in the center of the cell
            let char_size = 0.8; // 80% of cell size
            let char_offset = (1.0 - char_size) * 0.5;
            
            let char_left = left + (right - left) * char_offset;
            let char_right = left + (right - left) * (1.0 - char_offset);
            let char_top = top - (top - bottom) * char_offset;
            let char_bottom = top - (top - bottom) * (1.0 - char_offset);

            vertices.extend_from_slice(&[
                Vertex {
                    position: [char_left, char_top],
                    tex_coords: [0.0, 0.0],
                    color: cell.foreground,
                },
                Vertex {
                    position: [char_right, char_top],
                    tex_coords: [1.0, 0.0],
                    color: cell.foreground,
                },
                Vertex {
                    position: [char_right, char_bottom],
                    tex_coords: [1.0, 1.0],
                    color: cell.foreground,
                },
                Vertex {
                    position: [char_left, char_bottom],
                    tex_coords: [0.0, 1.0],
                    color: cell.foreground,
                },
            ]);

            indices.extend_from_slice(&[
                *vertex_index, *vertex_index + 1, *vertex_index + 2,
                *vertex_index, *vertex_index + 2, *vertex_index + 3,
            ]);
            *vertex_index += 4;
        }
    }

    fn add_cursor_quad(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        vertex_index: &mut u32,
        x: u32,
        y: u32,
    ) {
        let x_pos = x as f32 * self.cell_width;
        let y_pos = y as f32 * self.cell_height;
        let cell_w = self.cell_width;
        let cell_h = self.cell_height;

        // Convert screen coordinates to normalized device coordinates
        let left = (x_pos / self.config.width as f32) * 2.0 - 1.0;
        let right = ((x_pos + cell_w) / self.config.width as f32) * 2.0 - 1.0;
        let top = 1.0 - (y_pos / self.config.height as f32) * 2.0;
        let bottom = 1.0 - ((y_pos + cell_h) / self.config.height as f32) * 2.0;

        let cursor_color = [1.0, 1.0, 1.0, 0.8]; // Semi-transparent white

        vertices.extend_from_slice(&[
            Vertex {
                position: [left, top],
                tex_coords: [0.0, 0.0],
                color: cursor_color,
            },
            Vertex {
                position: [right, top],
                tex_coords: [1.0, 0.0],
                color: cursor_color,
            },
            Vertex {
                position: [right, bottom],
                tex_coords: [1.0, 1.0],
                color: cursor_color,
            },
            Vertex {
                position: [left, bottom],
                tex_coords: [0.0, 1.0],
                color: cursor_color,
            },
        ]);

        indices.extend_from_slice(&[
            *vertex_index, *vertex_index + 1, *vertex_index + 2,
            *vertex_index, *vertex_index + 2, *vertex_index + 3,
        ]);
        *vertex_index += 4;
    }
}