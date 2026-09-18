use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    time::Instant,
};

use sysinfo::{Process, ProcessRefreshKind, ProcessesToUpdate, System, Uid, UpdateKind, Users};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES},
        Graphics::Dxgi::{
            CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIFactory1,
        },
        NetworkManagement::{
            IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2},
            Ndis::NET_IF_OPER_STATUS_UP,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Performance::{
                PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W,
                PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW,
                PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
            },
            ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
            Services::{
                CloseServiceHandle, ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW,
                OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfigW,
                SC_ENUM_PROCESS_INFO, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_ACTIVE,
                SERVICE_QUERY_CONFIG, SERVICE_WIN32,
            },
        },
    },
    core::{HRESULT, PCWSTR},
};

use crate::model::{
    CommandLine, GpuAdapterSnapshot, GpuSnapshot, HistorySample, Metric, NetworkInterfaceSnapshot,
    NetworkSnapshot, ProcessSnapshot, Snapshot, SystemSnapshot, UserSource,
};

/// Collects the first-milestone process and system metrics.
///
/// `Collector` intentionally contains no terminal or application state. It is
/// run by the background worker while the UI remains responsive.
pub struct Collector {
    system: System,
    generation: u64,
    command_lines: HashMap<ProcessIdentity, CommandLine>,
    users: HashMap<Uid, String>,
    service_accounts: HashMap<String, Option<String>>,
    network_baselines: HashMap<u64, NetworkCounterBaseline>,
    gpu: GpuCollector,
}

#[derive(Clone, Copy, Debug)]
struct NetworkCounterBaseline {
    received_bytes: u64,
    transmitted_bytes: u64,
    sampled_at: Instant,
}

struct GpuCollector {
    query: Option<GpuPdhQuery>,
    adapters: Vec<GpuAdapterDescription>,
    discovery_error: Option<String>,
}

struct GpuPdhQuery {
    query: PDH_HQUERY,
    utilization: PDH_HCOUNTER,
    dedicated_memory: Option<PDH_HCOUNTER>,
    shared_memory: Option<PDH_HCOUNTER>,
    warmed_up: bool,
}

#[derive(Clone, Debug)]
struct GpuAdapterDescription {
    id: u64,
    name: String,
    dedicated_memory_capacity_bytes: Option<u64>,
    shared_memory_capacity_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ProcessIdentity {
    pid: u32,
    start_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ServiceAccount {
    Account(String),
    Mixed,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            generation: 0,
            command_lines: HashMap::new(),
            users: Users::new_with_refreshed_list()
                .list()
                .iter()
                .map(|user| (user.id().clone(), user.name().to_owned()))
                .collect(),
            service_accounts: HashMap::new(),
            network_baselines: HashMap::new(),
            gpu: GpuCollector::new(),
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
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(UpdateKind::OnlyIfNotSet),
        );
        self.refresh_new_process_details();

        self.generation = self.generation.saturating_add(1);

        let thread_counts = match thread_counts() {
            Ok(counts) => Metric::fresh(counts),
            Err(reason) => Metric::stale(HashMap::new(), reason),
        };
        let service_accounts =
            service_accounts_by_pid(&mut self.service_accounts).unwrap_or_default();
        let processes = match previous {
            Some(snapshot) if updated_processes == 0 && !snapshot.processes.value.is_empty() => {
                Metric::stale(
                    snapshot.processes.value.clone(),
                    "process refresh returned no processes",
                )
            }
            _ => Metric::fresh(self.collect_processes(previous, &thread_counts, &service_accounts)),
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
        let network = self.collect_network(previous);
        let gpu = self.collect_gpu(previous);

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
                network,
                gpu,
            },
            processes,
            history,
        }
    }

    fn collect_gpu(&mut self, previous: Option<&Snapshot>) -> GpuSnapshot {
        let utilization = match self.gpu.utilization_by_adapter() {
            Ok(utilization) => utilization,
            Err(reason) => {
                return previous.map_or_else(
                    || GpuSnapshot {
                        adapters: Metric::stale(Vec::new(), &reason),
                    },
                    |snapshot| GpuSnapshot {
                        adapters: Metric::stale(
                            snapshot.system.gpu.adapters.value.clone(),
                            &reason,
                        ),
                    },
                );
            }
        };
        let (dedicated_memory, shared_memory) = self.gpu.memory_by_adapter();
        GpuSnapshot {
            adapters: Metric::fresh(
                self.gpu
                    .adapters
                    .iter()
                    .map(|adapter| GpuAdapterSnapshot {
                        id: adapter.id,
                        name: adapter.name.clone(),
                        utilization_percent: Metric::fresh(utilization.get(&adapter.id).copied()),
                        dedicated_memory_used_bytes: Metric::fresh(
                            dedicated_memory.get(&adapter.id).copied(),
                        ),
                        dedicated_memory_capacity_bytes: Metric::fresh(
                            adapter.dedicated_memory_capacity_bytes,
                        ),
                        shared_memory_used_bytes: Metric::fresh(
                            shared_memory.get(&adapter.id).copied(),
                        ),
                        shared_memory_capacity_bytes: Metric::fresh(
                            adapter.shared_memory_capacity_bytes,
                        ),
                    })
                    .collect(),
            ),
        }
    }

    fn collect_network(&mut self, previous: Option<&Snapshot>) -> NetworkSnapshot {
        let sampled_at = Instant::now();
        let rows = match network_interfaces() {
            Ok(rows) => rows,
            Err(reason) => {
                return previous.map_or_else(
                    || NetworkSnapshot {
                        interfaces: Metric::stale(Vec::new(), reason.clone()),
                        total_transmit_bytes_per_second: Metric::stale(None, reason.clone()),
                        total_receive_bytes_per_second: Metric::stale(None, &reason),
                    },
                    |snapshot| stale_network_snapshot(&snapshot.system.network, &reason),
                );
            }
        };

        let mut live_interfaces = HashSet::new();
        let mut total_transmit = Some(0.0);
        let mut total_receive = Some(0.0);
        let mut interfaces = rows
            .into_iter()
            .map(|row| {
                live_interfaces.insert(row.id);
                let previous = self.network_baselines.insert(
                    row.id,
                    NetworkCounterBaseline {
                        received_bytes: row.received_bytes,
                        transmitted_bytes: row.transmitted_bytes,
                        sampled_at,
                    },
                );
                let transmit = previous.and_then(|baseline| {
                    counter_rate(
                        baseline.transmitted_bytes,
                        row.transmitted_bytes,
                        baseline.sampled_at,
                        sampled_at,
                    )
                });
                let receive = previous.and_then(|baseline| {
                    counter_rate(
                        baseline.received_bytes,
                        row.received_bytes,
                        baseline.sampled_at,
                        sampled_at,
                    )
                });
                if row.operational {
                    total_transmit = total_transmit
                        .zip(transmit)
                        .map(|(total, rate)| total + rate);
                    total_receive = total_receive.zip(receive).map(|(total, rate)| total + rate);
                }
                NetworkInterfaceSnapshot {
                    id: row.id,
                    alias: row.alias,
                    operational: row.operational,
                    transmit_bytes_per_second: Metric::fresh(transmit),
                    receive_bytes_per_second: Metric::fresh(receive),
                }
            })
            .filter(|interface| interface.operational)
            .collect::<Vec<_>>();
        self.network_baselines
            .retain(|id, _| live_interfaces.contains(id));
        interfaces.sort_by(|left, right| {
            let left_rate = left.transmit_bytes_per_second.value.unwrap_or_default()
                + left.receive_bytes_per_second.value.unwrap_or_default();
            let right_rate = right.transmit_bytes_per_second.value.unwrap_or_default()
                + right.receive_bytes_per_second.value.unwrap_or_default();
            right_rate
                .total_cmp(&left_rate)
                .then_with(|| left.alias.cmp(&right.alias))
        });

        NetworkSnapshot {
            interfaces: Metric::fresh(interfaces),
            total_transmit_bytes_per_second: Metric::fresh(total_transmit),
            total_receive_bytes_per_second: Metric::fresh(total_receive),
        }
    }

    fn collect_processes(
        &mut self,
        previous: Option<&Snapshot>,
        thread_counts: &Metric<HashMap<u32, u64>>,
        service_accounts: &HashMap<u32, ServiceAccount>,
    ) -> Vec<ProcessSnapshot> {
        let mut live_processes = HashSet::new();
        let (system, command_lines, users) = (&self.system, &mut self.command_lines, &self.users);
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

                let token_user = process.user_id().map(|uid| {
                    let sid = uid.to_string();
                    well_known_windows_user(&sid)
                        .map(str::to_owned)
                        .or_else(|| users.get(uid).cloned())
                        .unwrap_or(sid)
                });
                let (user, user_source) = match token_user {
                    Some(user) => (Some(user), UserSource::Token),
                    None => match service_accounts.get(&identity.pid) {
                        Some(ServiceAccount::Account(account)) => {
                            (Some(account.clone()), UserSource::ServiceConfiguration)
                        }
                        Some(ServiceAccount::Mixed) => (
                            Some("<mixed service accounts>".into()),
                            UserSource::ServiceConfiguration,
                        ),
                        None => (None, UserSource::Restricted),
                    },
                };

                ProcessSnapshot {
                    pid: identity.pid,
                    parent_pid: process.parent().map(|pid| pid.as_u32()),
                    name: process.name().to_string_lossy().into_owned(),
                    user,
                    user_source,
                    command_line: cached_command_line(command_lines, identity, process),
                    executable_path: process.exe().map(ToOwned::to_owned),
                    cpu_percent: normalize_process_cpu_percent(
                        process.cpu_usage(),
                        logical_cpu_count,
                    ),
                    thread_count: Metric {
                        value: thread_counts
                            .value
                            .get(&identity.pid)
                            .copied()
                            .or_else(|| {
                                previous.and_then(|snapshot| {
                                    snapshot
                                        .processes
                                        .value
                                        .iter()
                                        .find(|process| process.pid == identity.pid)
                                        .map(|process| process.thread_count.value)
                                })
                            })
                            .unwrap_or(0),
                        freshness: thread_counts.freshness.clone(),
                    },
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
                .with_exe(UpdateKind::Always)
                .with_user(UpdateKind::OnlyIfNotSet),
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

impl GpuCollector {
    fn new() -> Self {
        let (adapters, discovery_error) = match gpu_adapters() {
            Ok(adapters) => (adapters, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let query = GpuPdhQuery::new().ok();
        Self {
            query,
            adapters,
            discovery_error,
        }
    }

    fn utilization_by_adapter(&mut self) -> Result<HashMap<u64, f32>, String> {
        if let Some(error) = &self.discovery_error {
            return Err(error.clone());
        }
        let Some(query) = &mut self.query else {
            return Ok(HashMap::new());
        };
        query.collect()
    }

    fn memory_by_adapter(&self) -> (HashMap<u64, u64>, HashMap<u64, u64>) {
        self.query
            .as_ref()
            .map(GpuPdhQuery::memory_by_adapter)
            .unwrap_or_default()
    }
}

impl GpuPdhQuery {
    fn new() -> Result<Self, String> {
        let mut query = PDH_HQUERY::default();
        pdh_result(
            unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) },
            "PdhOpenQueryW",
        )?;
        let path = "\\GPU Engine(*)\\Utilization Percentage\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        let mut utilization = PDH_HCOUNTER::default();
        if let Err(error) = pdh_result(
            unsafe { PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut utilization) },
            "PdhAddEnglishCounterW(GPU Engine)",
        ) {
            unsafe { PdhCloseQuery(query) };
            return Err(error);
        }
        let dedicated_memory =
            pdh_add_english_counter(query, r"\GPU Adapter Memory(*)\Dedicated Usage").ok();
        let shared_memory =
            pdh_add_english_counter(query, r"\GPU Adapter Memory(*)\Shared Usage").ok();
        Ok(Self {
            query,
            utilization,
            dedicated_memory,
            shared_memory,
            warmed_up: false,
        })
    }

    fn collect(&mut self) -> Result<HashMap<u64, f32>, String> {
        pdh_result(
            unsafe { PdhCollectQueryData(self.query) },
            "PdhCollectQueryData",
        )?;
        if !self.warmed_up {
            self.warmed_up = true;
            return Ok(HashMap::new());
        }

        let items = pdh_counter_array(self.utilization)?;
        let mut utilization = HashMap::<u64, f32>::new();
        for (instance, value) in items {
            let Some(adapter_id) = gpu_luid_from_engine_instance(&instance) else {
                continue;
            };
            if !value.is_finite() {
                continue;
            }
            utilization
                .entry(adapter_id)
                .and_modify(|current| *current = current.max(value.clamp(0.0, 100.0)))
                .or_insert_with(|| value.clamp(0.0, 100.0));
        }
        Ok(utilization)
    }

    fn memory_by_adapter(&self) -> (HashMap<u64, u64>, HashMap<u64, u64>) {
        let dedicated = self
            .dedicated_memory
            .and_then(|counter| pdh_memory_by_adapter(counter).ok())
            .unwrap_or_default();
        let shared = self
            .shared_memory
            .and_then(|counter| pdh_memory_by_adapter(counter).ok())
            .unwrap_or_default();
        (dedicated, shared)
    }
}

impl Drop for GpuPdhQuery {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn pdh_result(result: u32, operation: &str) -> Result<(), String> {
    (result == 0)
        .then_some(())
        .ok_or_else(|| format!("{operation} failed with PDH status 0x{result:08X}"))
}

fn pdh_add_english_counter(query: PDH_HQUERY, path: &str) -> Result<PDH_HCOUNTER, String> {
    let path = format!("{path}\0").encode_utf16().collect::<Vec<_>>();
    let mut counter = PDH_HCOUNTER::default();
    pdh_result(
        unsafe { PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) },
        "PdhAddEnglishCounterW",
    )?;
    Ok(counter)
}

fn pdh_memory_by_adapter(counter: PDH_HCOUNTER) -> Result<HashMap<u64, u64>, String> {
    let mut memory = HashMap::new();
    for (instance, value) in pdh_counter_array(counter)? {
        let Some(adapter_id) = gpu_luid_from_engine_instance(&instance) else {
            continue;
        };
        if value.is_finite() && value >= 0.0 {
            memory.insert(adapter_id, value.round() as u64);
        }
    }
    Ok(memory)
}

fn pdh_counter_array(counter: PDH_HCOUNTER) -> Result<Vec<(String, f32)>, String> {
    let mut buffer_size = 0;
    let mut item_count = 0;
    let first_result = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            None,
        )
    };
    if first_result != PDH_MORE_DATA {
        return Err(format!(
            "PdhGetFormattedCounterArrayW size query failed with PDH status 0x{first_result:08X}"
        ));
    }
    let capacity = usize::try_from(buffer_size)
        .map_err(|_| "PDH counter buffer is too large".to_owned())?
        .div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
    let mut buffer = Vec::<PDH_FMT_COUNTERVALUE_ITEM_W>::with_capacity(capacity);
    let result = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            Some(buffer.as_mut_ptr()),
        )
    };
    pdh_result(result, "PdhGetFormattedCounterArrayW")?;
    let item_count = usize::try_from(item_count).map_err(|_| "PDH item count is too large")?;
    unsafe { buffer.set_len(item_count) };
    Ok(buffer
        .into_iter()
        .filter(|item| {
            matches!(
                item.FmtValue.CStatus,
                PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
            )
        })
        .map(|item| {
            let name = unsafe { item.szName.to_string() }.unwrap_or_default();
            let value = unsafe { item.FmtValue.Anonymous.doubleValue } as f32;
            (name, value)
        })
        .collect())
}

fn gpu_adapters() -> Result<Vec<GpuAdapterDescription>, String> {
    let factory =
        unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.map_err(|error| error.to_string())?;
    let mut adapters = Vec::new();
    for index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(error.to_string()),
        };
        let description = unsafe { adapter.GetDesc1() }.map_err(|error| error.to_string())?;
        if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        adapters.push(GpuAdapterDescription {
            id: luid_to_u64(
                description.AdapterLuid.LowPart,
                description.AdapterLuid.HighPart,
            ),
            name: wide_c_string(&description.Description),
            dedicated_memory_capacity_bytes: u64::try_from(description.DedicatedVideoMemory)
                .ok()
                .filter(|bytes| *bytes > 0),
            shared_memory_capacity_bytes: u64::try_from(description.SharedSystemMemory)
                .ok()
                .filter(|bytes| *bytes > 0),
        });
    }
    Ok(adapters)
}

fn gpu_luid_from_engine_instance(instance: &str) -> Option<u64> {
    let (_, suffix) = instance.split_once("luid_0x")?;
    let (high, suffix) = suffix.split_once("_0x")?;
    let low = suffix.split('_').next()?;
    let high = u32::from_str_radix(high, 16).ok()?;
    let low = u32::from_str_radix(low, 16).ok()?;
    Some(luid_to_u64(low, high as i32))
}

fn luid_to_u64(low: u32, high: i32) -> u64 {
    u64::from(low) | (u64::from(high as u32) << 32)
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

#[derive(Clone, Debug)]
struct NetworkInterfaceCounters {
    id: u64,
    alias: String,
    operational: bool,
    received_bytes: u64,
    transmitted_bytes: u64,
}

/// Returns counters for every Windows network interface. The returned table is
/// copied before its Windows-owned allocation is freed.
fn network_interfaces() -> Result<Vec<NetworkInterfaceCounters>, String> {
    let mut table = std::ptr::null_mut::<MIB_IF_TABLE2>();
    let result = unsafe { GetIfTable2(&mut table) };
    if !result.is_ok() {
        return Err(format!("GetIfTable2 failed: {result:?}"));
    }
    if table.is_null() {
        return Err("GetIfTable2 returned no table".into());
    }

    let rows = unsafe {
        let entries = usize::try_from((*table).NumEntries).unwrap_or(0);
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), entries);
        rows.iter()
            .map(|row| NetworkInterfaceCounters {
                id: row.InterfaceLuid.Value,
                alias: wide_c_string(&row.Alias),
                operational: row.OperStatus.0 == NET_IF_OPER_STATUS_UP.0,
                received_bytes: row.InOctets,
                transmitted_bytes: row.OutOctets,
            })
            .collect::<Vec<_>>()
    };
    unsafe { FreeMibTable(table.cast()) };
    Ok(rows)
}

fn wide_c_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..length])
}

fn counter_rate(
    previous: u64,
    current: u64,
    previous_at: Instant,
    sampled_at: Instant,
) -> Option<f64> {
    let elapsed = sampled_at
        .saturating_duration_since(previous_at)
        .as_secs_f64();
    (elapsed > 0.0)
        .then(|| {
            current
                .checked_sub(previous)
                .map(|delta| delta as f64 / elapsed)
        })
        .flatten()
}

fn stale_network_snapshot(previous: &NetworkSnapshot, reason: &str) -> NetworkSnapshot {
    NetworkSnapshot {
        interfaces: Metric::stale(previous.interfaces.value.clone(), reason),
        total_transmit_bytes_per_second: Metric::stale(
            previous.total_transmit_bytes_per_second.value,
            reason,
        ),
        total_receive_bytes_per_second: Metric::stale(
            previous.total_receive_bytes_per_second.value,
            reason,
        ),
    }
}

/// Returns concise names for Windows service accounts whose SIDs are stable
/// and commonly encountered even when they are absent from sysinfo's account
/// list.
fn well_known_windows_user(sid: &str) -> Option<&'static str> {
    match sid {
        "S-1-5-18" => Some("SYSTEM"),
        "S-1-5-19" => Some("LOCAL SERVICE"),
        "S-1-5-20" => Some("NETWORK SERVICE"),
        _ => None,
    }
}

/// Maps running service PIDs to their configured logon account. This provides
/// useful attribution when Windows denies access to a service process token.
/// The result is deliberately conservative: a PID with services configured
/// for different accounts is marked `Mixed`, never assigned an arbitrary one.
fn service_accounts_by_pid(
    cached_accounts: &mut HashMap<String, Option<String>>,
) -> Result<HashMap<u32, ServiceAccount>, String> {
    let manager =
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ENUMERATE_SERVICE) }
            .map_err(|error| error.to_string())?;

    let result = (|| {
        let mut by_pid = HashMap::<u32, Vec<Option<String>>>::new();
        let mut resume_handle = 0;

        loop {
            // SCM may return its service list in batches. `usize` storage keeps
            // the embedded service structures and their wide-string pointers
            // suitably aligned while the batch is parsed.
            let mut storage = vec![0_usize; 32 * 1024];
            let byte_len = storage.len() * std::mem::size_of::<usize>();
            let buffer = unsafe {
                std::slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), byte_len)
            };
            let mut bytes_needed = 0;
            let mut services_returned = 0;
            let enumeration = unsafe {
                EnumServicesStatusExW(
                    manager,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_ACTIVE,
                    Some(buffer),
                    &mut bytes_needed,
                    &mut services_returned,
                    Some(&mut resume_handle),
                    PCWSTR::null(),
                )
            };

            let entries = unsafe {
                std::slice::from_raw_parts(
                    storage.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                    services_returned as usize,
                )
            };
            for entry in entries {
                let pid = entry.ServiceStatusProcess.dwProcessId;
                if pid == 0 {
                    continue;
                }

                let service_name = unsafe { entry.lpServiceName.to_string() }
                    .map_err(|error| error.to_string())?;
                let account = cached_accounts
                    .entry(service_name.clone())
                    .or_insert_with(|| service_start_account(manager, &service_name));
                by_pid.entry(pid).or_default().push(account.clone());
            }

            match enumeration {
                Ok(()) => break,
                Err(error) if error.code() == HRESULT::from_win32(234) => continue,
                Err(error) => return Err(error.to_string()),
            }
        }

        Ok(by_pid
            .into_iter()
            .filter_map(|(pid, accounts)| {
                let accounts = accounts.into_iter().collect::<Option<HashSet<_>>>()?;
                (accounts.len() == 1)
                    .then(|| ServiceAccount::Account(accounts.into_iter().next().unwrap()))
                    .or(Some(ServiceAccount::Mixed))
                    .map(|account| (pid, account))
            })
            .collect())
    })();

    let close_result = unsafe { CloseServiceHandle(manager) }.map_err(|error| error.to_string());
    match (result, close_result) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(accounts), Ok(())) => Ok(accounts),
    }
}

fn service_start_account(
    manager: windows::Win32::System::Services::SC_HANDLE,
    name: &str,
) -> Option<String> {
    let wide_name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let service = unsafe {
        OpenServiceW(
            manager,
            PCWSTR::from_raw(wide_name.as_ptr()),
            SERVICE_QUERY_CONFIG,
        )
    }
    .ok()?;

    let result = (|| {
        let mut bytes_needed = 0;
        let _ = unsafe { QueryServiceConfigW(service, None, 0, &mut bytes_needed) };
        if bytes_needed == 0 {
            return None;
        }

        let storage_len = (bytes_needed as usize).div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0_usize; storage_len];
        let config = storage.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        unsafe { QueryServiceConfigW(service, Some(config), bytes_needed, &mut bytes_needed) }
            .ok()?;
        let start_name = unsafe { (*config).lpServiceStartName.to_string() }.ok()?;
        Some(normalize_service_account(&start_name))
    })();

    let _ = unsafe { CloseServiceHandle(service) };
    result
}

fn normalize_service_account(account: &str) -> String {
    if account.eq_ignore_ascii_case("LocalSystem")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\LocalSystem")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM")
    {
        "SYSTEM".into()
    } else if account.eq_ignore_ascii_case("LocalService")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\LocalService")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\LOCAL SERVICE")
    {
        "LOCAL SERVICE".into()
    } else if account.eq_ignore_ascii_case("NetworkService")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\NetworkService")
        || account.eq_ignore_ascii_case("NT AUTHORITY\\NETWORK SERVICE")
    {
        "NETWORK SERVICE".into()
    } else {
        account.to_owned()
    }
}

/// Returns the number of threads currently reported for each process by the
/// Windows Tool Help snapshot. This is intentionally a separate query because
/// sysinfo does not expose a Windows thread count per process.
fn thread_counts() -> Result<HashMap<u32, u64>, String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|error| error.to_string())?;

    let result: Result<HashMap<u32, u64>, String> = (|| {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe { Process32FirstW(snapshot, &mut entry) }.map_err(|error| error.to_string())?;

        let mut counts = HashMap::new();
        loop {
            counts.insert(entry.th32ProcessID, u64::from(entry.cntThreads));

            match unsafe { Process32NextW(snapshot, &mut entry) } {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => break,
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(counts)
    })();

    let close_result = unsafe { CloseHandle(snapshot) }.map_err(|error| error.to_string());
    match (result, close_result) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(counts), Ok(())) => Ok(counts),
    }
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
        Collector, command_line_from_arguments, counter_rate, gpu_luid_from_engine_instance,
        normalize_process_cpu_percent, normalize_service_account, normalize_system_cpu_percent,
        pages_to_bytes, stale_metric, well_known_windows_user,
    };
    use crate::model::{CommandLine, Freshness, Metric};
    use std::time::{Duration, Instant};

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
    fn network_rates_use_the_actual_elapsed_time_and_reject_counter_resets() {
        let started_at = Instant::now();
        assert_eq!(
            counter_rate(
                100,
                1_100,
                started_at,
                started_at + Duration::from_millis(250)
            ),
            Some(4_000.0)
        );
        assert_eq!(
            counter_rate(1_100, 100, started_at, started_at + Duration::from_secs(1)),
            None
        );
        assert_eq!(counter_rate(100, 200, started_at, started_at), None);
    }

    #[test]
    fn gpu_engine_instance_luid_maps_to_the_dxgi_adapter_identity() {
        assert_eq!(
            gpu_luid_from_engine_instance(
                "pid_42_luid_0x00000001_0x00000002_phys_0_eng_0_engtype_3D"
            ),
            Some(0x00000001_00000002)
        );
        assert_eq!(gpu_luid_from_engine_instance("pid_42_engtype_3D"), None);
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
    fn well_known_service_sids_have_concise_labels() {
        assert_eq!(well_known_windows_user("S-1-5-18"), Some("SYSTEM"));
        assert_eq!(well_known_windows_user("S-1-5-19"), Some("LOCAL SERVICE"));
        assert_eq!(well_known_windows_user("S-1-5-20"), Some("NETWORK SERVICE"));
        assert_eq!(well_known_windows_user("S-1-5-21-123"), None);
    }

    #[test]
    fn service_configuration_accounts_are_normalized_for_display() {
        assert_eq!(normalize_service_account("LocalSystem"), "SYSTEM");
        assert_eq!(
            normalize_service_account("NT AUTHORITY\\LocalSystem"),
            "SYSTEM"
        );
        assert_eq!(
            normalize_service_account("NT AUTHORITY\\LocalService"),
            "LOCAL SERVICE"
        );
        assert_eq!(
            normalize_service_account("NT AUTHORITY\\LOCAL SERVICE"),
            "LOCAL SERVICE"
        );
        assert_eq!(
            normalize_service_account("NetworkService"),
            "NETWORK SERVICE"
        );
        assert_eq!(
            normalize_service_account("NT AUTHORITY\\NetworkService"),
            "NETWORK SERVICE"
        );
        assert_eq!(
            normalize_service_account("CONTOSO\\agent"),
            "CONTOSO\\agent"
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
