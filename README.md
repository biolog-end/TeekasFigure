# TeekasFigure

**GPU-accelerated evolutionary image & video approximation using geometric shapes**

[🇷🇺 Читать на русском](README_ru.md)

---

## What is TeekasFigure?

TeekasFigure recreates images and videos by sequentially placing geometric shapes (your custom PNG textures) onto a canvas using an evolutionary algorithm. The GPU (via WGPU/Vulkan/DX12) evaluates thousands of placement candidates in parallel, making the process incredibly fast and visually stunning.

The algorithm evolves a population of shape candidates through tournament selection, mutation, and survival-of-the-fittest, producing artworks that look like paintings made of geometric brushstrokes. In video mode, the algorithm gives shapes "memory" and a life cycle, creating incredibly smooth and mesmerizing animations.

### Key Features

- **Evolutionary Algorithm** — Tournament selection, mutations, and multi-generation refinement.
- **GPU-accelerated** — Compute shaders evaluate thousands of candidates in parallel in fractions of a second.
- **Custom Shapes** — Use any PNG as a brush (circles, squares, splashes, leaves, logos—anything!).
- **Advanced Video Processing (Temporal Coherence)** — Shapes aren't just redrawn every frame. They "live" on the canvas, adapting to changes (moving, rotating, scaling), and die only during harsh scene changes to make room for new ones.
- **Frame Interpolation (Motion Blur effect)** — The algorithm can generate intermediate frames, making the animation incredibly smooth (shapes glide smoothly from one point to another, new shapes softly fade in, and old ones fade out).
- **Shape Diversity Mode** — Prevents the algorithm from being "lazy" and forces it to use your entire brush arsenal.
- **Highly Configurable** — Detailed control over evolution, shape life cycles, and the video pipeline via the `settings.toml` file.

### Requirements

- **Rust** 1.70+ ([rustup.rs](https://rustup.rs/))
- **GPU** with Vulkan, DX12, or Metal support
- **FFmpeg** (required for video processing) — must be installed and added to your system PATH.

### Quick Start

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/TeekasFigure.git
cd TeekasFigure

# Build release (MANDATORY for adequate performance)
cargo build --release

# Run
cargo run --release
```

### Folder Structure

```text
TeekasFigure/
├── input_media/       ← Place your target image (PNG/JPG/BMP) or video (MP4) here
├── raw_shapes/        ← Place your raw custom images/brushes here
├── input_shapes/      ← Program reads prepared textures from here (do NOT put raw files here!)
├── output/            ← Finished artworks (PNG, MP4) are saved here
├── settings.toml      ← Configuration file (auto-created on first run)
└── TeekasFigure.exe   ← Compiled application binary
```

### ⚠️ Preparing Custom Shapes (Very Important!)

**Do NOT put your raw images directly into the `input_shapes/` folder!** The program requires shapes to be in a strictly defined format (128x128, grayscale + preserved alpha channel). If you feed it standard PNGs, the algorithm won't be able to process them.

Instead, place your raw images (PNG, JPG, BMP, WebP) into the `raw_shapes/` folder and run the built-in prep script:

```bash
cargo run --example prepare_shapes
```
This script will automatically crop, recolor, and move the ready-to-use optimized textures into the `input_shapes/` folder.

### Configuration (`settings.toml`)

All parameters are generated in the `settings.toml` file upon the first launch. They are grouped logically for your convenience:

#### ⚙️ Basic Settings
| Parameter | Description |
|-----------|-------------|
| **`batch_size`** | Number of candidates generated and evaluated per GPU pass (1–4096). |
| **`max_shapes`** | Maximum number of shapes on the canvas. In video mode, this acts as the target population size. |
| **`mutations_per_frame`** | How many shapes are successfully placed per rendered UI frame (affects visual speed). |
| **`max_texture_size`** | Maximum resolution the input media will be downscaled to (saves resources). |
| **`vram_budget_mb`** | Video memory limit for the shape texture array (in megabytes). |
| **`scale_min` / `scale_max`**| The minimum and starting maximum size of a shape (scale adaptively decreases as the canvas fills). |
| **`shape_resolution`** | Resolution of loaded shapes (default 128, change only if you modified the prep script). |

#### 🎬 Video & Animation Settings
| Parameter | Description |
|-----------|-------------|
| **`target_fps`** | Framerate to downsample the input video to before processing. Final output FPS will be higher if interpolation is enabled. |
| **`scene_change_tolerance`**| Shape "death" threshold (-10.0 to 10.0). If a shape worsens the image beyond this value, it dies and is reborn as a new one. **Negative values** enforce strictness: a shape *must* actively improve its area, or it dies. Lower value = more aggressive canvas redrawing. |
| **`interpolation_steps`** | Number of smoothly interpolated frames between keyframes. `0` disables it. If > 0, the final video becomes super smooth, and new shapes fade in softly. Final FPS = `target_fps * (steps + 1)`. |
| **`video_recolor`** | `false` (default) — a shape permanently remembers its original color. `true` — shapes continuously recolor themselves to match the new frame (may lead to washed-out details). |
| **`mutations_per_shape`** | How many local movement/scaling attempts a shape gets to adapt to a new video frame. |
| **`displacement_weight`** | Penalty for moving a shape too far in a video (forces shapes to "hold" their positions, preventing chaotic jitter). |

#### 🧬 Evolution Parameters
| Parameter | Description |
|-----------|-------------|
| **`evolve_opacity`** | Whether to allow the algorithm to pick varying opacity (`true`), or always draw fully opaque strokes (`false`). |
| **`num_generations`** | Number of evolutionary generations (tournaments and mutations) to find the perfect shape. |
| **`min_improvement`** | Minimum MSE improvement threshold required to accept a stroke (negative = improvement). |
| **`use_min_improvement`** | Whether to enforce the `min_improvement` threshold. **Important: disable this when using `diversity_mode`!** |
| **`max_rejections`** | How many consecutive failures the algorithm must hit before declaring the artwork finished. |
| **`survival_rate`** | The fraction of top shapes (e.g., 0.10 = 10%) that survive to breed in the next generation. |
| **`children_per_parent`** | Number of slightly modified (mutated) copies created from each surviving shape. |

#### 🎨 Diversity Mode
| Parameter | Description |
|-----------|-------------|
| **`diversity_mode`** | Enables penalties for overused shapes, forcing the algorithm to use different brushes. |
| **`diversity_penalty_increment`**| Penalty score added to a shape each time it is placed on the canvas. |
| **`diversity_decay_enabled`**| Whether penalties decay over time, allowing old brushes to be used again later. |
| **`diversity_decay_amount`** | How much penalty is removed from unused shapes per step. |

> **🔥 IMPORTANT FOR DIVERSITY MODE:**
> The penalty score artificially distorts the mathematical "image improvement" evaluation. Therefore, **when you set `diversity_mode = true`, you MUST set `use_min_improvement = false`**, otherwise the algorithm will reject all shapes due to their penalties and generation will stall.

### In-App Controls

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume generation |
| `S` | Instantly save a snapshot of the current progress to the `output/` folder |
| `Escape` | Emergency exit |

### License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.