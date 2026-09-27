use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

#[derive(Default)]
pub struct WorkControl {
    pub paused: AtomicBool,
    pub cancelled: AtomicBool,
    pub snapshot: AtomicBool,
    pub evaluated: AtomicU64,
    pub evaluation_ns: AtomicU64,
    pub generations: AtomicU64,
    pub search_generation: AtomicU64,
    pub search_total: AtomicU64,
    pub placed: AtomicU64,
    pub rejections: AtomicU64,
    pub best_score: std::sync::atomic::AtomicU32,
    checkpoints: AtomicU64,
}
impl WorkControl {
    pub fn begin_search(&self, total: u32) {
        self.search_generation.store(0, Ordering::Relaxed);
        self.search_total
            .store(u64::from(total) + 1, Ordering::Relaxed);
        self.best_score
            .store(f32::INFINITY.to_bits(), Ordering::Relaxed);
    }
    pub fn searched(&self, best: f32) {
        self.search_generation.fetch_add(1, Ordering::Relaxed);
        self.best_score.store(best.to_bits(), Ordering::Relaxed);
    }
    /// Called between bounded compute submissions, including inside long video loops.
    pub fn checkpoint(&self) -> bool {
        while self.paused.load(Ordering::Relaxed) && !self.cancelled.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.checkpoints.fetch_add(1, Ordering::Relaxed) % 128 == 0
            && crate::monitor::available_memory() < 256 * 1024 * 1024
        {
            panic!("Generation stopped: low RAM / Генерация остановлена: мало свободной памяти");
        }
        !self.cancelled.load(Ordering::Relaxed)
    }
}
