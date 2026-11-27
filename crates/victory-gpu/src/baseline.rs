//! Wall-clock samples for low-memory runs, including intervals without a present.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Start once at the beginning of `run --execute` when `VICTORY_BASELINE_DIR`
/// names a directory. Each title process (including relaunches) gets its own file.
pub fn start_baseline() {
    let Some(dir) = std::env::var_os("VICTORY_BASELINE_DIR").filter(|v| !v.is_empty()) else {
        return;
    };
    let start = Instant::now();
    let result = std::thread::Builder::new()
        .name("victory-baseline".into())
        .spawn(move || {
            let path = PathBuf::from(dir).join(format!("baseline-{}.csv", std::process::id()));
            if let Err(error) = record(&path, start) {
                tracing::warn!(path = %path.display(), %error, "baseline recording stopped");
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start baseline sampler");
    }
}

fn record(path: &Path, start: Instant) -> io::Result<()> {
    // The operator creates the run directory. Preserve evidence on PID reuse.
    let mut file = BufWriter::new(File::create_new(path)?);
    tracing::info!(path = %path.display(), "recording baseline samples");
    let mut first = true;
    loop {
        let elapsed = start.elapsed().as_millis() as u64;
        let utc = SystemTime::now().duration_since(UNIX_EPOCH).ok();
        let ram = crate::host::ram_sample();
        // Never initialize D3D12 or touch the renderer's budget-change tracking.
        let [local, nonlocal] = crate::host::made_host()
            .map(|host| host.query_video_memory_groups())
            .unwrap_or_default();
        let blocks = crate::pool::sample_texture_blocks();
        let kinds = &crate::resource::KINDS;
        let (shader_hits, shader_misses) = crate::shader_cache::counts();
        let fields = [
            ("elapsed_ms", Some(elapsed)),
            ("unix_ms", utc.map(|t| t.as_millis() as u64)),
            ("title_presents_total", Some(crate::queue::presented())),
            ("working_set_bytes", ram.map(|r| r.0)),
            ("private_commit_bytes", ram.map(|r| r.1)),
            ("available_physical_bytes", ram.map(|r| r.2)),
            ("available_commit_bytes", ram.map(|r| r.3)),
            ("local_usage_bytes", local.map(|m| m.CurrentUsage)),
            ("local_budget_bytes", local.map(|m| m.Budget)),
            ("nonlocal_usage_bytes", nonlocal.map(|m| m.CurrentUsage)),
            ("nonlocal_budget_bytes", nonlocal.map(|m| m.Budget)),
            (
                "host_texture_bytes",
                Some(crate::resource::HOST_TEXTURES[1].load(Relaxed)),
            ),
            ("texture_pool_bytes", blocks.map(|b| b.0)),
            ("empty_texture_pool_bytes", blocks.map(|b| b.1)),
            (
                "list_scratch_bytes",
                Some(crate::host::ALIVE[2].load(Relaxed)),
            ),
            (
                "spare_scratch_bytes",
                Some(crate::host::ALIVE[4].load(Relaxed)),
            ),
            ("title_filled_texture_bytes", Some(kinds[1].load(Relaxed))),
            (
                "live_staging_bytes",
                Some(crate::texture::live_staging_bytes()),
            ),
            ("texture_evicted_bytes_total", Some(kinds[3].load(Relaxed))),
            (
                "texture_recreated_bytes_total",
                Some(kinds[5].load(Relaxed)),
            ),
            (
                "texture_slots_waiting",
                Some(crate::draw::HEAP_TEXTURES_WAITING.load(Relaxed)),
            ),
            (
                "texture_no_room_passes_total",
                Some(crate::draw::HEAP_TEXTURES_FULL.load(Relaxed)),
            ),
            ("shader_cache_hits_total", Some(shader_hits)),
            ("shader_cache_misses_total", Some(shader_misses)),
            ("pipelines_pending", Some(crate::pipeline::making())),
        ];
        write_sample(&mut file, &fields, first)?;
        file.flush()?; // Keep completed rows if the title or driver fails.
        first = false;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn write_sample(
    out: &mut impl Write,
    fields: &[(&str, Option<u64>)],
    first: bool,
) -> io::Result<()> {
    if first {
        for (i, (name, _)) in fields.iter().enumerate() {
            if i > 0 {
                write!(out, ",")?;
            }
            write!(out, "{name}")?;
        }
        writeln!(out)?;
    }
    for (i, (_, value)) in fields.iter().enumerate() {
        if i > 0 {
            write!(out, ",")?;
        }
        if let Some(value) = value {
            write!(out, "{value}")?;
        }
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_counters_are_blank_and_header_is_written_once() {
        let mut csv = Vec::new();
        write_sample(&mut csv, &[("local", None), ("nonlocal", Some(0))], true).unwrap();
        write_sample(&mut csv, &[("local", Some(42)), ("nonlocal", None)], false).unwrap();
        assert_eq!(csv, b"local,nonlocal\n,0\n42,\n");
    }
}
