use crate::{error::AppError, settings::Settings};

pub fn population_peak(settings: &Settings, budget: u64) -> Result<u64, AppError> {
    let mut population = u64::from(settings.batch_size);
    let mut peak = population;
    for _ in 0..settings.num_generations {
        if population.saturating_mul(256) > budget {
            return Err(resource_error("Evolution population exceeds the RAM working budget / Популяция превышает бюджет памяти"));
        }
        let survivors = ((population as f32 * settings.survival_rate) as u64)
            .max(1)
            .min(population);
        let next = survivors
            .checked_mul(u64::from(settings.children_per_parent) + 1)
            .ok_or_else(|| {
                resource_error("Population size overflow / Переполнение размера популяции")
            })?;
        peak = peak.max(next);
        if next == population {
            break;
        }
        population = next;
    }
    if peak.saturating_mul(256) > budget {
        return Err(resource_error("Evolution population exceeds the RAM working budget / Популяция превышает бюджет памяти"));
    }
    Ok(peak)
}
pub fn resource_error(message: &str) -> AppError {
    AppError::GpuInit(message.into())
}

pub fn validate(
    settings: &Settings,
    limits: &wgpu::Limits,
    target: (u32, u32),
    layers: u32,
    video: bool,
) -> Result<(), AppError> {
    settings.validate()?;
    let budget = crate::monitor::available_memory() / 3;
    let gpu_budget = (u64::from(settings.vram_budget_mb) * 1024 * 1024).min(budget);
    if target.0 == 0
        || target.1 == 0
        || target.0 > limits.max_texture_dimension_2d
        || target.1 > limits.max_texture_dimension_2d
        || settings.shape_resolution > limits.max_texture_dimension_2d
        || layers == 0
        || layers > limits.max_texture_array_layers
    {
        return Err(resource_error("Texture dimensions / layers exceed this GPU's limits / Разрешение или число текстур превышает возможности GPU"));
    }
    let pixels = u64::from(target.0) * u64::from(target.1);
    let readback_bytes = (u64::from(target.0) * 4).div_ceil(256) * 256 * u64::from(target.1);
    if readback_bytes > limits.max_buffer_size {
        return Err(resource_error(
            "Canvas readback exceeds GPU buffer limits / Холст превышает размер буфера GPU",
        ));
    }
    if pixels > i32::MAX as u64 {
        return Err(resource_error(
            "Canvas exceeds shader indexing limits / Холст превышает пределы индексации шейдера",
        ));
    }
    let shape_bytes = u64::from(settings.shape_resolution)
        .saturating_pow(2)
        .saturating_mul(4)
        .saturating_mul(u64::from(layers));
    let gpu_bytes = shape_bytes.saturating_add(pixels.saturating_mul(12));
    if gpu_bytes > gpu_budget {
        return Err(resource_error(&format!("GPU working set needs {} MiB; available safety budget {} MiB (settings + free RAM) / Недостаточно бюджета памяти", gpu_bytes / 1048576, gpu_budget / 1048576)));
    }
    let mut ram = shape_bytes.saturating_add(pixels.saturating_mul(16));
    ram = ram.saturating_add(population_peak(settings, budget)?.saturating_mul(256));
    if video {
        ram = ram
            .saturating_add(u64::from(settings.max_shapes).saturating_mul(512))
            .saturating_add(u64::from(settings.mutations_per_shape).saturating_mul(128));
    }
    if !video && settings.save_progress_gif {
        let ratio = (settings.gif_max_width as f64 / target.0 as f64).min(1.0);
        let gif_pixels =
            (target.0 as f64 * ratio).ceil() as u64 * (target.1 as f64 * ratio).ceil() as u64;
        ram = ram.saturating_add(gif_pixels.saturating_mul(4).saturating_mul(
            u64::from(settings.gif_frames).min(u64::from(settings.max_shapes)) + 2,
        ));
    }
    if ram > budget {
        return Err(resource_error(&format!("Estimated RAM {} MiB exceeds safe working budget {} MiB / Расчётная память превышает безопасный бюджет", ram / 1048576, budget / 1048576)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn millions_of_stable_generations_need_constant_memory() {
        let s = Settings {
            num_generations: 5_000_000,
            ..Settings::default()
        };
        assert_eq!(population_peak(&s, 64 * 1024 * 1024).unwrap(), 1000);
    }
    #[test]
    fn explosive_population_is_rejected_before_allocation() {
        let s = Settings {
            children_per_parent: u32::MAX,
            num_generations: u32::MAX,
            ..Settings::default()
        };
        assert!(population_peak(&s, 1024 * 1024 * 1024).is_err());
    }
}
