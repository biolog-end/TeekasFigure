# TeekasFigure

**GPU-accelerated evolutionary image & video approximation using geometric shapes**

[Читать на понятном](README_ru.md)

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
- **Built-in Settings UI** — Configure everything in-app (English/Russian) with smart sliders and toggles, a file picker, and savable presets — no need to hand-edit `settings.toml`.
- **Progress GIF** — Optionally save an animated GIF of the creation process next to the result image.
- **Audio-preserving video** — The original soundtrack of a video is kept in the rendered MP4.
- **Broad input format support** — Images: PNG, JPG/JPEG, WebP, BMP, GIF (first frame), TIFF, TGA, ICO, QOI, PNM and more (via the `image` crate). Videos: MP4, MOV, MKV, AVI, WebM, WMV, M4V, MPG/MPEG, TS, 3GP, OGV and more (decoded natively by FFmpeg — no manual conversion needed). Output is always rendered to MP4.
- **Highly Configurable** — Detailed control over evolution, shape life cycles, and the video pipeline via the in-app UI and the `settings.toml` file.

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

### Using the App

When you launch TeekasFigure, it opens on the **Settings screen** (one window, two screens: Settings → Generation):

- **Language** — switch the interface between English and Russian (top-right). Your choice is remembered between runs.
- **Input file** — pick any image/video from `input_media/` in the dropdown (press *Refresh* after adding new files).
- **Parameters** — every setting is a slider (you can also click it to type an exact number) or a toggle, grouped into collapsing sections. Irrelevant options grey out automatically — e.g. video options when an image is selected, or diversity sub-options when diversity mode is off.
- **Presets** — save the current configuration under a name, load it back later, or delete it. Presets are stored as `presets/*.toml`.
- **Save settings.toml** — writes the current form to `settings.toml` without starting.
- **▶ Start** — validates the settings, saves them to `settings.toml`, then loads the media + shapes and begins generation.

On the next launch the form loads its initial values straight from `settings.toml`.

### Folder Structure

```text
TeekasFigure/
├── input_media/       ← Place your target image (PNG/JPG/WebP/BMP/...) or video (MP4/MOV/MKV/AVI/WebM/...) here
├── raw_shapes/        ← Place your raw custom images/brushes here
├── input_shapes/      ← Program reads prepared textures from here (do NOT put raw files here!)
├── output/            ← Finished artworks (PNG, MP4, and optional _process.gif) are saved here
├── presets/           ← Saved settings presets (created from the in-app UI)
├── settings.toml      ← Configuration file (auto-created on first run, updated from the UI)
└── TeekasFigure.exe   ← Compiled application binary
```

### ⚠️ Preparing Custom Shapes (Very Important!)

**Do NOT put your raw images directly into the `input_shapes/` folder!** The program requires shapes to be in a strictly defined format (128x128, grayscale + preserved alpha channel). If you feed it standard PNGs, the algorithm won't be able to process them.

Instead, place your raw images (PNG, JPG, BMP, WebP) into the `raw_shapes/` folder and run the built-in prep script:

```bash
cargo run --example prepare_shapes
```
This script will automatically crop, recolor, and move the ready-to-use optimized textures into the `input_shapes/` folder.

### 🍎 Bad Apple Addon (optional)

There's a bonus tool for the classic "Bad Apple!!"-style effect: rebuild a black-and-white silhouette video out of its **own** frames. The `prepare_bad_apple` example samples frames from such a clip, strips the solid background to transparency, draws a clean outline around each silhouette, and saves them as colored brushes in `raw_shapes/`.

```bash
cargo run --release --example prepare_bad_apple -- "path/to/clip.mp4"
```
(or just run `prepare_bad_apple.bat` and pass your own video path)

| Option | Description |
|--------|-------------|
| `--interval <sec>` | Seconds between sampled frames (default `2.0`). |
| `--out <folder>` | Output folder for the shape PNGs (default `raw_shapes`). |
| `--max-size <px>` | Longest side of each output shape in pixels (default `512`). |
| `--mono <0..1>` | Drop near-blank frames where a single near-pure color covers ≥ this fraction (default `0.97`). |
| `--bg-tol <0..255>` | How far from pure white/black still counts as removable background (default `40`). |
| `--outline <px>` | Thickness of the silhouette outline drawn in the background color (default `3`). |

After preparing the brushes: put the source clip into `input_media/`, set `use_original_colors = true` in `settings.toml` (so the black/white fills are kept intact), then run the app to approximate the video.

> **Note:** the source video and the `bad apple addon/` folder are intentionally excluded from version control. Supply your own clip — FFmpeg must be in your PATH.

### Configuration (`settings.toml`)

All parameters are generated in the `settings.toml` file upon the first launch. The easiest way to change them is the in-app Settings screen, but you can also edit the file directly. They are grouped logically for your convenience:

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
| **`preserve_audio`** | Keep the source video's original audio track in the rendered MP4 (`true`, default). If the source has no audio, this is a no-op. |

#### 🎞️ Progress GIF (image mode)
| Parameter | Description |
|-----------|-------------|
| **`save_progress_gif`** | Save an animated GIF of the creation process next to the result image (`false` by default). Applies to images only — produces `<name>_process.gif` in `output/`. |
| **`gif_fps`** | Playback speed of the GIF in frames per second (1–50). |
| **`gif_frames`** | Approximate number of frames captured, spread evenly across the placement process (2–2000). |
| **`gif_max_width`** | Maximum GIF width in pixels; larger canvases are downscaled to keep the file small (16–2048). |

#### 🧬 Evolution Parameters
| Parameter | Description |
|-----------|-------------|
| **`evolve_opacity`** | Whether to allow the algorithm to pick varying opacity (`true`), or always draw fully opaque strokes (`false`). |
| **`use_original_colors`** | Use the shapes' original colors (`true`) instead of tinting grayscale brushes (`false`, default). When `true`, shapes are loaded from the **`raw_shapes/`** folder keeping their original RGB colors and are never recolored — only placed/moved/rotated/scaled. Put your colored PNG shapes (ideally with transparency) into `raw_shapes/`. |
| **`evolve_non_uniform_scale`** | Allow independent X/Y axis scaling (`true`) so shapes can stretch and squash (circle → ellipse, square → rectangle). Default `false` (uniform scaling). |
| **`evolve_hue`** | Real-color mode only (`use_original_colors = true`). Evolve the **hue** of the shapes' original colors (`true`) so each shape can rotate its color around the wheel to better match the target. Default `false`. |
| **`evolve_saturation`** | Real-color mode only (`use_original_colors = true`). Evolve the **saturation** of the shapes' original colors (`true`) so each shape can become more/less vivid to better match the target. Default `false`. |
| **`evolve_brightness`** | Real-color mode only (`use_original_colors = true`). Evolve the **brightness** (value) of the shapes' original colors (`true`) so each shape can become darker or brighter to better match the target. Default `false`. |
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
> The penalty score artificially distorts the mathematical "image improvement" evaluation. Therefore, **when `diversity_mode = true`, `use_min_improvement` must be `false`**, otherwise the algorithm will reject all shapes due to their penalties and generation will stall. The in-app Settings screen enforces this for you automatically (it disables `use_min_improvement` while diversity mode is on); if you edit `settings.toml` by hand, set it yourself.

### In-App Controls

Generation starts after you press **▶ Start** on the Settings screen. During generation:

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume generation |
| `S` | Instantly save a snapshot of the current progress to the `output/` folder |
| `Escape` | Emergency exit |

### License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.