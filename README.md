# TeekasFigure

**GPU-accelerated evolutionary image & video approximation using geometric shapes**

[Читать на русском](README_ru.md)

---

## What is TeekasFigure?

TeekasFigure recreates images and videos by sequentially placing shapes (your own PNG textures) onto a canvas using an evolutionary algorithm. The GPU (via WGPU on Vulkan/DX12/Metal) evaluates thousands of placement candidates in parallel, making the process fast and fun to watch.

The algorithm evolves a population of shape candidates through tournament selection, mutation and survival of the fittest, producing artworks that look like paintings made of brushstrokes. In video mode, shapes get "memory" and a life cycle, which produces smooth, coherent animations.

### Key Features

- **Evolutionary algorithm** — tournament selection, mutations and multi-generation refinement.
- **GPU-accelerated** — compute shaders evaluate thousands of candidates in parallel.
- **Custom shapes** — use any image as a brush: circles, splashes, leaves, logos, memes, video frames.
- **Temporal coherence for video** — shapes are not redrawn from scratch every frame. They live on the canvas, move, rotate and scale to follow the video, and die only on harsh scene changes to make room for new ones.
- **Frame interpolation** — intermediate frames make shapes glide between keyframes, fade in when born and fade out when they die.
- **Shape libraries** — thumbnail libraries for targets and building blocks, with drag-and-drop, deletion and undo.
- **Shape sets from video** — cut frames from any clip into a brush set, optionally removing the background (plain color or U²-Net).
- **Palette adaptation** — prepare a target so it matches the colors your original-color sprites can actually produce.
- **Brightness priority** — optional BT.709 luma/chroma scoring that favors light and shadow over exact hue.
- **Minecraft playback** — export vanilla mob models as brushes, then replay the result in-game with real mobs.
- **Shape diversity mode** — stops the algorithm from overusing a few favorite brushes.
- **Built-in settings UI** — English/Russian interface with sliders, toggles and named presets.
- **Progress GIF** — optionally save an animated GIF of the creation process.
- **Audio-preserving video** — the source soundtrack is kept in the rendered MP4.
- **Broad format support** — images: PNG, JPG/JPEG, WebP, BMP, GIF (first frame), TIFF, TGA, ICO, QOI, PNM and more. Videos: anything FFmpeg can read (MP4, MOV, MKV, AVI, WebM, WMV, …). Video output is always MP4.

### Requirements

- **Rust** 1.80+ ([rustup.rs](https://rustup.rs/))
- **GPU** with Vulkan, DX12 or Metal support
- **FFmpeg** and **ffprobe** on your PATH (required for video)
- **Windows SDK** Resource Compiler (`rc.exe`) to build the Windows executable with its icon; the built `.exe` does not need the SDK
- Optional: **Python** with `numpy` and `Pillow` for shape sets from video (plus `onnxruntime` for AI background removal)
- Optional: **JDK 21** to build the Minecraft mod

### Quick Start

```bash
git clone https://github.com/biolog-end/TeekasFigure.git
cd TeekasFigure

# Release build is strongly recommended for performance
cargo run --release
```

---

## Using the App

TeekasFigure opens on the **Settings screen**. One window has two screens: Settings → Generation.

- **Language** — switch between English and Russian (top right). The choice is remembered.
- **1. What to recreate** — the target library. Add files with **+ Add images / videos…** and click a thumbnail to select it. One target is generated at a time.
- **2. Building blocks** — the shape library. **+ Add photos…** copies the originals and prepares grayscale brushes with their transparency preserved. Original-color mode uses the colored originals.
- **Parameters** — every setting is a slider (click to type an exact number) or a toggle, grouped into collapsible sections. Options that don't apply grey out automatically.
- **Presets** — save the current configuration under a name, load it later or delete it. Stored as `presets/*.toml`.
- **Save settings.toml** — write the form to `settings.toml` without starting.
- **▶ Start** — validate the settings, save them and begin generation.

The add buttons open the standard Windows Explorer dialog with Ctrl/Shift multi-selection. You can also drag several files into the window and choose where they go. Name clashes get unique names; external originals are never modified.

To delete, tick the thumbnails and press **Delete**. A building block is removed together with its prepared copy. **Undo deletion** restores the latest batch, even after a restart. Deleted files are kept in `.library_trash/` next to the app until you clear it manually.

Slider ranges are recommendations, not hard caps. You can type larger values; out-of-range values get an orange warning. Before starting, the app checks GPU and RAM requirements and stops with a clear message if there isn't enough memory or disk space.

The settings screen shows the GPU in use. Automatic selection prefers a discrete GPU; with several GPUs you can pick one from the list, save, and restart.

### During Generation

A sidebar shows progress, placed shapes, rejected searches, CPU/RAM/GPU usage and candidates evaluated per second. It also has **Pause**, **Snapshot** and **Stop / Settings** buttons. **Hide · H** gives the whole window to the canvas.

| Key | Action |
|-----|--------|
| `Space` | Pause / resume |
| `S` | Save a snapshot of the current canvas to `output/` |
| `H` | Hide / show the sidebar |
| `Escape` | Stop generation and return to Settings (on the Settings screen: quit) |

Results are saved to `output/`: `<name>_result.png` for images, `<name>_result.mp4` for videos, and optionally `<name>_process.gif`.

---

## Preparing Custom Shapes

Use **Building blocks → + Add photos…** to import images; grayscale brushes are prepared automatically. **Folder conversion → Convert folder to grayscale** reprocesses existing originals from `raw_shapes/` into `input_shapes/` at the shape resolution chosen in settings. Aspect ratio and the original alpha channel are preserved.

The same conversion is available from the command line (fixed 128×128):

```bash
cargo run --release --example prepare_shapes -- raw_shapes input_shapes
```

## Shape Sets from Any Video

In **Building blocks**, press **Create set from video…**, pick a video, then set **From**, **Until** and **Capture every, seconds**. For example, `2:00` → `3:00` every `2` seconds gives about 30 pictures. The end time is excluded; an empty end means the end of the clip. Choose a maximum frame size and a background mode:

- **Keep background** — the whole frame.
- **Remove plain background** — removes the dominant edge-connected color; good for silhouette clips like Bad Apple.
- **AI foreground (U²-Net)** — extracts the main object locally with [U²-Net](https://github.com/xuebinqin/U-2-Net) via [ONNX Runtime](https://onnxruntime.ai/). Complex scenes may need cleanup.

Transparent margins can be cropped and solid-color frames skipped. The new set is selected automatically and stored in `frame_sets/<name>/`. A checkbox can also select the same video as the generation target.

Background processing needs Python on PATH:

```bash
python -m pip install numpy Pillow onnxruntime
```

AI mode looks for `models/u2net.onnx` or `models/u2netp.onnx` next to the executable, then `U2NET_HOME`, then existing `.u2net/` and `.rembg/models/` caches in your user profile. Compatible weights are linked from [rembg](https://github.com/danielgatis/rembg). The app never downloads models or uploads your videos.

## Palette Adaptation

When you build a picture from original-color sprites, the result can only contain colors those sprites have. Select a target and a sprite set, then press **Optimize for selected sprite set** under the target library.

The app extracts a palette from the visible sprite pixels and prepares a new target: light edge-preserving smoothing, a little extra contrast, mild desaturation and mapping to the palette with optional dithering. **Palette adaptation controls** set the strength of each step, the palette size (1–256) and the dither cell size. The result is saved as a new `_palette.png` / `_palette.mp4` next to the original, which is never overwritten.

Videos keep one palette and one fixed pattern for the whole clip, so there is no flicker. Intermediate videos are encoded losslessly with `libx264rgb`, so FFmpeg must include that encoder, and the files can be large.

If fine dithering hurts the result, try larger cells, turn off color mixing or lower the strength. Adaptation can't create dark or light shades your sprites don't have.

## Brightness Priority

**General → Prioritize brightness (BT.709 luma / color)** switches candidate scoring from plain RGB error to a weighted Y′CbCr error. Details and light/shadow then matter more than exact hue. It works in all image and video modes. The default weights are 6:1:1 (luma : Cb : Cr), and the luma weight is editable.

<details>
<summary>Formula</summary>

`error = 3 × (w·ΔY′² + ΔCb² + ΔCr²) / (w + 2)`, where
`Y′ = 0.2126R′ + 0.7152G′ + 0.0722B′`, `Cb = (B′ − Y′) / 1.8556`, `Cr = (R′ − Y′) / 1.5748`.

Coefficients follow [BT.709](https://www.itu.int/rec/R-REC-BT.709). The normalization keeps the error comparable to RGB, so thresholds like `min_improvement` stay meaningful. The 6:1:1 weighting is a tuning choice for this task, not a vision standard.

</details>

## 🍎 Bad Apple Addon

A bonus tool for the classic "Bad Apple!!" effect: rebuild a black-and-white silhouette video out of its **own** frames. `prepare_bad_apple` samples frames, removes the solid background, draws a clean outline around each silhouette and saves them as colored brushes in `raw_shapes/`.

```bash
cargo run --release --example prepare_bad_apple -- "path/to/clip.mp4"
```

(or run `prepare_bad_apple.bat` with your video path)

| Option | Description |
|--------|-------------|
| `--interval <sec>` | Seconds between sampled frames (default `2.0`). |
| `--out <folder>` | Output folder for the shape PNGs (default `raw_shapes`). |
| `--max-size <px>` | Longest side of each shape in pixels (default `512`). |
| `--mono <0..1>` | Drop near-blank frames where one near-pure color covers ≥ this fraction (default `0.97`). |
| `--bg-tol <0..255>` | How far from pure white/black still counts as background (default `40`). |
| `--outline <px>` | Thickness of the outline drawn in the background color (default `3`). |

Then put the clip into `input_media/`, enable `use_original_colors` and generate.

## Minecraft Living-Mob Playback

The **TeekasFigure Mob Tools** Fabric mod (Minecraft Java 1.21.1, Java 21, no Fabric API needed) turns results into scenes made of real mobs.

1. **Capture brushes.** In a singleplayer world press Escape → **TeekasFigure: mob brushes** → **Export top view** or **Export facing forward**. The mod renders every default vanilla mob model to a transparent 512×512 PNG.
2. **Import.** In the app open **Minecraft / scene export → Import mobs from Minecraft…** and select the folder with `mob_mapping.json`. The app creates a separate library and configures itself: original colors and scene export on; HSV changes, opacity, stretching, recoloring and fade interpolation off. Rotation stays on for top view and off for front view.
3. **Generate** any image or video as usual. The result includes `output/scene_.../scene.json`.
4. **Play.** In the game open **TeekasFigure: mob video**, choose `scene.json` and press **Start scene**. The mod builds the picture from real mobs high above the world, with an orthographic camera. **Stop and restore** removes the scene and returns the player to where they were. World blocks are never changed.

See the [mod README](minecraft/mob-exporter/README.md) for details and limitations.

---

## Folder Structure

```text
TeekasFigure/
├── input_media/       ← Target images and videos ("What to recreate")
├── raw_shapes/        ← Original shape images ("Building blocks")
├── input_shapes/      ← Prepared grayscale brushes (generated, don't edit by hand)
├── frame_sets/        ← Shape sets created from videos
├── output/            ← Results: PNG, MP4, optional GIF and scene packages
├── presets/           ← Saved settings presets
├── settings.toml      ← Configuration (created on first run, updated from the UI)
└── TeekasFigure.exe
```

## Configuration (`settings.toml`)

All parameters are written to `settings.toml` on first launch. The easiest way to change them is the Settings screen, but you can also edit the file.

#### ⚙️ Basic Settings
| Parameter | Description |
|-----------|-------------|
| **`batch_size`** | Initial population size per search (1–4096). |
| **`max_shapes`** | Maximum number of shapes on the canvas. In video mode it is the target population size. |
| **`mutations_per_frame`** | Target placements per worker cycle; the UI refreshes independently. |
| **`max_texture_size`** | Maximum resolution the input is downscaled to. |
| **`vram_budget_mb`** | Video memory limit for the shape texture array (MB). |
| **`scale_min` / `scale_max`** | Minimum and starting maximum shape size. The maximum shrinks as the canvas fills. |
| **`shape_resolution`** | Resolution of loaded shapes and of the built-in conversion (default 128). |
| **`perceptual_scoring`** | Use BT.709 brightness-priority scoring instead of RGB (`false` by default). |
| **`luma_weight`** | Relative luma weight for brightness-priority scoring (default `6.0`). |

#### 🎬 Video & Animation
| Parameter | Description |
|-----------|-------------|
| **`target_fps`** | Framerate the input video is resampled to before processing. Output FPS is higher with interpolation. |
| **`scene_change_tolerance`** | Shape "death" threshold (-10.0 to 10.0). A shape that makes its region worse than this per pixel dies and is reborn. **Negative values** are strict: a shape must actively improve its area to survive. |
| **`interpolation_steps`** | Interpolated frames between keyframes. `0` disables. Output FPS = `target_fps × (steps + 1)`. |
| **`video_recolor`** | `false` (default) — a shape keeps its color for the whole video. `true` — shapes recolor to each new frame (can wash out detail). |
| **`mutations_per_shape`** | Local move/scale attempts a shape gets to adapt to a new frame. |
| **`displacement_weight`** | Penalty for moving a shape; keeps shapes in place and prevents jitter. |
| **`preserve_audio`** | Keep the source audio track in the rendered MP4 (`true` by default). |

#### 🎞️ Progress GIF (image mode)
| Parameter | Description |
|-----------|-------------|
| **`save_progress_gif`** | Save `<name>_process.gif` next to the result (`false` by default). |
| **`gif_fps`** | GIF playback speed (1–50). |
| **`gif_frames`** | Approximate number of captured frames, spread across the process (2–2000). |
| **`gif_max_width`** | Maximum GIF width in pixels; larger canvases are downscaled (16–2048). |

#### 🧬 Evolution
| Parameter | Description |
|-----------|-------------|
| **`evolve_opacity`** | Let the algorithm choose opacity (`true`) or always draw fully opaque (`false`). |
| **`evolve_rotation`** | Allow rotation (`true` by default). Disable for side-view sprites that must stay upright. |
| **`use_original_colors`** | Keep the shapes' original colors (`true`) instead of tinting grayscale brushes (`false`, default). |
| **`evolve_non_uniform_scale`** | Allow independent X/Y scaling so shapes can stretch and squash. Default `false`. |
| **`evolve_hue`** / **`evolve_saturation`** / **`evolve_brightness`** | Original-color mode only: let each shape shift its hue, saturation or brightness to match the target. Default `false`. |
| **`export_scene`** | Export brushes and every output frame's shapes as a JSON scene package (`false` by default). |
| **`num_generations`** | Evolutionary generations per shape placement. |
| **`min_improvement`** | Acceptance threshold for the change in error (negative = improvement). |
| **`use_min_improvement`** | Enforce `min_improvement`. **Disable it when using `diversity_mode`.** |
| **`max_rejections`** | Consecutive failed searches before the artwork is considered finished. |
| **`survival_rate`** | Fraction of top candidates that survive each generation (e.g. 0.10 = 10%). |
| **`children_per_parent`** | Mutated children bred from each survivor. |

#### 🎨 Diversity Mode
| Parameter | Description |
|-----------|-------------|
| **`diversity_mode`** | Penalize overused shapes so the algorithm uses more of your brushes. |
| **`diversity_penalty_increment`** | Penalty added to a shape each time it is placed. |
| **`diversity_decay_enabled`** | Let penalties of other shapes decay over time. |
| **`diversity_decay_amount`** | How much penalty other shapes lose per placement. |

> **🔥 Important:** the diversity penalty distorts the "improvement" score, so **with `diversity_mode = true`, `use_min_improvement` must be `false`**, otherwise every shape is rejected. The Settings screen handles this automatically; set it yourself if you edit `settings.toml` by hand.

## License

MIT — see [LICENSE](LICENSE).
