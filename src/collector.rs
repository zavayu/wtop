use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    time::Instant,
};

use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

use crate::model::{CommandLine, Metric, ProcessSnapshot, Snapshot, SystemSnapshot};

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

        Snapshot {
            generation: self.generation,
            collected_at: Instant::now(),
            system: SystemSnapshot {
                cpu_percent: Metric::fresh(self.system.global_cpu_usage()),
                total_memory_bytes: Metric::fresh(self.system.total_memory()),
                used_memory_bytes: Metric::fresh(self.system.used_memory()),
                commit_charge_bytes,
                commit_limit_bytes,
            },
            processes,
        }
    }

    fn collect_processes(&mut self) -> Vec<ProcessSnapshot> {
        let mut live_processes = HashSet::new();
        let (system, command_lines) = (&self.system, &mut self.command_lines);
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
                    cpu_percent: process.cpu_usage(),
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
    use super::{Collector, command_line_from_arguments, pages_to_bytes, stale_metric};
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
}
