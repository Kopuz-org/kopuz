// Release builds start from a launcher, so on Windows they open no console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

/// jemalloc, so the profiler can ask the allocator what is held instead of reading RSS.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Hand freed pages back to the OS on a timer, so releasing a cover shows up in RSS rather than
/// leaving the process looking as large as it ever was.
#[cfg(not(target_env = "msvc"))]
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8] = b"background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000\0";

fn main() -> anyhow::Result<()> {
    kopuz_app::run()
}
