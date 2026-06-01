// MSE evaluation compute shader
// One workgroup per candidate, workgroup_size(256) threads per workgroup.
// Each workgroup computes the MSE for a single candidate shape placement.

struct CandidateParams {
    shape_index: u32,
    x: f32,
    y: f32,
    rotation: f32,
    scale: f32,
    r: f32,
    g: f32,
    b: f32,
    alpha: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

struct EvalUniforms {
    canvas_width: u32,
    canvas_height: u32,
    num_candidates: u32,
    shape_resolution: u32,
    displacement_weight: f32,
}

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var target_tex: texture_2d<f32>;
@group(0) @binding(2) var shapes: texture_2d_array<f32>;
@group(0) @binding(3) var<storage, read> candidates: array<CandidateParams>;
@group(0) @binding(4) var<storage, read_write> fitness: array<f32>;
@group(0) @binding(5) var<uniform> uniforms: EvalUniforms;

var<workgroup> partial_sums: array<f32, 256>;

@compute @workgroup_size(256)
fn eval_mse(
    @builtin(workgroup_id) wg_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
) {
    let candidate_idx = wg_id.x;
    let thread_idx = local_id.x;

    // Load candidate parameters
    let candidate = candidates[candidate_idx];

    // Compute bounding box from candidate position and scale
    let shape_res = f32(uniforms.shape_resolution);
    let shape_size = candidate.scale * shape_res;
    let half_size = shape_size * 0.5;

    let center = vec2<f32>(candidate.x, candidate.y);

    // Axis-aligned bounding box (conservative: accounts for rotation by using half_size in both axes)
    // For a rotated square of half-diagonal = half_size * sqrt(2), but we use half_size * sqrt(2)
    // to be safe. Actually for a square rotated by any angle, the AABB half-extent is:
    // half_size * (|cos(r)| + |sin(r)|) per axis, but since we're dealing with a square shape,
    // the maximum extent is half_size * sqrt(2). We use a simpler conservative bound.
    let cos_r = cos(candidate.rotation);
    let sin_r = sin(candidate.rotation);
    let abs_cos = abs(cos_r);
    let abs_sin = abs(sin_r);
    // For a square with half-side = half_size, rotated AABB half-extents:
    let extent_x = half_size * (abs_cos + abs_sin);
    let extent_y = half_size * (abs_sin + abs_cos);

    let bb_min_x = max(0, i32(floor(center.x - extent_x)));
    let bb_min_y = max(0, i32(floor(center.y - extent_y)));
    let bb_max_x = min(i32(uniforms.canvas_width), i32(ceil(center.x + extent_x)));
    let bb_max_y = min(i32(uniforms.canvas_height), i32(ceil(center.y + extent_y)));

    let bb_width = bb_max_x - bb_min_x;
    let bb_height = bb_max_y - bb_min_y;
    let total_pixels = bb_width * bb_height;

    // Edge case: if bounding box has 0 pixels, write max fitness
    if total_pixels <= 0 {
        partial_sums[thread_idx] = 0.0;
        workgroupBarrier();
        if thread_idx == 0u {
            fitness[candidate_idx] = 3.40282347e+38; // f32 max approximation
        }
        return;
    }

    // Each thread accumulates squared error for its assigned pixels
    var thread_error: f32 = 0.0;

    // Distribute pixels across 256 threads: thread processes pixel indices
    // thread_idx, thread_idx + 256, thread_idx + 512, ...
    var pixel_idx = i32(thread_idx);
    let total_pixels_i = total_pixels;

    // Precompute inverse rotation for shape-local coordinate transform
    let inv_cos = cos(-candidate.rotation);
    let inv_sin = sin(-candidate.rotation);

    while pixel_idx < total_pixels_i {
        // Convert linear pixel index to 2D coordinates within bounding box
        let local_x = pixel_idx % bb_width;
        let local_y = pixel_idx / bb_width;

        let px = bb_min_x + local_x;
        let py = bb_min_y + local_y;

        // Vector from candidate center to this pixel
        let delta_x = f32(px) + 0.5 - center.x;
        let delta_y = f32(py) + 0.5 - center.y;

        // Apply inverse rotation to get shape-local coordinates
        let shape_local_x = delta_x * inv_cos - delta_y * inv_sin;
        let shape_local_y = delta_x * inv_sin + delta_y * inv_cos;

        // Convert to UV in shape texture space [0, 1]
        let u = (shape_local_x / shape_size) + 0.5;
        let v = (shape_local_y / shape_size) + 0.5;

        // Load canvas and target pixels (always needed for MSE in bounding box)
        let canvas_pixel = textureLoad(canvas, vec2<i32>(px, py), 0);
        let target_pixel = textureLoad(target_tex, vec2<i32>(px, py), 0);

        // Compute current error (canvas vs target, without the shape)
        let current_diff = canvas_pixel.rgb - target_pixel.rgb;
        let current_error = dot(current_diff, current_diff);

        // Determine composited color (canvas + shape)
        var composited: vec3<f32>;

        if u >= 0.0 && u <= 1.0 && v >= 0.0 && v <= 1.0 {
            // Sample shape texture at integer coordinates
            let tex_x = i32(u * shape_res);
            let tex_y = i32(v * shape_res);
            // Clamp to valid texture coordinates
            let clamped_x = clamp(tex_x, 0, i32(uniforms.shape_resolution) - 1);
            let clamped_y = clamp(tex_y, 0, i32(uniforms.shape_resolution) - 1);

            let shape_texel = textureLoad(shapes, vec2<i32>(clamped_x, clamped_y), i32(candidate.shape_index), 0);

            // Shape is grayscale+alpha: luminance in .r, alpha in .a
            let luminance = shape_texel.r;
            let shape_alpha = shape_texel.a * candidate.alpha;

            // Tint shape by candidate color
            let shape_color = vec3<f32>(candidate.r, candidate.g, candidate.b) * luminance;

            // Alpha-blend shape over canvas: composited = shape_color * alpha + canvas * (1 - alpha)
            composited = shape_color * shape_alpha + canvas_pixel.rgb * (1.0 - shape_alpha);
        } else {
            // Outside shape bounds: composited is just the canvas pixel
            composited = canvas_pixel.rgb;
        }

        // Compute new error (composited vs target)
        let new_diff = composited - target_pixel.rgb;
        let new_error = dot(new_diff, new_diff);

        // Accumulate the IMPROVEMENT (negative = better)
        // fitness = sum(new_error - old_error) — negative means improvement
        thread_error += new_error - current_error;

        pixel_idx += 256;
    }

    // Store this thread's accumulated error
    partial_sums[thread_idx] = thread_error;
    workgroupBarrier();

    // Parallel reduction: sum all partial_sums into partial_sums[0]
    var stride: u32 = 128u;
    while stride > 0u {
        if thread_idx < stride {
            partial_sums[thread_idx] += partial_sums[thread_idx + stride];
        }
        workgroupBarrier();
        stride = stride >> 1u;
    }

    // Thread 0 writes final fitness = total delta error (negative = improvement)
    if thread_idx == 0u {
        fitness[candidate_idx] = partial_sums[0];
    }
}
