use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    time::Instant,
};

use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

use crate::model::{CommandLine, HistorySample, Metric, ProcessSnapshot, Snapshot, SystemSnapshot};

/// Collects the first-milestone process and system metrics.
///
/// `Collector` intentionally contains no terminal or application state. It is
/// run by the background worker while the UI remains responsive.
pub struct Collector {
    system: System,
    generation: u64,
    command_lines: HashMap<ProcessIdentity, CommandLine>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ProcessIdentity {
    pid: u32,
    start_time: u64,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            generation: 0,
            command_lines: HashMap::new(),
        }
    }

    /// Refreshes metrics and returns a self-contained, immutable snapshot.
    ///
    /// The first call establishes CPU sampling baselines. Later calls provide
    /// meaningful CPU percentages because this collector retains its `System`.
    pub fn collect(&mut self, previous: Option<&Snapshot>) -> Snapshot {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        let updated_processes = self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        self.refresh_new_process_details();

        self.generation = self.generation.saturating_add(1);

        let processes = match previous {
            Some(snapshot) if updated_processes == 0 && !snapshot.processes.value.is_empty() => {
                Metric::stale(
                    snapshot.processes.value.clone(),
                    "process refresh returned no processes",
                )
            }
            _ => Metric::fresh(self.collect_processes()),
        };
        let (commit_charge_bytes, commit_limit_bytes) = match commit_metrics() {
            Ok((charge, limit)) => (Metric::fresh(charge), Metric::fresh(limit)),
            Err(reason) => (
                stale_metric(
                    previous.map(|snapshot| &snapshot.system.commit_charge_bytes),
                    &reason,
                ),
                stale_metric(
                    previous.map(|snapshot| &snapshot.system.commit_limit_bytes),
                    &reason,
                ),
            ),
        };

        let cpu_percent = self.system.global_cpu_usage();
        let logical_cpu_percentages = self
            .system
            .cpus()
            .iter()
            .map(|cpu| normalize_system_cpu_percent(cpu.cpu_usage()))
            .collect();
        let total_memory_bytes = self.system.total_memory();
        let used_memory_bytes = self.system.used_memory();

        let mut history = previous
            .map(|snapshot| snapshot.history.clone())
            .unwrap_or_default();
        history.push(HistorySample { cpu_percent });

        Snapshot {
            generation: self.generation,
            collected_at: Instant::now(),
            system: SystemSnapshot {
                cpu_percent: Metric::fresh(cpu_percent),
                logical_cpu_percentages: Metric::fresh(logical_cpu_percentages),
                total_memory_bytes: Metric::fresh(total_memory_bytes),
                used_memory_bytes: Metric::fresh(used_memory_bytes),
                commit_charge_bytes,
                commit_limit_bytes,
            },
            processes,
            history,
        }
    }

    fn collect_processes(&mut self) -> Vec<ProcessSnapshot> {
        let mut live_processes = HashSet::new();
        let (system, command_lines) = (&self.system, &mut self.command_lines);
        let logical_cpu_count = system.cpus().len();
        let mut processes = system
            .processes()
            .values()
            .map(|process| {
                let identity = ProcessIdentity {
                    pid: process.pid().as_u32(),
                    start_time: process.start_time(),
                };
                live_processes.insert(identity);

                ProcessSnapshot {
                    pid: identity.pid,
                    parent_pid: process.parent().map(|pid| pid.as_u32()),
                    name: process.name().to_string_lossy().into_owned(),
                    command_line: cached_command_line(command_lines, identity, process),
                    executable_path: process.exe().map(ToOwned::to_owned),
                    cpu_percent: normalize_process_cpu_percent(
                        process.cpu_usage(),
                        logical_cpu_count,
                    ),
                    memory_bytes: process.memory(),
                }
            })
            .collect::<Vec<_>>();

        self.command_lines
            .retain(|identity, _| live_processes.contains(identity));
        processes.sort_by_key(|process| process.pid);
        processes
    }

    /// Queries command-line and executable details once for each process
    /// identity. In particular, a denied command-line read is not retried on
    /// every refresh.
    fn refresh_new_process_details(&mut self) {
        let pids = self
            .system
            .processes()
            .values()
            .filter_map(|process| {
                let identity = ProcessIdentity {
                    pid: process.pid().as_u32(),
                    start_time: process.start_time(),
                };

                (!self.command_lines.contains_key(&identity)).then_some(process.pid())
            })
            .collect::<Vec<_>>();

        if pids.is_empty() {
            return;
        }

        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            false,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .with_exe(UpdateKind::Always),
        );
    }
}

/// Converts sysinfo's per-logical-CPU process usage into a percentage of the
/// whole machine, matching the system CPU metric displayed in the header.
fn normalize_process_cpu_percent(raw_percent: f32, logical_cpu_count: usize) -> f32 {
    if !raw_percent.is_finite() {
        return 0.0;
    }

    let logical_cpu_count = logical_cpu_count.max(1) as f32;
    (raw_percent / logical_cpu_count).clamp(0.0, 100.0)
}

fn normalize_system_cpu_percent(raw_percent: f32) -> f32 {
    if raw_percent.is_finite() {
        raw_percent.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

fn command_line_from_arguments(arguments: &[OsString]) -> CommandLine {
    if arguments.is_empty() {
        CommandLine::Unavailable
    } else {
        CommandLine::Present(
            arguments
                .iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

fn cached_command_line(
    command_lines: &mut HashMap<ProcessIdentity, CommandLine>,
    identity: ProcessIdentity,
    process: &Process,
) -> CommandLine {
    command_lines
        .entry(identity)
        .or_insert_with(|| command_line_from_arguments(process.cmd()))
        .clone()
}

fn stale_metric(previous: Option<&Metric<u64>>, reason: &str) -> Metric<u64> {
    let value = previous.map_or(0, |metric| metric.value);
    Metric::stale(value, reason)
}

fn commit_metrics() -> Result<(u64, u64), String> {
    let mut performance = PERFORMANCE_INFORMATION {
        cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        ..Default::default()
    };

    unsafe {
        GetPerformanceInfo(
            &mut performance,
            std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        )
        .map_err(|error| error.to_string())?;
    }

    Ok((
        pages_to_bytes(performance.CommitTotal, performance.PageSize)?,
        pages_to_bytes(performance.CommitLimit, performance.PageSize)?,
    ))
}

fn pages_to_bytes(pages: usize, page_size: usize) -> Result<u64, String> {
    let pages = u64::try_from(pages).map_err(|_| "commit page count is too large".to_owned())?;
    let page_size =
        u64::try_from(page_size).map_err(|_| "system page size is too large".to_owned())?;

    pages
        .checked_mul(page_size)
        .ok_or_else(|| "commit byte count overflowed u64".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        Collector, command_line_from_arguments, normalize_process_cpu_percent,
        normalize_system_cpu_percent, pages_to_bytes, stale_metric,
    };
    use crate::model::{CommandLine, Freshness, Metric};

    #[test]
    fn command_line_mapping_preserves_unavailable_state() {
        assert_eq!(command_line_from_arguments(&[]), CommandLine::Unavailable);
        assert_eq!(
            command_line_from_arguments(&["worker.exe".into(), "--port".into(), "8080".into()]),
            CommandLine::Present("worker.exe --port 8080".into())
        );
    }

    #[test]
    fn commit_pages_are_converted_to_bytes_without_rounding() {
        assert_eq!(pages_to_bytes(1_024, 4_096), Ok(4_194_304));
    }

    #[test]
    fn failed_commit_metric_reuses_the_previous_value() {
        let metric = stale_metric(Some(&Metric::fresh(123_u64)), "performance query failed");

        assert_eq!(metric.value, 123);
        assert_eq!(
            metric.freshness,
            Freshness::Stale {
                reason: "performance query failed".into(),
            }
        );
    }

    #[test]
    fn collector_advances_snapshot_generation() {
        let mut collector = Collector::new();
        let first = collector.collect(None);
        let second = collector.collect(Some(&first));

        assert_eq!(second.generation, first.generation + 1);
    }

    #[test]
    fn process_cpu_is_normalized_to_total_machine_capacity() {
        assert_eq!(normalize_process_cpu_percent(250.0, 8), 31.25);
        assert_eq!(normalize_process_cpu_percent(1_600.0, 8), 100.0);
        assert_eq!(normalize_process_cpu_percent(50.0, 0), 50.0);
        assert_eq!(normalize_process_cpu_percent(f32::NAN, 8), 0.0);
    }

    #[test]
    fn logical_cpu_readings_are_clamped_and_non_finite_values_are_safe() {
        assert_eq!(normalize_system_cpu_percent(-1.0), 0.0);
        assert_eq!(normalize_system_cpu_percent(101.0), 100.0);
        assert_eq!(normalize_system_cpu_percent(f32::NAN), 0.0);
    }

    #[test]
    fn collector_records_one_history_sample_per_refresh() {
        let mut collector = Collector::new();
        let first = collector.collect(None);
        let second = collector.collect(Some(&first));

        assert_eq!(second.history.len(), 2);
        let newest = second
            .history
            .samples()
            .next_back()
            .expect("a sample was appended");
        assert_eq!(second.system.cpu_percent.value, newest.cpu_percent);
        assert_eq!(
            second.system.logical_cpu_percentages.value.len(),
            collector.system.cpus().len()
        );
    }
}
