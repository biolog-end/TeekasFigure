// GPU context: device, queue, textures, buffers, pipelines, and dispatch/readback functions

use crate::error::AppError;
use crate::settings::Settings;
use crate::types::{CandidateParams, EvalUniforms, ShapeLayer};

use std::sync::Arc;

use super::pipelines::{CompositePipeline, MsePipeline};

/// Holds all GPU resources needed for the image approximation pipeline.
pub struct GpuContext {
    pub control: Arc<super::control::WorkControl>,
    active_candidates: std::sync::atomic::AtomicU32,
    evaluation_submission: std::sync::Mutex<Option<wgpu::SubmissionIndex>>,
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
    luma_weight: f32,
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
        Self::new_from_device(
            Arc::new(device),
            Arc::new(queue),
            wgpu::TextureFormat::Bgra8Unorm,
            target_data,
            target_size,
            shapes,
            settings,
        )
    }

    /// Write candidate params to GPU buffer, update uniforms, dispatch compute shader.
    ///
    /// Submits a compute pass that evaluates MSE for all candidates in parallel.
    /// Each candidate gets one workgroup of 256 threads.
    pub fn dispatch_mse_evaluation(&self, candidates: &[CandidateParams]) {
        let num_candidates = candidates.len() as u32;
        assert!(num_candidates <= self.batch_size);
        self.active_candidates
            .store(num_candidates, std::sync::atomic::Ordering::Relaxed);
        if num_candidates == 0 {
            return;
        }

        // Write candidate data to the GPU buffer
        self.queue
            .write_buffer(&self.candidate_buffer, 0, bytemuck::cast_slice(candidates));

        // Write EvalUniforms to the uniform buffer
        let uniforms = EvalUniforms {
            canvas_width: self.canvas_size.0,
            canvas_height: self.canvas_size.1,
            num_candidates,
            shape_resolution: self.shape_resolution,
            displacement_weight: 0.0, // Set by caller if needed for video mode
            luma_weight: self.luma_weight,
            _padding: [0; 2],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Create command encoder and dispatch compute pass
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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

        // Copy scores in the same submission as compute so presentation work
        // cannot slip between evaluation and its readback.
        encoder.copy_buffer_to_buffer(
            &self.fitness_buffer,
            0,
            &self.fitness_staging,
            0,
            u64::from(num_candidates) * 4,
        );
        let submission = self.queue.submit(std::iter::once(encoder.finish()));
        *self.evaluation_submission.lock().unwrap() = Some(submission);
    }

    /// Read fitness scores back from GPU to CPU.
    ///
    /// Maps the scores copied by the evaluation submission and reads the f32 array.
    /// The worker waits for that submission's completion event.
    pub fn read_fitness_scores(&self) -> Vec<f32> {
        let buffer_size = 4 * u64::from(
            self.active_candidates
                .load(std::sync::atomic::Ordering::Relaxed),
        );

        if buffer_size == 0 {
            return Vec::new();
        }
        let submission = self
            .evaluation_submission
            .lock()
            .unwrap()
            .take()
            .expect("Dispatch before reading scores");

        // Map the staging buffer for reading
        let buffer_slice = self.fitness_staging.slice(..buffer_size);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });

        // Only the worker waits, and only for this bounded compute submission.
        // No timer sleeps and no wait for newer presentation submissions.
        self.device
            .poll(wgpu::Maintain::WaitForSubmissionIndex(submission));
        receiver
            .recv()
            .expect("GPU readback disconnected")
            .expect("GPU readback failed");

        // Read the mapped data
        let data = buffer_slice.get_mapped_range();
        let scores: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);

        // Unmap the staging buffer so it can be reused
        self.fitness_staging.unmap();

        scores
    }

    /// Evaluate any population in bounded submissions; only used by the generation worker.
    pub fn evaluate_candidates(&self, candidates: &[CandidateParams]) -> Vec<f32> {
        let mut scores = Vec::with_capacity(candidates.len());
        // Bound pixel work per submission to avoid submitting a massive long-running shader.
        let pixels = u64::from(self.canvas_size.0) * u64::from(self.canvas_size.1);
        let chunk =
            ((256 * 1024 * 1024 / pixels.max(1)) as usize).clamp(1, self.batch_size as usize);
        for part in candidates.chunks(chunk) {
            if !self.control.checkpoint() {
                return Vec::new();
            }
            let start = std::time::Instant::now();
            self.dispatch_mse_evaluation(part);
            scores.extend(self.read_fitness_scores());
            self.control
                .evaluated
                .fetch_add(part.len() as u64, std::sync::atomic::Ordering::Relaxed);
            self.control.evaluation_ns.fetch_add(
                start.elapsed().as_nanos() as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        viewport: [f32; 4],
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
        let [x, y, width, height] = viewport;
        let scale = (width / self.canvas_size.0 as f32).min(height / self.canvas_size.1 as f32);
        let w = (self.canvas_size.0 as f32 * scale).max(1.0);
        let h = (self.canvas_size.1 as f32 * scale).max(1.0);
        render_pass.set_viewport(
            x + (width - w) * 0.5,
            y + (height - h) * 0.5,
            w,
            h,
            0.0,
            1.0,
        );
        render_pass.draw(0..6, 0..1);
    }

    /// Create a surface configuration for presenting the canvas to a window.
    ///
    /// Uses the provided format (should come from surface capabilities).
    /// Includes `COPY_DST` usage to allow `copy_texture_to_texture` from canvas.
    pub fn create_surface_config(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::SurfaceConfiguration {
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
        let (device, queue, surface_format, _, _) =
            pollster::block_on(Self::init_device_with_surface(instance, surface, None))?;
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
        // App preflight accounts for mode-specific GIF/video storage. This constructor
        // validates the common GPU working set without charging inactive GIF settings.
        let resource_settings = Settings {
            save_progress_gif: false,
            ..settings.clone()
        };
        super::safety::validate(
            &resource_settings,
            &device.limits(),
            target_size,
            num_shapes,
            false,
        )?;
        let batch_size = settings
            .batch_size
            .min(4096)
            .min(device.limits().max_compute_workgroups_per_dimension);
        if target_data.len() != width as usize * height as usize * 4
            || shapes.iter().any(|s| {
                s.pixels.len() != shape_resolution as usize * shape_resolution as usize * 4
            })
        {
            return Err(AppError::GpuInit("Invalid texture data length".into()));
        }

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
            size: (std::mem::size_of::<CandidateParams>() as u32 * batch_size) as u64,
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
        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            control: Arc::new(super::control::WorkControl::default()),
            active_candidates: std::sync::atomic::AtomicU32::new(0),
            evaluation_submission: std::sync::Mutex::new(None),
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
            luma_weight: if settings.perceptual_scoring { settings.luma_weight } else { 0.0 },
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

        // Request the adapter's full capabilities (instead of the conservative
        // `Limits::default()`), so that `max_texture_array_layers` reflects what
        // the GPU actually supports. The default caps array layers at 256, which
        // in turn caps the number of shape brushes; real desktop GPUs support
        // 2048, enabling much larger shape sets (e.g. one brush per video frame).
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("GPU Image Approximator Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
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
        requested: Option<&str>,
    ) -> Result<
        (
            wgpu::Device,
            wgpu::Queue,
            wgpu::TextureFormat,
            String,
            Vec<String>,
        ),
        AppError,
    > {
        let preferred = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(surface),
                force_fallback_adapter: false,
            })
            .await
            .map(|a| a.get_info());
        let mut adapters: Vec<_> = instance
            .enumerate_adapters(wgpu::Backends::all())
            .into_iter()
            .filter(|a| a.is_surface_supported(surface))
            .collect();
        let labels: Vec<_> = adapters
            .iter()
            .map(|a| adapter_label(&a.get_info()))
            .collect();
        adapters.sort_by_key(|a| {
            let i = a.get_info();
            let explicit = requested.map(|s| s == adapter_label(&i)).unwrap_or(false);
            let kind = match i.device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Other => 3,
                wgpu::DeviceType::Cpu => 4,
            };
            let preferred_match = preferred
                .as_ref()
                .map(|p| p.name == i.name && p.device == i.device && p.backend == i.backend)
                .unwrap_or(false);
            let backend = match i.backend {
                wgpu::Backend::Vulkan => 0,
                wgpu::Backend::Dx12 => 1,
                wgpu::Backend::Metal => 2,
                _ => 3,
            };
            (!explicit, kind, !preferred_match, backend)
        });
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| AppError::GpuInit("No compatible GPU found".into()))?;
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

        // Request the adapter's full capabilities so `max_texture_array_layers`
        // reflects the real GPU limit (default caps it at 256). This raises the
        // ceiling on how many shape brushes can be uploaded as array layers.
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("GPU Image Approximator Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
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

        Ok((device, queue, surface_format, adapter_label(&info), labels))
    }
}

fn adapter_label(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend)
}

#[cfg(test)]
mod hardware_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    #[ignore = "requires GPU; validates scene export against exact rendered pixels"]
    fn exported_scene_reconstructs_keyframes_and_interpolated_frames() {
        let settings = Settings {
            shape_resolution: 16,
            batch_size: 8,
            use_original_colors: true,
            evolve_opacity: false,
            scale_min: 0.5,
            scale_max: 1.0,
            export_scene: true,
            ..Settings::default()
        };
        let layers = [
            ShapeLayer {
                pixels: [90, 200, 160, 255].repeat(256),
            },
            ShapeLayer {
                pixels: [200, 70, 130, 180].repeat(256),
            },
        ];
        let gpu = GpuContext::new(
            &[128, 128, 128, 255].repeat(32 * 32),
            (32, 32),
            &layers,
            &settings,
        )
        .unwrap();
        let mut generator = crate::algorithm::CandidateGenerator::new(
            settings.clone(),
            [128, 128, 128, 255].repeat(32 * 32),
            (32, 32),
        );
        let candidates = generator.generate_batch(3, 0, (32, 32), 2);
        let mut pipeline = crate::algorithm::VideoPipeline::new();
        pipeline.record_placed_shape(candidates[0]);
        pipeline.record_placed_shape(candidates[1]);
        let tmp = tempfile::tempdir().unwrap();
        let mut writer = crate::io::scene::SceneWriter::new(
            tmp.path(),
            std::path::Path::new("mobs.mp4"),
            (32, 32),
            24.0,
            true,
            &settings,
            &layers,
            &["creeper.png".into(), "rabbit.png".into()],
        )
        .unwrap();
        let mut reference = Vec::new();
        pipeline.rebuild_canvas(&gpu);
        reference.push(crate::io::output::read_canvas_image(&gpu).unwrap());
        writer
            .write_frame(pipeline.shapes.iter().map(|s| (s.id, s.params)))
            .unwrap();
        pipeline.frame_index = 1;
        for s in &mut pipeline.shapes {
            s.prev_params = s.params;
            s.params.x = (s.params.x + 5.0).min(31.0);
            s.params.rotation += 0.1;
        }
        pipeline.record_placed_shape(candidates[2]);
        pipeline.render_interpolated_frame(&gpu, 0.5);
        reference.push(crate::io::output::read_canvas_image(&gpu).unwrap());
        writer
            .write_frame(pipeline.interpolated_shapes(0.5))
            .unwrap();
        pipeline.rebuild_canvas(&gpu);
        reference.push(crate::io::output::read_canvas_image(&gpu).unwrap());
        writer
            .write_frame(pipeline.shapes.iter().map(|s| (s.id, s.params)))
            .unwrap();
        let folder = writer.finish().unwrap();
        for (index, line) in std::fs::read_to_string(folder.join("frames.jsonl"))
            .unwrap()
            .lines()
            .enumerate()
        {
            let frame: crate::io::scene::Frame = serde_json::from_str(line).unwrap();
            gpu.clear_canvas();
            for s in frame.shapes {
                gpu.composite_shape(&CandidateParams {
                    shape_index: s.brush_index,
                    x: s.center_px[0],
                    y: s.center_px[1],
                    rotation: s.rotation_radians,
                    scale: s.size_px[0] / 16.0,
                    scale_y: s.size_px[1] / 16.0,
                    alpha: s.opacity,
                    use_original_color: if s.original_colors { 1.0 } else { 0.0 },
                    r: s.tint_rgb[0],
                    g: s.tint_rgb[1],
                    b: s.tint_rgb[2],
                    hue_shift: s.hue_turns,
                    saturation_scale: s.saturation,
                    brightness_scale: s.brightness,
                    _padding: [0.0; 2],
                });
            }
            assert_eq!(
                crate::io::output::read_canvas_image(&gpu).unwrap(),
                reference[index]
            );
        }
    }

    #[test]
    #[ignore = "requires GPU; creates a settings UI preview in target/"]
    fn render_settings_preview() {
        let temp = tempfile::tempdir().unwrap();
        crate::io::media_loader::ensure_directories(temp.path()).unwrap();
        let icon =
            image::load_from_memory(include_bytes!("../../assets/teekasfigure.png")).unwrap();
        icon.save(temp.path().join("input_media/Мозаика.png"))
            .unwrap();
        icon.save(temp.path().join("raw_shapes/Пейзаж.png"))
            .unwrap();
        for index in 0..5 {
            let img = image::RgbImage::from_fn(144, 96, |x, y| {
                image::Rgb([
                    (x + index * 35) as u8,
                    (y * 2 + index * 15) as u8,
                    (190 - index * 25) as u8,
                ])
            });
            img.save(
                temp.path()
                    .join(format!("raw_shapes/Фото {}.png", index + 1)),
            )
            .unwrap();
        }
        crate::io::shape_conversion::convert_folder(
            &temp.path().join("raw_shapes"),
            &temp.path().join("input_shapes"),
            128,
        )
        .unwrap();
        let settings = Settings {
            batch_size: 5000,
            num_generations: 5_000_000,
            max_texture_size: 1200,
            ..Settings::default()
        };
        let mut screen = crate::ui::SettingsScreen::new(
            temp.path(),
            settings.clone(),
            crate::ui::Language::Russian,
        );
        screen.active_gpu = "GPU selected by WGPU · Vulkan / DX12".into();
        let gpu = GpuContext::new(
            &vec![0; 1000 * 1000 * 4],
            (1000, 1000),
            &[ShapeLayer {
                pixels: vec![255; 128 * 128 * 4],
            }],
            &settings,
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut renderer =
            egui_wgpu::Renderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, None, 1, false);
        let mut form = crate::ui::frame_set_form::FrameSetForm::default();
        form.open = true;
        form.video = Some(temp.path().join("Видео для набора.mp4"));
        for iteration in 0..12 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1000.0),
                )),
                ..Default::default()
            };
            let output = ctx.run(input, |ctx| {
                screen.render(ctx);
                if iteration >= 8 {
                    form.show(ctx, crate::ui::Language::Russian, false, temp.path(), 128);
                }
            });
            let jobs = ctx.tessellate(output.shapes, 1.0);
            let descriptor = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [1000, 1000],
                pixels_per_point: 1.0,
            };
            for (id, delta) in &output.textures_delta.set {
                renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &descriptor);
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.canvas_view,
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
                renderer.render(&mut pass.forget_lifetime(), &jobs, &descriptor);
            }
            gpu.queue.submit(Some(encoder.finish()));
            std::thread::sleep(std::time::Duration::from_millis(40));
            if iteration == 7 {
                crate::io::output::read_canvas_image(&gpu)
                    .unwrap()
                    .save("target/settings-preview.png")
                    .unwrap();
            }
        }
        crate::io::output::read_canvas_image(&gpu)
            .unwrap()
            .save("target/frame-set-preview.png")
            .unwrap();
        assert_eq!(screen.settings.batch_size, 5000);
        assert_eq!(screen.settings.num_generations, 5_000_000);
    }

    #[test]
    #[ignore = "requires a real GPU; run explicitly"]
    fn gpu_chunked_evaluation_and_long_run_cancellation() {
        let settings = Settings {
            batch_size: 1100,
            shape_resolution: 16,
            max_texture_size: 32,
            num_generations: 5_000_000,
            use_min_improvement: false,
            ..Settings::default()
        };
        let gpu = Arc::new(
            GpuContext::new(
                &vec![255; 32 * 32 * 4],
                (32, 32),
                &[ShapeLayer {
                    pixels: vec![255; 16 * 16 * 4],
                }],
                &settings,
            )
            .unwrap(),
        );
        let mut generator = crate::algorithm::CandidateGenerator::new(
            settings.clone(),
            vec![255; 32 * 32 * 4],
            (32, 32),
        );
        let candidates = generator.generate_batch(2300, 0, (32, 32), 1);
        let scores = gpu.evaluate_candidates(&candidates);
        assert_eq!(scores.len(), candidates.len());
        assert!(scores.iter().all(|s| s.is_finite()));
        for index in [0, 1099, 1100, 2299] {
            let single = gpu.evaluate_candidates(&candidates[index..index + 1]);
            assert!((single[0] - scores[index]).abs() < 0.01);
        }
        gpu.composite_shape(&candidates[0]);
        assert!(crate::io::output::read_canvas_image(&gpu)
            .unwrap()
            .pixels()
            .all(|p| p[3] == 255));
        let video_settings = Settings {
            batch_size: 8,
            num_generations: 2,
            mutations_per_shape: 1100,
            max_shapes: 2,
            ..settings.clone()
        };
        let mut pipeline = crate::algorithm::VideoPipeline::new();
        pipeline.record_placed_shape(candidates[0]);
        pipeline.adapt_to_new_frame(&gpu, &mut generator, &video_settings);
        pipeline.grow_population(&gpu, &mut generator, &video_settings);
        pipeline.render_interpolated_frame(&gpu, 0.5);
        let control = gpu.control.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let work_gpu = gpu.clone();
        let worker = std::thread::spawn(move || {
            let mut climber = crate::algorithm::HillClimber::new();
            climber.step(&work_gpu, &mut generator, &settings);
            tx.send(()).unwrap();
        });
        let deadline = std::time::Instant::now();
        while control.generations.load(Ordering::Relaxed) == 0 {
            assert!(deadline.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        control.paused.store(true, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let paused = control.generations.load(Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(paused, control.generations.load(Ordering::Relaxed));
        control.cancelled.store(true, Ordering::Relaxed);
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .expect("Cancellation should interrupt millions of generations, even while paused");
        worker.join().unwrap();
    }
    #[test]
    #[ignore = "GPU diagnostic benchmark; run in release mode"]
    fn benchmark_evaluation_readback() {
        let settings = Settings {
            batch_size: 2000,
            shape_resolution: 128,
            max_texture_size: 512,
            scale_min: 0.15,
            scale_max: 8.0,
            use_original_colors: true,
            num_generations: 10,
            ..Settings::default()
        };
        let gpu = GpuContext::new(
            &vec![220; 512 * 512 * 4],
            (512, 512),
            &[ShapeLayer {
                pixels: vec![255; 128 * 128 * 4],
            }],
            &settings,
        )
        .unwrap();
        let mut generator = crate::algorithm::CandidateGenerator::new(
            settings.clone(),
            vec![220; 512 * 512 * 4],
            (512, 512),
        );
        let candidates = generator.generate_batch(2000, 0, (512, 512), 1);
        let reference = gpu.evaluate_candidates(&candidates);
        for (label, chunk) in [
            ("256-candidate chunks", 256),
            ("Full buffer", gpu.batch_size as usize),
        ] {
            let start = std::time::Instant::now();
            let mut scores = Vec::new();
            for _ in 0..4 {
                scores.clear();
                for part in candidates.chunks(chunk) {
                    gpu.dispatch_mse_evaluation(part);
                    scores.extend(gpu.read_fitness_scores());
                }
            }
            eprintln!(
                "{label}: {:.1} ms / 2000 candidates, {:.0} candidates/s",
                start.elapsed().as_secs_f64() * 250.0,
                8000.0 / start.elapsed().as_secs_f64()
            );
            for (a, b) in scores.iter().zip(&reference) {
                assert!((a - b).abs() < 0.1);
            }
        }
        let mut climber = crate::algorithm::HillClimber::new();
        let start = std::time::Instant::now();
        for _ in 0..4 {
            gpu.evaluate_candidates(&candidates);
        }
        eprintln!(
            "Optimized evaluation: {:.1} ms / 2000 candidates, {:.0} candidates/s",
            start.elapsed().as_secs_f64() * 250.0,
            8000.0 / start.elapsed().as_secs_f64()
        );
        for _ in 0..3 {
            climber.step(&gpu, &mut generator, &settings);
        }
        assert!(climber.placed_shapes > 0);
        let canvas = crate::io::output::read_canvas_image(&gpu).unwrap();
        assert!(canvas.pixels().any(|p| p[0] > 20));
        eprintln!("Visible shapes verified: {}", climber.placed_shapes);
    }
}
