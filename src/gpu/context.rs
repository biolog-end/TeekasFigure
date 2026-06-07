// GPU context: device, queue, textures, buffers, pipelines, and dispatch/readback functions

use crate::error::AppError;
use crate::settings::Settings;
use crate::types::{CandidateParams, EvalUniforms, ShapeLayer};

use std::sync::Arc;

use super::pipelines::{CompositePipeline, MsePipeline};

/// Holds all GPU resources needed for the image approximation pipeline.
pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub canvas: wgpu::Texture,
    pub canvas_view: wgpu::TextureView,
    pub target: wgpu::Texture,
    pub target_view: wgpu::TextureView,
    pub shape_array: wgpu::Texture,
    pub shape_array_view: wgpu::TextureView,
    pub candidate_buffer: wgpu::Buffer,
    pub fitness_buffer: wgpu::Buffer,
    pub fitness_staging: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
    pub canvas_size: (u32, u32),
    pub batch_size: u32,
    pub num_shapes: u32,
    pub shape_resolution: u32,
    // Pipeline resources
    pub mse_pipeline: MsePipeline,
    pub composite_pipeline: CompositePipeline,
    pub mse_bind_group: wgpu::BindGroup,
    pub composite_sampler: wgpu::Sampler,
    pub composite_uniform_buffer: wgpu::Buffer,
    pub surface_format: wgpu::TextureFormat,
    pub blit_pipeline: wgpu::RenderPipeline,
    pub blit_bind_group_layout: wgpu::BindGroupLayout,
    pub blit_sampler: wgpu::Sampler,
}

impl GpuContext {
    /// Initialize the GPU context with all required resources.
    ///
    /// Creates the WGPU device and queue, then allocates textures and buffers
    /// for the canvas, target image, shape array, candidates, fitness scores,
    /// and uniforms. Also creates pipelines and bind groups.
    ///
    /// # Arguments
    /// * `target_data` - RGBA8 pixel data of the target image
    /// * `target_size` - (width, height) of the target image in pixels
    /// * `shapes` - Preprocessed shape layers to upload as a 2D texture array
    /// * `settings` - Application settings controlling batch size and shape resolution
    ///
    /// # Errors
    /// Returns `AppError::GpuInit` if adapter/device request fails or allocation fails.
    pub fn new(
        target_data: &[u8],
        target_size: (u32, u32),
        shapes: &[ShapeLayer],
        settings: &Settings,
    ) -> Result<Self, AppError> {
        let (device, queue) = pollster::block_on(Self::init_device())?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let (width, height) = target_size;
        let num_shapes = shapes.len() as u32;
        let shape_resolution = settings.shape_resolution;
        let batch_size = settings.batch_size;

        // Canvas texture: the evolving approximation, stays GPU-resident
        let canvas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Canvas Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let canvas_view = canvas.create_view(&wgpu::TextureViewDescriptor::default());

        // Target texture: the reference image we're approximating
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Target Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Upload target image data
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            target_data,
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

        // Shape array texture: 2D texture array with one layer per shape
        let shape_array = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shape Array Texture"),
            size: wgpu::Extent3d {
                width: shape_resolution,
                height: shape_resolution,
                depth_or_array_layers: num_shapes,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let shape_array_view = shape_array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // Upload each shape layer into the texture array
        for (i, layer) in shapes.iter().enumerate() {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &shape_array,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &layer.pixels,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * shape_resolution),
                    rows_per_image: Some(shape_resolution),
                },
                wgpu::Extent3d {
                    width: shape_resolution,
                    height: shape_resolution,
                    depth_or_array_layers: 1,
                },
            );
        }

        // Candidate buffer: storage buffer holding candidate parameters (48 bytes each)
        let candidate_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Candidate Buffer"),
            size: (48 * batch_size) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Fitness buffer: storage buffer for MSE results (one f32 per candidate)
        let fitness_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fitness Buffer"),
            size: (4 * batch_size) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Fitness staging buffer: for reading fitness scores back to CPU
        let fitness_staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fitness Staging Buffer"),
            size: (4 * batch_size) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Uniform buffer: holds EvalUniforms for the compute shader
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: std::mem::size_of::<EvalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create pipelines
        let mse_pipeline = MsePipeline::new(&device);
        let composite_pipeline = CompositePipeline::new(&device);

        // Create the MSE bind group (references all textures and buffers)
        let mse_bind_group =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MSE Eval Bind Group"),
                layout: &mse_pipeline.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&canvas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&target_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&shape_array_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: candidate_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: fitness_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            });

        // Create sampler for composite pipeline (bilinear filtering)
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Composite Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Composite uniform buffer: holds a single CandidateParams for compositing
        let composite_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Composite Uniform Buffer"),
            size: std::mem::size_of::<CandidateParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create blit resources
        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blit BGL"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Blit PL"), bind_group_layouts: &[&blit_bind_group_layout], push_constant_ranges: &[] });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit RP"), layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState { module: &blit_shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState { module: &blit_shader, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: wgpu::PipelineCompilationOptions::default() }),
            multiview: None, cache: None,
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("Blit Sampler"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });

        Ok(Self {
            device,
            queue,
            canvas,
            canvas_view,
            target,
            target_view,
            shape_array,
            shape_array_view,
            candidate_buffer,
            fitness_buffer,
            fitness_staging,
            uniform_buffer,
            canvas_size: target_size,
            batch_size,
            num_shapes,
            shape_resolution,
            mse_pipeline,
            composite_pipeline,
            mse_bind_group,
            composite_sampler,
            composite_uniform_buffer,
            surface_format: wgpu::TextureFormat::Rgba8Unorm,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
        })
    }

    /// Write candidate params to GPU buffer, update uniforms, dispatch compute shader.
    ///
    /// Submits a compute pass that evaluates MSE for all candidates in parallel.
    /// Each candidate gets one workgroup of 256 threads.
    pub fn dispatch_mse_evaluation(&self, candidates: &[CandidateParams]) {
        let num_candidates = candidates.len() as u32;
        if num_candidates == 0 {
            return;
        }

        // Write candidate data to the GPU buffer
        self.queue.write_buffer(
            &self.candidate_buffer,
            0,
            bytemuck::cast_slice(candidates),
        );

        // Write EvalUniforms to the uniform buffer
        let uniforms = EvalUniforms {
            canvas_width: self.canvas_size.0,
            canvas_height: self.canvas_size.1,
            num_candidates,
            shape_resolution: self.shape_resolution,
            displacement_weight: 0.0, // Set by caller if needed for video mode
        };
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&uniforms),
        );

        // Create command encoder and dispatch compute pass
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("MSE Dispatch Encoder"),
        });

        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("MSE Compute Pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.mse_pipeline.pipeline);
            compute_pass.set_bind_group(0, &self.mse_bind_group, &[]);
            // Dispatch one workgroup per candidate
            compute_pass.dispatch_workgroups(num_candidates, 1, 1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Read fitness scores back from GPU to CPU.
    ///
    /// Copies the fitness buffer to a staging buffer, maps it, and reads the f32 array.
    /// Uses `pollster::block_on` for the async buffer mapping.
    pub fn read_fitness_scores(&self) -> Vec<f32> {
        let buffer_size = (4 * self.batch_size) as u64;

        // Create encoder to copy fitness buffer to staging
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Fitness Readback Encoder"),
        });
        encoder.copy_buffer_to_buffer(&self.fitness_buffer, 0, &self.fitness_staging, 0, buffer_size);
        self.queue.submit(std::iter::once(encoder.finish()));

        // Map the staging buffer for reading
        let buffer_slice = self.fitness_staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });

        // Poll the device until the mapping completes
        self.device.poll(wgpu::Maintain::Wait);
        receiver.recv().unwrap().unwrap();

        // Read the mapped data
        let data = buffer_slice.get_mapped_range();
        let scores: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);

        // Unmap the staging buffer so it can be reused
        self.fitness_staging.unmap();

        scores
    }

    /// Composite a single winning shape onto the canvas using the render pipeline.
    ///
    /// Writes the candidate params to the composite uniform buffer, creates a bind group,
    /// and executes a render pass that draws a fullscreen quad with alpha blending.
    pub fn composite_shape(&self, candidate: &CandidateParams) {
        // Write candidate params to the composite uniform buffer
        self.queue.write_buffer(
            &self.composite_uniform_buffer,
            0,
            bytemuck::bytes_of(candidate),
        );

        // Create bind group for the composite pipeline
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite Bind Group"),
            layout: &self.composite_pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.shape_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.composite_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.composite_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Create encoder and begin render pass targeting the canvas
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Composite Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Composite Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.canvas_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Load existing canvas content (we're blending on top)
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(&self.composite_pipeline.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            // Draw 6 vertices (fullscreen quad: 2 triangles)
            render_pass.draw(0..6, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Clear the canvas to black.
    pub fn clear_canvas(&self) {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Canvas Clear"),
        });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.canvas_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Blit the canvas texture to the window surface via a render pass.
    ///
    /// Uses a fullscreen quad shader to copy the canvas content to the surface,
    /// handling format conversion (e.g., Rgba8Unorm → Bgra8Unorm) automatically.
    pub fn blit_canvas_to_surface(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
    ) {
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blit Bind Group"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.canvas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Blit Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(&self.blit_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }

    /// Create a surface configuration for presenting the canvas to a window.
    ///
    /// Uses the provided format (should come from surface capabilities).
    /// Includes `COPY_DST` usage to allow `copy_texture_to_texture` from canvas.
    pub fn create_surface_config(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> wgpu::SurfaceConfiguration {
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        }
    }

    /// Initialize the GPU context with an externally-created instance and surface.
    ///
    /// This variant requests an adapter compatible with the given surface, ensuring
    /// that the device can present to the window.
    ///
    /// # Arguments
    /// * `instance` - The wgpu instance (created externally for surface compatibility)
    /// * `surface` - The window surface to ensure adapter compatibility
    /// * `target_data` - RGBA8 pixel data of the target image
    /// * `target_size` - (width, height) of the target image in pixels
    /// * `shapes` - Preprocessed shape layers to upload as a 2D texture array
    /// * `settings` - Application settings controlling batch size and shape resolution
    pub fn new_with_surface(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        target_data: &[u8],
        target_size: (u32, u32),
        shapes: &[ShapeLayer],
        settings: &Settings,
    ) -> Result<Self, AppError> {
        let (device, queue, surface_format) =
            pollster::block_on(Self::init_device_with_surface(instance, surface))?;
        Self::new_from_device(
            Arc::new(device),
            Arc::new(queue),
            surface_format,
            target_data,
            target_size,
            shapes,
            settings,
        )
    }

    /// Build all image-specific GPU resources on top of an already-created
    /// device/queue (typically shared with the egui renderer).
    ///
    /// This allows the window, surface and egui to be created up-front for the
    /// Settings screen, deferring the heavy per-media GPU allocation until the
    /// user presses "Start". `Arc<wgpu::Device>`/`Arc<wgpu::Queue>` are cheap to
    /// clone, so the same device keeps backing the egui renderer.
    pub fn new_from_device(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        surface_format: wgpu::TextureFormat,
        target_data: &[u8],
        target_size: (u32, u32),
        shapes: &[ShapeLayer],
        settings: &Settings,
    ) -> Result<Self, AppError> {
        let (width, height) = target_size;
        let num_shapes = shapes.len() as u32;
        let shape_resolution = settings.shape_resolution;
        let batch_size = settings.batch_size;

        // Canvas texture
        let canvas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Canvas Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let canvas_view = canvas.create_view(&wgpu::TextureViewDescriptor::default());

        // Clear canvas to black so it starts in a known state
        {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Canvas Clear Encoder"),
            });
            {
                let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Canvas Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &canvas_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.1, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit(std::iter::once(encoder.finish()));
        }

        // Target texture
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Target Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            target_data,
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

        // Shape array texture
        let shape_array = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shape Array Texture"),
            size: wgpu::Extent3d {
                width: shape_resolution,
                height: shape_resolution,
                depth_or_array_layers: num_shapes,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let shape_array_view = shape_array.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        for (i, layer) in shapes.iter().enumerate() {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &shape_array,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &layer.pixels,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * shape_resolution),
                    rows_per_image: Some(shape_resolution),
                },
                wgpu::Extent3d {
                    width: shape_resolution,
                    height: shape_resolution,
                    depth_or_array_layers: 1,
                },
            );
        }

        // Buffers
        let candidate_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Candidate Buffer"),
            size: (48 * batch_size) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fitness_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fitness Buffer"),
            size: (4 * batch_size) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let fitness_staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fitness Staging Buffer"),
            size: (4 * batch_size) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: std::mem::size_of::<EvalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Pipelines
        let mse_pipeline = MsePipeline::new(&device);
        let composite_pipeline = CompositePipeline::new(&device);

        let mse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MSE Eval Bind Group"),
            layout: &mse_pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&canvas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&target_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&shape_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: candidate_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: fitness_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Composite Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let composite_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Composite Uniform Buffer"),
            size: std::mem::size_of::<CandidateParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Blit pipeline: renders canvas texture to surface via fullscreen quad
        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blit Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blit Pipeline Layout"),
            bind_group_layouts: &[&blit_bind_group_layout],
            push_constant_ranges: &[],
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit Render Pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blit Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            device,
            queue,
            canvas,
            canvas_view,
            target,
            target_view,
            shape_array,
            shape_array_view,
            candidate_buffer,
            fitness_buffer,
            fitness_staging,
            uniform_buffer,
            canvas_size: target_size,
            batch_size,
            num_shapes,
            shape_resolution,
            mse_pipeline,
            composite_pipeline,
            mse_bind_group,
            composite_sampler,
            composite_uniform_buffer,
            surface_format,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
        })
    }

    /// Request a WGPU adapter and device with default limits.
    async fn init_device() -> Result<(wgpu::Device, wgpu::Queue), AppError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| {
                AppError::GpuInit(
                    "Failed to find a compatible GPU adapter. \
                     Ensure a Vulkan, DX12, or Metal capable GPU is available."
                        .to_string(),
                )
            })?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("GPU Image Approximator Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| {
                AppError::GpuInit(format!(
                    "Failed to create GPU device: {}. \
                     Try reducing batch_size or max_texture_size in settings.toml.",
                    e
                ))
            })?;

        Ok((device, queue))
    }

    /// Request a WGPU adapter compatible with the given surface, then create a device.
    /// Explicitly enumerates adapters and prefers discrete GPUs to avoid using integrated graphics.
    pub async fn init_device_with_surface(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<(wgpu::Device, wgpu::Queue, wgpu::TextureFormat), AppError> {
        // Enumerate all adapters and prefer discrete GPU
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        let mut discrete_adapter: Option<wgpu::Adapter> = None;
        let mut any_compatible: Option<wgpu::Adapter> = None;

        for adapter in adapters {
            let info = adapter.get_info();
            log::info!(
                "Found GPU adapter: '{}' (type: {:?}, backend: {:?})",
                info.name,
                info.device_type,
                info.backend
            );

            // Check surface compatibility
            if !adapter.is_surface_supported(surface) {
                log::info!("  -> Not compatible with surface, skipping");
                continue;
            }

            if info.device_type == wgpu::DeviceType::DiscreteGpu {
                if discrete_adapter.is_none() {
                    log::info!("  -> Selected as discrete GPU");
                    discrete_adapter = Some(adapter);
                }
            } else if any_compatible.is_none() {
                any_compatible = Some(adapter);
            }
        }

        let adapter = discrete_adapter
            .or(any_compatible)
            .ok_or_else(|| {
                AppError::GpuInit(
                    "Failed to find a compatible GPU adapter. \
                     Ensure a Vulkan, DX12, or Metal capable GPU is available."
                        .to_string(),
                )
            })?;

        let info = adapter.get_info();
        log::info!("Using GPU: '{}' ({:?})", info.name, info.device_type);

        // Query surface format
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);
        log::info!("Surface format: {:?}", surface_format);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("GPU Image Approximator Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| {
                AppError::GpuInit(format!(
                    "Failed to create GPU device: {}. \
                     Try reducing batch_size or max_texture_size in settings.toml.",
                    e
                ))
            })?;

        Ok((device, queue, surface_format))
    }
}
