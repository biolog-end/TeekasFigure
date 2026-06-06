# TeekasFigure — Полное описание проекта

> Документ-карта проекта для быстрой ориентации при будущих правках.
> Дата создания: 2026-06-05. Обновлено: 2026-06-06 (видео: temporal coherence,
> заморозка цвета + `video_recolor`, перерождение через полную эволюцию,
> очистка `frame_*.png`, проверка готовности MP4).
> Crate name (`Cargo.toml`): **`gpu-image-approximator`**, edition 2021.

---

## 1. Что делает проект

**TeekasFigure** — GPU-ускорённый эволюционный аппроксиматор изображений и видео. Программа воссоздаёт целевую картинку (или видео покадрово), последовательно накладывая на холст полупрозрачные геометрические фигуры (PNG-текстуры). Алгоритм:

1. Генерирует **batch_size** случайных кандидатов (позиция, поворот, масштаб, цвет, прозрачность).
2. На GPU параллельно через compute-шейдер считает MSE-улучшение каждого кандидата.
3. Запускает эволюцию: турнирный отбор + мутации + N поколений.
4. Лучшую фигуру композитит на холст через render pipeline с alpha-blending.
5. Повторяет, пока не достигнет `max_shapes` или не сойдётся (`max_rejections`).

Поверх рендера накладывается egui-оверлей со статистикой (FPS, MSE, прогресс, уведомления).

**Видео-режим** использует тот же алгоритм для первого кадра, а далее переносит популяцию фигур на следующий кадр и адаптирует её (temporal coherence). Ключевые принципы текущей реализации (`VideoPipeline::adapt_to_new_frame`):

1. **Наследование популяции.** Канвас не перерисовывается с нуля: фигуры предыдущего кадра переносятся и адаптируются. Размер популяции **заморожен** с первого кадра (никакого ежекадрового дозаполнения climber'ом — оно перекрашивало холст и было удалено).
2. **Адаптация = только геометрия.** Каждая фигура пробует [без изменений] + N локальных мутаций (сдвиг/поворот/масштаб). **Цвет по умолчанию НЕ меняется** — фигура хранит свой цвет с момента размещения. Выбор движения = `min(дельта MSE + штраф за движение)`, штраф считается от текущей позиции этого кадра.
3. **Жизнь/смерть + перерождение.** Фигура живёт, если средняя попиксельная дельта `<= scene_change_tolerance`. Иначе она умирает (плавно гаснет через интерполяцию), а на её месте **рождается новая фигура через полный цикл эволюции** (как на первом кадре), что сохраняет плотность.
4. **Опциональная перекраска.** Настройка `video_recolor` (по умолчанию `false`) возвращает старое поведение: фигуры пересэмплят цвет нового кадра при адаптации.
5. **Линейная интерполяция** между ключевыми кадрами (`interpolation_steps`): новые фигуры плавно проявляются, умершие — гаснут.

---

## 2. Структура каталогов

```
videofigure/
├── Cargo.toml                  # crate gpu-image-approximator
├── Cargo.lock
├── README.md / README_ru.md    # пользовательская документация
├── prepare_shapes.bat          # обёртка над cargo run --example prepare_shapes
├── input_media/                # ← пользователь кладёт PNG/JPG/BMP/MP4
├── input_shapes/               # ← готовые 128x128 grayscale+alpha кисти
├── output/                     # ← результаты (PNG, MP4, snapshot_*)
├── examples/
│   ├── generate_shapes.rs      # генератор тестовых circle.png/square.png
│   └── prepare_shapes.rs       # утилита: raw_shapes/ → input_shapes/
├── tests/                      # unit/integration/property — пока пусто
└── src/
    ├── main.rs                 # точка входа, инициализация, polling медиа
    ├── app.rs                  # ApplicationHandler (winit 0.30), event loop, FPS, рендер кадра
    ├── error.rs                # AppError (thiserror)
    ├── settings.rs             # Settings (TOML), валидация, defaults
    ├── types.rs                # CandidateParams, PlacedShape, GenerationState, StepResult, ShapeLayer, EvalUniforms
    ├── algorithm/
    │   ├── mod.rs
    │   ├── candidate_generator.rs   # CandidateGenerator: батчи кандидатов, sample_color_at, adaptive scale
    │   ├── hill_climber.rs          # HillClimber: эволюция, diversity_mode, tournament, мутации
    │   ├── video_evolution.rs       # VideoEvolution: temporal coherence (старая версия)
    │   └── video_pipeline.rs        # VideoPipeline: новый pipeline для видео (используется в App)
    ├── gpu/
    │   ├── mod.rs
    │   ├── context.rs               # GpuContext: device/queue, текстуры, буферы, dispatch, readback
    │   ├── pipelines.rs             # MsePipeline (compute), CompositePipeline (render)
    │   └── shaders/
    │       ├── blit.wgsl            # копирование canvas → surface (с конверсией формата)
    │       ├── composite.wgsl       # alpha-blend одной фигуры на холст
    │       └── mse_eval.wgsl        # параллельное вычисление дельты MSE по кандидатам
    ├── io/
    │   ├── mod.rs
    │   ├── media_loader.rs          # find_first_supported_file, load_media, ensure_directories, get_base_dir
    │   ├── output.rs                # save_canvas_png, completion_filename, snapshot_filename
    │   ├── shape_preprocessor.rs    # load_and_preprocess (PNG → grayscale+alpha), check_vram_budget
    │   └── video.rs                 # VideoProcessor (FFmpeg decode/encode pipes)
    └── overlay/
        ├── mod.rs
        └── state.rs                 # OverlayState (egui), Notification
```

---

## 3. Зависимости (Cargo.toml)

| Библиотека | Версия | Назначение |
|-----------|--------|------------|
| `wgpu` | 23.0.1 | GPU API (Vulkan/DX12/Metal) |
| `winit` | 0.30.8 | окно и event loop (новый ApplicationHandler) |
| `egui` / `egui-wgpu` / `egui-winit` | 0.30.0 | оверлей со статистикой |
| `image` | 0.25.5 | загрузка/сохранение PNG/JPG/BMP |
| `toml` + `serde` + `serde_derive` | — | парсинг settings.toml |
| `bytemuck` | 1.21.0 | Pod/Zeroable для GPU-буферов |
| `rand` (`small_rng`) | 0.8.5 | быстрый PRNG в горячих циклах |
| `thiserror` | 2.0.11 | типизированные ошибки `AppError` |
| `chrono` | 0.4.39 | timestamp для snapshot имени |
| `pollster` | 0.4.0 | block_on для async wgpu |
| `env_logger` + `log` | — | логирование |
| `proptest` + `tempfile` (dev) | — | тестирование |

⚠️ **FFmpeg должен быть в PATH** — для видео используется через `std::process::Command` (модуль `io::video` и фрагменты в `main.rs`/`app.rs`).

---

## 4. Поток выполнения (high-level)

```
main()
 └─ run()
     1. get_base_dir()                             → путь рядом с .exe
     2. ensure_directories()                       → создаёт input_media/, input_shapes/, output/
     3. Settings::load_or_create() + .validate()   → settings.toml
     4. poll_for_media()                           → ждёт файл в input_media/ (опрос каждые 2 сек)
     5. load_media()                               → MediaType::Image | MediaType::Video
        └─ для видео: FFmpeg извлекает первый кадр (raw RGBA или PNG fallback)
     6. shape_preprocessor::load_and_preprocess()  → Vec<ShapeLayer>
        + check_vram_budget()
     7. EventLoop + Window (winit 0.30)
     8. wgpu::Instance + Surface + GpuContext::new_with_surface()
     9. egui_ctx + egui_winit::State + egui_wgpu::Renderer
    10. HillClimber::new(), CandidateGenerator::new(), OverlayState::new()
    11. App::new(...)
        └─ если видео: создаёт VideoProcessor (decoder pipe)
    12. app.run(event_loop) ──── входит в ApplicationHandler цикл
```

### Внутри event loop (`app.rs::AppHandler`):

- `WindowEvent::RedrawRequested`:
  - `app.run_generation_step()` — крутит `HillClimber::step` пока не наберёт `mutations_per_frame` принятых кандидатов (или предохранитель `mutations_per_frame * 100` попыток).
  - `app.render_frame()` — blit холста на surface + egui-оверлей сверху.
  - `request_redraw()` для следующего кадра (60 FPS).
- `WindowEvent::KeyboardInput`:
  - `Space` — переключает `GenerationState::Running ↔ Paused`.
  - `S` — `io::output::save_canvas_png` со snapshot-именем.
  - `Esc` — выход.
- `WindowEvent::Resized` — реконфигурирует surface.
- При `StepResult::Completed` для видео — сохраняет PNG кадра, грузит следующий через `VideoProcessor::next_frame()`, запускает `VideoPipeline::adapt_to_new_frame()` (адаптация геометрии + перерождение мёртвых) + `rebuild_canvas()`. **Для кадров > 0 climber НЕ дозаполняет популяцию** — он помечается «заполнен» (`placed_shapes = max_shapes`), чтобы его следующий `step` сразу вернул `Completed` и просто перевёл на следующий кадр (раньше дозаполнение перекрашивало холст новыми фигурами каждый кадр — баг). По окончании декодера — собирает MP4 через `ffmpeg`, причём успех объявляется только после проверки, что файл существует и непустой.

---

## 5. Детальное описание модулей

### 5.1 `src/types.rs` — общие типы

| Тип | Описание |
|-----|----------|
| `CandidateParams` | `#[repr(C)] bytemuck::Pod`, 48 байт. Поля: `shape_index, x, y, rotation, scale, r, g, b, alpha, _padding[3]`. Загружается в storage- и uniform-буферы. |
| `PlacedShape` | `params: CandidateParams` + `prev_centroid: (f32, f32)` для temporal coherence в `VideoEvolution`. |
| `GenerationState` | Enum: `Running`, `Paused`, `Completed`. |
| `StepResult` | Enum: `Accepted(CandidateParams)`, `Rejected`, `Completed`, `Error(String)`. |
| `ShapeLayer` | `pixels: Vec<u8>` (RGBA8, `shape_resolution²`). |
| `EvalUniforms` | `#[repr(C)]`. Поля: `canvas_width, canvas_height, num_candidates, shape_resolution, displacement_weight`. |

### 5.2 `src/error.rs` — ошибки

`AppError` (через `thiserror`):
- `GpuInit(String)`
- `SettingsValidation { name, value, range }`
- `SettingsParse { location, message }`
- `NoShapes { path }`
- `VramBudget { required_mb, budget_mb }`
- `NoMedia { path }`
- `ComputeTimeout { timeout_ms }`
- `SaveFailed { reason }`
- `Ffmpeg(String)`

### 5.3 `src/settings.rs` — конфиг

- `Settings` — `Serialize + Deserialize + Default`, `#[serde(default)]` (частичный TOML использует defaults).
- `Settings::load_or_create(path)` — если файла нет, пишет `DEFAULT_SETTINGS_TOML` (с комментариями) и возвращает defaults. Парсинг ошибок маппится в `AppError::SettingsParse` с line:col.
- `Settings::validate()` — проверяет диапазоны через `validate_u32` / `validate_f32` (NaN отклоняется).
- ⚠️ В коде есть неиспользуемый `validate_u8` (мёртвый код, без warn-suppress).

**Ключевые параметры** (см. README для полного списка): `batch_size`, `max_shapes`, `mutations_per_frame`, `max_texture_size`, `vram_budget_mb`, `scale_min/max`, `shape_resolution`, `mutations_per_shape`, `displacement_weight`, `scene_change_tolerance` (видео: порог гибели по средней попиксельной дельте; **диапазон −10.0…10.0**, отрицательное значение требует от фигуры активно улучшать свою зону, чтобы выжить), `interpolation_steps` (видео: число интерполированных кадров между ключевыми, 0–20), `video_recolor` (видео: разрешить ли фигурам перекрашиваться под новый кадр; по умолчанию `false` — цвет сохраняется, меняется только геометрия), `target_fps`, `evolve_opacity`, `num_generations`, `min_improvement`, `use_min_improvement`, `max_rejections`, `survival_rate`, `children_per_parent`, `diversity_mode`, `diversity_penalty_increment`, `diversity_decay_enabled`, `diversity_decay_amount`.

### 5.4 `src/main.rs`

- Инициализирует логгер, вызывает `run()`, на ошибке выводит и `exit(1)`.
- `run()`: см. поток выше.
- `poll_for_media()` — блокирует поток, опрашивая `input_media/` каждые 2 секунды.

### 5.5 `src/app.rs` — главный цикл

- `compute_window_size(target, display) -> (u32, u32)` — масштабирует под 90% экрана, сохраняет aspect ratio, минимум 1×1. Покрыто 8 unit-тестами.
- `pub struct App` — владеет `gpu`, `climber`, `generator`, `overlay`, `settings`, `window` (Arc), `surface`, egui-состоянием, `video_pipeline`, `video_decoder`, frame timing.
- `App::new(...)` — конфигурирует surface (`gpu.create_surface_config`).
- `App::run(self, event_loop)` — оборачивает в `AppHandler` и вызывает `event_loop.run_app`.
- `compute_fps()` — скользящее среднее по последним 60 кадрам.
- `run_generation_step()` — крутит `climber.step` до `mutations_per_frame` accepted (предохранитель = `mutations_per_frame * 100`); для accepted записывает в `video_pipeline` (если есть).
- `auto_save_on_completion()` — для image-режима один раз сохраняет результат и меняет title окна. Для видео делегирует в `handle_video_frame_complete()`.
- `handle_video_frame_complete()` — рендерит интерполированные кадры (если `interpolation_steps > 0`), сохраняет `frame_NNNNN.png`, грузит следующий кадр, обновляет target-текстуру, пересоздаёт `CandidateGenerator` под новые пиксели, вызывает `video_pipeline.adapt_to_new_frame()` (адаптация + перерождение) + `rebuild_canvas()`. **Climber для кадров > 0 ставится в «заполнен» (`placed_shapes = max_shapes`)**, чтобы следующий redraw сразу завершил кадр и перешёл к следующему — без перекрашивающего дозаполнения. По окончании декодера: чистит остаточные `frame_*.png`? (нет — чистка делается в `main.rs` ДО старта), собирает MP4 через `ffmpeg`, и **объявляет успех только после проверки**, что выходной файл существует и непустой (явный лог `VIDEO READY: ...` в stdout). Заголовок окна отражает реальный статус (готово / ошибка кодирования).
- `render_frame()` — blit canvas на surface, egui run/tessellate/render, `submit + present`. Использует `forget_lifetime()` для render pass из-за требований egui_wgpu 0.30.
- `AppHandler` — реализует `ApplicationHandler` (winit 0.30). События сначала уходят в `egui_state.on_window_event`; если egui «съел» — выходим. Иначе — собственный обработчик.

### 5.6 `src/algorithm/`

#### `candidate_generator.rs`

`CandidateGenerator { rng: SmallRng, settings, target_pixels, target_width, target_height }`:
- `generate_batch(batch_size, placed_shapes, canvas_size, num_shapes)` — рандомизирует все поля; цвет берётся из `sample_color_at` (3×3 average).
- `compute_adaptive_scale(placed_shapes)` — линейно `scale_max → scale_min` по мере прогресса. Покрыто тестами на границах.
- `sample_color_at(x, y) -> (f32, f32, f32)` — нормализованный 3×3 средний цвет; используется и в hill climber, и в video pipeline для подбора цвета по позиции.
- `rng_mut()` — даёт доступ к RNG из `VideoEvolution`.

#### `hill_climber.rs`

`HillClimber { current_mse, placed_shapes, state, rng, consecutive_rejections, shape_penalties: Vec<f32> }`:
- Алгоритм одного `step`:
  1. Если `consecutive_rejections >= max_rejections` или `placed_shapes >= max_shapes` → `Completed`.
  2. `generator.generate_batch(...)` → начальная популяция.
  3. Пересамплит цвет по позиции для всей популяции.
  4. Цикл `num_generations`: dispatch MSE → `read_fitness_scores` → применяет penalty (если diversity_mode) → выбирает топ `survival_rate` → `mutate(parent, ...)` × `children_per_parent`.
  5. Финальный dispatch + поиск лучшего.
  6. Если `!use_min_improvement || best_score < min_improvement` — `composite_shape`, обновляет penalties, `Accepted(winner)`. Иначе `Rejected`.
- `mutate(parent, ...)` — ±10% позиция, ±0.5 рад поворот, ×0.7..1.3 scale (clamp 0.02..20.0), ±0.2 alpha (или фикс 1.0), цвет ресемплится в новой точке.
- Свободная функция `select_best(scores)` — поиск минимума с tie-break по индексу.
- Diversity penalty: к фитнесу прибавляется `shape_penalties[idx]`; при принятии — выбранной фигуре +`diversity_penalty_increment`, остальным `-diversity_decay_amount` (если `diversity_decay_enabled`), clamp ≥ 0.
- **Переиспользуемые свободные функции** (общие для climber и видео-перерождения, без дублирования логики):
  - `mutate_candidate(parent, canvas_size, generator, evolve_opacity, rng)` — стандартная (широкая) мутация; `HillClimber::mutate` теперь тонкая обёртка над ней.
  - `top_n_by_score(candidates, scores, n)` — отбор лучших; `select_top_n` делегирует сюда.
  - `evolve_best_candidate(gpu, generator, settings, placed_shapes, rng) -> Option<(CandidateParams, f32)>` — **полный цикл эволюции** (batch_size случайных → `num_generations` × отбор `survival_rate` + `children_per_parent` детей), возвращает лучшего кандидата и его фитнес. НЕ композитит, НЕ применяет порог `min_improvement`, игнорирует diversity. Используется видео-перерождением, чтобы новая фигура эволюционировала так же, как на первом кадре. `placed_shapes=0` даёт полный диапазон масштаба.

#### `video_evolution.rs` (старый pipeline)

`VideoEvolution { shape_list: Vec<PlacedShape> }`:
- `record_shape(params)` — добавляет с `prev_centroid = (x, y)`.
- `mutate_for_new_frame(...)` — для каждой фигуры baseline-фитнес + до `mutations_per_shape` мутаций; если ни одна не лучше → помечает на удаление. После — обновляет `prev_centroid`.
- `displacement_penalty(prev, current, weight)` — `weight × √((dx)² + (dy)²)`. Полностью покрыта unit-тестами.
- `vacant_slots(max_shapes)` — сколько новых фигур можно ещё добавить.
- `mutate_shape(...)` (приватная) — ±5% поз, ±0.3 рад, ±20% scale, ±0.1 RGB/alpha.

#### `video_pipeline.rs` (актуальный pipeline, используется в `App`)

`PlacedShapeRecord { id, params, prev_params, just_born }` + `VideoPipeline { shapes, dying, frame_index, next_id, rng }`:
- `record_placed_shape(params)` — на первом кадре `prev_params == params`; на последующих новые фигуры помечаются `just_born` для fade-in.
- `adapt_to_new_frame(gpu, generator, settings) -> u32` — главная функция адаптации. Возвращает число фигур, умерших **без** немедленной замены. Алгоритм:
  1. `frame_index += 1`, очистка `dying`, сброс `just_born`, `clear_canvas`.
  2. **Проход 1** — по всем фигурам в хронологическом порядке (без сортировки): набор кандидатов = [без изменений] + N локальных мутаций; **если `video_recolor`** — добавляется ещё кандидат «перекраска на месте». Выбор движения по `min(дельта + movement_penalty)`, штраф считается **от текущей позиции** (`current`, не от `prev_params` — это был off-by-one баг). Жизнь, если `mean_delta = chosen_raw / footprint_area <= scene_change_tolerance` → композит + survivor; иначе фигура добавляется в `dying` (fade-out) и считается мёртвой.
  3. **Проход 2 (перерождение)** — для каждой мёртвой фигуры вызывается `evolve_best_candidate(...)` (полный цикл эволюции, полный диапазон масштаба), и если результат проходит тот же порог `scene_change_tolerance`, он композитится и добавляется как новая `just_born` фигура. Так плотность сохраняется. Ранний выход, если даже лучший кандидат не проходит порог.
- `create_local_mutation(parent, canvas_size, generator, settings)` — мелкая мутация: ±3% позиция, ±0.15 рад, ×0.9..1.1 масштаб, ±0.05 alpha (если `evolve_opacity`). **Цвет:** по умолчанию сохраняется родительский; пересэмплится из кадра только при `video_recolor = true`. Clamps по `settings.scale_min/scale_max`.
- `rebuild_canvas(gpu)` — `clear_canvas` + `composite_shape` для всех `shapes` по порядку.
- `render_interpolated_frame(gpu, t)` — линейная интерполяция `prev_params → params` для всех фигур; `dying` гаснут (alpha→0), `just_born` проявляются (alpha 0→full). Без GPU-оценки.
- `movement_penalty(prev, current, settings)` — `pos_dist + scale_ratio*50 + rot_diff*10`, умножается на `displacement_weight`.
- Свободные функции: `recolor_in_place(shape, generator)` — копия фигуры с цветом из кадра в её центре (используется только при `video_recolor`); `footprint_area(shape, canvas_size, shape_resolution)` — площадь следа `(scale × shape_resolution)²`, обрезанная по канвасу, для нормировки дельты.

⚠️ `video_evolution.rs` существует параллельно, но в `App` НЕ используется — legacy. Реэкспортируется в `algorithm::mod.rs`.

### 5.7 `src/gpu/`

#### `context.rs` — `GpuContext`

Поля: `device, queue, canvas, canvas_view, target, target_view, shape_array, shape_array_view, candidate_buffer, fitness_buffer, fitness_staging, uniform_buffer, canvas_size, batch_size, num_shapes, shape_resolution, mse_pipeline, composite_pipeline, mse_bind_group, composite_sampler, composite_uniform_buffer, surface_format, blit_pipeline, blit_bind_group_layout, blit_sampler`.

Два конструктора:
- `new(target_data, target_size, shapes, settings)` — без surface (headless). Создаёт девайс через `init_device`.
- `new_with_surface(instance, surface, ...)` — для оконного режима, использует `init_device_with_surface` (адаптер совместимый с surface). Включает первоначальную очистку canvas в тёмно-синий.

Методы:
- `dpatch_mse_evaluation(candidates: &[CandidateParams])` — `write_buffer(candidate_buffer)` + `write_buffer(uniform_buffer)` + compute pass с `dispatch_workgroups(num_candidates, 1, 1)`. Workgroup size = 256.
- `read_fitness_scores() -> Vec<f32>` — copy `fitness_buffer → fitness_staging`, `map_async + poll(Wait)`, `bytemuck::cast_slice`, `unmap`. Блокирующая операция через `pollster`.
- `composite_shape(candidate)` — пишет 1 candidate в uniform-буфер, рендер pass на canvas (`LoadOp::Load + alpha blend`), `draw(0..6)`.
- `clear_canvas()` — render pass с `LoadOp::Clear(BLACK)`.
- `blit_canvas_to_surface(encoder, surface_view)` — fullscreen quad через `blit.wgsl`, конвертирует Rgba8Unorm canvas → surface format.
- `create_surface_config(width, height, format)` — `RENDER_ATTACHMENT`, `Fifo`, `desired_maximum_frame_latency: 2`.

Текстуры:
- `canvas`: `Rgba8Unorm`, `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`.
- `target`: `Rgba8Unorm`, `TEXTURE_BINDING | COPY_DST`.
- `shape_array`: `Rgba8Unorm`, 2D-array, по слою на фигуру.

Буферы:
- `candidate_buffer`: STORAGE + COPY_DST, 48 × `batch_size` байт.
- `fitness_buffer`: STORAGE + COPY_SRC, 4 × `batch_size`.
- `fitness_staging`: MAP_READ + COPY_DST, 4 × `batch_size`.
- `uniform_buffer`: UNIFORM + COPY_DST, `sizeof(EvalUniforms)`.
- `composite_uniform_buffer`: UNIFORM + COPY_DST, `sizeof(CandidateParams)`.

#### `pipelines.rs`

- `MsePipeline::new(device)` — bind group layout (canvas/target/shapes/candidates/fitness/uniforms) + compute pipeline, entry `eval_mse`.
- `CompositePipeline::new(device)` — render pipeline с alpha blending (`SrcAlpha + (1-SrcAlpha)`), entry `vs_main`/`fs_main`. Bind group layout: shapes texture array, sampler, uniform CandidateParams (`VERTEX_FRAGMENT`).

#### Шейдеры (WGSL)

- **`mse_eval.wgsl`** (workgroup 256): один workgroup на кандидата. Считает консервативный AABB вокруг повёрнутого квадрата `shape_size * (|cos|+|sin|)`. Каждый поток обрабатывает пиксели по шагу 256 в линейном индексе. Для каждого пикселя:
  - `current_diff = canvas - target`
  - сэмпл шейпа в shape-local координатах (через inverse rotation)
  - `composited = shape_color * shape_alpha + canvas * (1 - shape_alpha)`
  - `thread_error += new_error - current_error` (отрицательное — улучшение).
  - Параллельная редукция в `partial_sums[256]` через `workgroupBarrier`.
  - Поток 0 пишет финальный fitness в `fitness[candidate_idx]`.
- **`composite.wgsl`** — full-screen quad из 6 вершин по `vertex_index`. Фрагмент-шейдер вычисляет shape UV через inverse rotation, выходит за пределы → `vec4(0)`, иначе тинт по `(r,g,b)` × luminance, alpha = `shape.a * candidate.alpha`.
- **`blit.wgsl`** — простой sample texture → screen.

### 5.8 `src/io/`

#### `media_loader.rs`
- `MediaType::Image(DynamicImage)` / `MediaType::Video(PathBuf)`.
- Поддержка: `png, jpg, jpeg, bmp` (изображения) + `mp4` (видео).
- `find_first_supported_file` — альфавитная сортировка (case-insensitive) по имени файла.
- `ensure_directories` — создаёт `input_media`, `input_shapes`, `output`.
- `get_base_dir` — каталог exe (fallback на `current_dir`).

#### `output.rs`
- `completion_filename(source) -> "<stem>_result.png"`.
- `snapshot_filename() -> "snapshot_YYYYMMDD_HHMMSS.png"` (chrono).
- `save_canvas_png(gpu, output_folder, filename)` — `copy_texture_to_buffer` со 256-byte row alignment, `map_async`, удаление паддинга, `image::RgbaImage::from_raw + save`.
- `clean_frame_sequence(output_folder) -> usize` — удаляет остаточные `frame_<цифры>.png` от предыдущего прогона (вызывается в `main.rs` ДО старта видео). Без этого FFmpeg по маске `frame_%05d.png` подхватывал бы «хвост» от более длинного прошлого видео. `*_result.*` и `snapshot_*.png` не трогаются. Хелпер `is_frame_sequence_name` распознаёт паттерн.
- `align_to(value, alignment)` — power-of-2 align up.

#### `shape_preprocessor.rs`
- `MAX_SHAPES = 256` (хард-лимит на массив текстур).
- `load_and_preprocess(folder, shape_resolution)` — собирает `*.png`, сортирует, режет до 256, для каждого — `image::open`, `resize_exact(Triangle)`, `convert_to_grayscale_alpha` (luminance ITU-R BT.709). Невалидные PNG пропускаются с `warn!`.
- `check_vram_budget(shape_resolution, num_layers, vram_budget_mb)` — округляет вверх до МБ.

#### `video.rs` — `VideoProcessor`
- `probe_video` через `ffprobe` — width/height/r_frame_rate/nb_frames; парсит `30/1` или `29.97`; при отсутствии nb_frames — оценка по duration × fps.
- `spawn_decoder` — `ffmpeg -i ... -vf fps=N -f rawvideo -pix_fmt rgba -` с пайпом stdout.
- `spawn_encoder` — `ffmpeg -y -f rawvideo -pix_fmt rgba -s WxH -r FPS -i - -c:v libx264 -pix_fmt yuv420p OUT`.
- `next_frame()` — читает ровно `width*height*4` байт со stdout декодера, копит частичные read'ы. Возвращает `None` на EOF.
- `encode_frame(data)` — пишет raw RGBA в stdin энкодера.
- `finalize()` — закрывает stdin энкодера, ждёт оба процесса.

⚠️ В текущем `app.rs` используется альтернативный путь: кадры сохраняются как PNG, в конце вызывается отдельный `ffmpeg` для сборки `frame_%05d.png` → MP4. `VideoProcessor::encode_frame/finalize` фактически не вызываются (energy дёшево, но функционал готов).

### 5.9 `src/overlay/state.rs`
- `Notification { message, expires_at: Instant, is_error: bool }`.
- `OverlayState { frame_number, placed_shapes, max_shapes, current_mse, fps, notifications, is_video }`.
- `render(ctx)` — `egui::Area("overlay_stats")` в (10,10), полупрозрачный фон, текст со статистикой; уведомления красным/жёлтым.
- `add_notification` — лимит 5 (старые удаляются), `cleanup_notifications` — снимает истёкшие.

---

## 6. Соглашения и нюансы

- **Координаты**: канвас в пикселях с (0,0) в левом-верхнем; UV шейпа — [0,1]. Вращение применяется как inverse rotation в фрагмент/compute шейдере.
- **Alpha-blending канваса**: `Rgba8Unorm` + `SrcAlpha + (1-SrcAlpha)` для color, `One + (1-SrcAlpha)` для alpha. Стартовая заливка canvas в `new_with_surface` — тёмно-синяя `(0, 0, 0.1, 1)`, в `clear_canvas` — чёрная.
- **GPU-readback** блокирующий через `pollster + poll(Maintain::Wait)`. Так что `mutations_per_frame` напрямую влияет на FPS UI.
- **`dispatch_workgroups(num_candidates, 1, 1)`** — лимит = мин(`maxComputeWorkgroupsPerDimension`, обычно 65535). При очень больших `batch_size` (>65535) надо будет перейти на 2D dispatch.
- **`fitness_buffer` имеет размер `batch_size`**, но `dispatch_mse_evaluation` принимает произвольное число `candidates.len()`. ⚠️ Если `candidates.len() > batch_size` — переполнение буфера. В `hill_climber` популяция растёт через `survivors * (1 + children_per_parent)` — следить, чтобы не превысила `batch_size`.
- **`shape_resolution` нельзя менять без подготовки шейпов** — `prepare_shapes.rs` хардкодит 128.
- **Видео-режим в `app.rs` пересоздаёт `CandidateGenerator` на каждом новом кадре** — берёт новые пиксели для color sampling. Для кадров > 0 `HillClimber` НЕ дозаполняет популяцию (ставится `placed_shapes = max_shapes` → мгновенный `Completed` → переход к следующему кадру). Вся работа по кадру делается в `VideoPipeline::adapt_to_new_frame` (адаптация геометрии + перерождение мёртвых). Популяция заморожена с первого кадра.
- **Сборка: пользователь запускает release.** Изменения видны только после `cargo build --release` (debug-сборка не обновляет `target/release`). При проверке правок видео всегда собирать `--release`.
- **`settings.toml` не дополняется автоматически.** Новые поля пишутся в файл только при его отсутствии (через `DEFAULT_SETTINGS_TOML`). Если файл уже есть, новое поле берётся из serde-default (`#[serde(default)]`), но в самом файле не появится — при добавлении параметра либо сообщить пользователю, либо предложить удалить `settings.toml` для пересоздания.
- **Размер кандидата = 48 байт** жёстко (24 поля как f32 + 3 padding + shape_index u32). Любое изменение требует синхронной правки `CandidateParams` (Rust) + `composite.wgsl` + `mse_eval.wgsl` + размера в `candidate_buffer`.

---

## 7. Известные точки внимания / TODO

- `validate_u8` в `settings.rs` не используется (мёртвый код).
- `VideoProcessor::encode_frame/finalize` не вызываются — финальный MP4 собирается через `frame_*.png` + отдельный `ffmpeg` запуск в `app.rs`. Можно либо упростить (удалить encoder из `VideoProcessor`), либо переключить `app.rs` на pipe-режим.
- `video_evolution.rs` не используется в `App` — пересекается с `video_pipeline.rs`. Перед удалением проверить, нет ли внешних потребителей.
- В `main.rs` ветка extraction первого кадра видео имеет fallback на `image2pipe → PNG via temp file` — может медленно работать на больших видео.
- В тестовых файлах (`tests/unit`, `tests/integration`, `tests/property`) только заголовки-заглушки. Все реальные тесты — `#[cfg(test)] mod tests` рядом с кодом (всего 79 на момент обновления).
- ⚠️ Несоответствие дефолта `mutations_per_frame`: `Settings::default()` = **10**, а `DEFAULT_SETTINGS_TOML` пишет **1**. Тест `test_default_values` ассертит 10 (против `Default`) и проходит. Но свежесозданный `settings.toml` даст 1, а отсутствующий файл → 10. Стоит привести к одному значению.
- `scene_change_tolerance` теперь допускает отрицательные значения (диапазон валидации −10.0…10.0).

---

## 8. Сборка и запуск

```bash
# Релизная сборка (обязательно для производительности)
cargo build --release

# Запуск (.exe рядом — ищет input_media/, input_shapes/, output/, settings.toml)
cargo run --release

# Подготовить кисти из raw_shapes/
cargo run --example prepare_shapes

# Сгенерировать тестовые кисти (circle.png, square.png)
cargo run --example generate_shapes
```

Управление в окне:
- **Space** — pause/resume
- **S** — мгновенный snapshot в `output/`
- **Esc** — выход

---

## 9. Контракт расширения

Если добавляется новое поле в `CandidateParams`:
1. Обновить `repr(C)` и `_padding` в `src/types.rs` (выравнивание до 16/48 байт).
2. Синхронно отразить в `mse_eval.wgsl` и `composite.wgsl` (структура `CandidateParams`).
3. Обновить размер `candidate_buffer` в `gpu/context.rs` (две функции: `new` и `new_with_surface`).
4. При необходимости — добавить инициализацию в `CandidateGenerator::generate_batch` и логику мутации в `hill_climber::mutate` / `video_pipeline::create_local_mutation`.

Если добавляется новый параметр настройки:
1. Поле + default в `Settings` (`src/settings.rs`).
2. Документация в `DEFAULT_SETTINGS_TOML`.
3. Валидация в `Settings::validate`.
4. Обновить README.md и README_ru.md.
