# TeekasFigure

**GPU-accelerated evolutionary image & video approximation using geometric shapes**

[🇷🇺 Читать на русском](README_ru.md)

---

## What is TeekasFigure?

TeekasFigure recreates images and videos by placing geometric shapes (custom PNG textures) onto a canvas using an evolutionary algorithm. The GPU (via WGPU/Vulkan/DX12) evaluates thousands of candidates in parallel, making the process fast and visually stunning.

The algorithm evolves a population of shape candidates through selection, mutation, and survival-of-the-fittest — producing artistic approximations that look like paintings made of geometric brushstrokes. Shapes can be fully opaque or semi-transparent based on your settings.

### Features

- **Evolutionary algorithm** — Tournament selection, mutation, multi-generation refinement
- **GPU-accelerated** — Compute shaders evaluate 1000+ candidates per batch in parallel
- **Custom shapes** — Use any PNG as a brush (circles, squares, splashes, leaves, or even a photo of your dog!)
- **Shape diversity mode** — Prevents the algorithm from always picking the "mathematically optimal" shape
- **Real-time visualization** — Watch the artwork being constructed live at 60 FPS
- **Video support (Demo)** — An experimental mode with temporal coherence to keep shapes stable between frames. *Note: currently in early beta, may be unstable or produce suboptimal results.*
- **Highly Configurable** — Detailed control over evolution parameters via `settings.toml`
- **Zero-CLI** — Folder-based workflow

### Requirements

- **Rust** 1.70+ ([rustup.rs](https://rustup.rs/))
- **GPU** with Vulkan, DX12, or Metal support
- **FFmpeg** (optional, for video processing) — must be in PATH

### Quick Start

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/TeekasFigure.git
cd TeekasFigure

# Build release
cargo build --release

# Run
cargo run --release
```

### Folder Structure

```text
TeekasFigure/
├── input_media/       ← Place your target image (PNG/JPG/BMP) or video (MP4) here
├── raw_shapes/        ← Place your raw custom images/shapes here
├── input_shapes/      ← Program reads prepared textures from here (do NOT put raw files here)
├── output/            ← Results are saved here automatically
├── settings.toml      ← Configuration (auto-created on first run)
└── TeekasFigure.exe   ← The application
```

### ⚠️ Preparing Custom Shapes (Important)

**Do NOT put raw images directly into the `input_shapes/` folder!** The program requires shapes to be perfectly formatted (128x128, specific grayscale + alpha channels) and will likely reject or crash on unprocessed files.

Instead, place your raw images (PNG, JPG, BMP, WebP) into the `raw_shapes/` folder and run the built-in preparation script:

```bash
cargo run --example prepare_shapes
```
This script will automatically resize, reformat, and move the processed shapes into the `input_shapes/` folder for you.

### Configuration (`settings.toml`)

All parameters are tunable via `settings.toml`. Below is a detailed description of every setting:

| Parameter | Default | Description |
|-----------|---------|-------------|
| **`batch_size`** | 1000 | Number of candidates generated and evaluated per batch on the GPU (1-4096). |
| **`max_shapes`** | 4000 | Maximum number of shapes to place before the algorithm stops. |
| **`mutations_per_frame`** | 1 | How many shapes are successfully placed per rendered 60FPS frame. |
| **`max_texture_size`** | 512 | Maximum dimension (width/height) the input media will be scaled to. |
| **`vram_budget_mb`** | 2048 | Maximum allowed VRAM usage for the shape textures array. |
| **`scale_min`** | 0.02 | Minimum allowed scale factor for a shape. |
| **`scale_max`** | 9.0 | Maximum allowed scale factor for a shape at the start. |
| **`shape_resolution`** | 128 | Internal resolution of the shapes (do not change unless you modify the prep script). |
| **`mutations_per_shape`** | 1 | (Video mode only) Local mutation attempts per existing shape. |
| **`displacement_weight`** | 0.1 | (Video mode only) Movement penalty to stop shapes from jittering. |
| **`target_fps`** | 12 | (Video mode only) Framerate to resample input video to. |
| **`evolve_opacity`** | true | If `true`, the algorithm will use varying opacity for shapes. If `false`, shapes are fully opaque. |
| **`num_generations`** | 6 | Number of evolutionary generations per shape placement. |
| **`min_improvement`** | -0.5 | Minimum MSE delta required to accept a shape (negative = improvement). |
| **`use_min_improvement`** | true | Whether to use the `min_improvement` threshold. Set to `false` if using `diversity_mode`. |
| **`max_rejections`** | 50 | Consecutive failed batches before the program assumes convergence and stops. |
| **`survival_rate`** | 0.10 | Fraction of the population that survives to the next generation (e.g., 0.10 = 10%). |
| **`children_per_parent`** | 9 | Number of mutated children each survivor produces. |
| **`diversity_mode`** | false | Enable shape diversity penalties. See note below. |
| **`diversity_penalty_increment`**| 5.0 | Penalty score added to a shape each time it gets used on the canvas. |
| **`diversity_decay_enabled`**| true | Whether other shapes' penalties decay when one shape is chosen. |
| **`diversity_decay_amount`** | 0.01 | How much penalty is removed from unused shapes per step. |

> **🔥 IMPORTANT FOR DIVERSITY MODE:** 
> When you enable `diversity_mode = true`, the algorithm adds penalty scores to frequently used shapes to force variety. However, this penalty will artificially ruin the "improvement score". Because of this, **you MUST set `use_min_improvement = false`** when using Diversity Mode, otherwise the algorithm will reject everything and get stuck.

### Controls

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume |
| `S` | Save snapshot immediately |
| `Escape` | Exit |

### License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.