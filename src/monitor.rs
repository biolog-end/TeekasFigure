//! Low-frequency OS counters. Missing driver counters are reported as unavailable.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Sample {
    pub cpu_percent: Option<f64>,
    pub ram_bytes: Option<u64>,
    pub gpu_percent: Option<f64>,
    pub gpu_engine: String,
    pub gpu_memory_bytes: Option<u64>,
}
pub struct Monitor {
    sample: Arc<Mutex<Sample>>,
    stop: Arc<AtomicBool>,
}
impl Monitor {
    pub fn new() -> Self {
        let sample = Arc::new(Mutex::new(Sample::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (out, shutdown) = (sample.clone(), stop.clone());
        std::thread::spawn(move || {
            #[cfg(windows)]
            let mut counters = windows::Counters::new();
            while !shutdown.load(Ordering::Relaxed) {
                #[cfg(windows)]
                {
                    *out.lock().unwrap() = counters.sample();
                }
                #[cfg(not(windows))]
                {
                    let _ = &out;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        Self { sample, stop }
    }
    pub fn sample(&self) -> Sample {
        self.sample.lock().unwrap().clone()
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn available_memory() -> u64 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::*;
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of_val(&m) as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            return m.ullAvailPhys;
        }
    }
    1024 * 1024 * 1024
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn process_counters_produce_finite_cpu_and_memory() {
        let mut counters = windows::Counters::new();
        counters.sample();
        std::thread::sleep(Duration::from_millis(50));
        let sample = counters.sample();
        assert!(sample.ram_bytes.unwrap() > 0);
        assert!((0.0..=100.0).contains(&sample.cpu_percent.unwrap()));
        assert!(available_memory() > 0);
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{Performance::*, ProcessStatus::*, Threading::*},
    };
    pub struct Counters {
        query: PDH_HQUERY,
        engine: PDH_HCOUNTER,
        memory: PDH_HCOUNTER,
        previous: Option<(Instant, u64)>,
    }
    impl Counters {
        pub fn new() -> Self {
            unsafe {
                let mut query = std::mem::zeroed();
                let mut engine = std::mem::zeroed();
                let mut memory = std::mem::zeroed();
                if PdhOpenQueryW(std::ptr::null(), 0, &mut query) == 0 {
                    for (path, counter) in [
                        (r"\GPU Engine(*)\Utilization Percentage", &mut engine),
                        (r"\GPU Process Memory(*)\Dedicated Usage", &mut memory),
                    ] {
                        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                        if PdhAddEnglishCounterW(query, wide.as_ptr(), 0, counter) != 0 {
                            *counter = std::mem::zeroed();
                        }
                    }
                    PdhCollectQueryData(query);
                }
                Self {
                    query,
                    engine,
                    memory,
                    previous: None,
                }
            }
        }
        pub fn sample(&mut self) -> Sample {
            let mut s = Sample::default();
            unsafe {
                let process = GetCurrentProcess();
                let mut times = [FILETIME {
                    dwLowDateTime: 0,
                    dwHighDateTime: 0,
                }; 4];
                if GetProcessTimes(
                    process,
                    &mut times[0],
                    &mut times[1],
                    &mut times[2],
                    &mut times[3],
                ) != 0
                {
                    let ticks = times[2..]
                        .iter()
                        .map(|t| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64)
                        .sum::<u64>();
                    let now = Instant::now();
                    if let Some((last, prior)) = self.previous {
                        let cores = std::thread::available_parallelism()
                            .map(|n| n.get())
                            .unwrap_or(1);
                        s.cpu_percent = Some(
                            (ticks.saturating_sub(prior) as f64
                                / 1e7
                                / now.duration_since(last).as_secs_f64()
                                / cores as f64
                                * 100.0)
                                .clamp(0.0, 100.0),
                        );
                    }
                    self.previous = Some((now, ticks));
                }
                let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                let size = std::mem::size_of_val(&mem) as u32;
                if K32GetProcessMemoryInfo(process, &mut mem, size) != 0 {
                    s.ram_bytes = Some(mem.WorkingSetSize as u64);
                }
                if !self.query.is_null() && PdhCollectQueryData(self.query) == 0 {
                    let mut engines = std::collections::BTreeMap::<String, f64>::new();
                    for (name, value) in self.values(self.engine) {
                        // Aggregate duplicate contexts on the SAME physical engine only.
                        let key = name
                            .split_once("_luid_")
                            .map(|(_, x)| x)
                            .unwrap_or(&name)
                            .split('#')
                            .next()
                            .unwrap_or(&name)
                            .to_string();
                        *engines.entry(key).or_default() += value;
                    }
                    if let Some((name, value)) =
                        engines.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))
                    {
                        s.gpu_percent = Some(value.clamp(0.0, 100.0));
                        s.gpu_engine = name;
                    }
                    let memory = self.values(self.memory);
                    if !memory.is_empty() {
                        s.gpu_memory_bytes = Some(memory.iter().map(|(_, v)| *v as u64).sum());
                    }
                }
            }
            s
        }
        unsafe fn values(&self, counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
            if counter.is_null() {
                return Vec::new();
            }
            let mut bytes = 0;
            let mut count = 0;
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                std::ptr::null_mut(),
            ) != PDH_MORE_DATA
            {
                return Vec::new();
            }
            if bytes > 16 * 1024 * 1024 {
                return Vec::new();
            }
            // u64 storage provides the alignment required by PDH structures.
            let mut buffer = vec![0u64; (bytes as usize + 7) / 8];
            let ptr = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, ptr)
                != 0
            {
                return Vec::new();
            }
            let prefix = format!("pid_{}_", std::process::id());
            std::slice::from_raw_parts(ptr, count as usize)
                .iter()
                .filter_map(|item| {
                    if item.FmtValue.CStatus > 1 || item.szName.is_null() {
                        return None;
                    }
                    let mut len = 0;
                    while *item.szName.add(len) != 0 {
                        len += 1;
                    }
                    let name =
                        String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
                    let value = item.FmtValue.Anonymous.doubleValue;
                    (name.starts_with(&prefix) && value.is_finite()).then_some((name, value))
                })
                .collect()
        }
    }
    impl Drop for Counters {
        fn drop(&mut self) {
            unsafe {
                if !self.query.is_null() {
                    PdhCloseQuery(self.query);
                }
            }
        }
    }
}
