use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

use crate::{
    app::{App, CpuDisplayMode, ProcessRow, ProcessViewMode, SortColumn, SortDirection},
    model::{Freshness, History, HistorySample, Metric, Snapshot, SystemSnapshot},
};

const MINIMUM_WIDTH: u16 = 64;
const MINIMUM_HEIGHT: u16 = 12;
const PID_COLUMN_WIDTH: u16 = 7;
const USER_COLUMN_WIDTH: u16 = 16;
const THREADS_COLUMN_WIDTH: u16 = 8;
const CPU_COLUMN_WIDTH: u16 = 7;
const MEMORY_COLUMN_WIDTH: u16 = 10;
const COLUMN_SPACING: u16 = 1;
const MIN_BAR_CELLS: u16 = 6;
const LABEL_WIDTH: usize = 8;
const COLUMN_GAP: usize = 2;
const MIN_LOGICAL_CPU_METER_WIDTH: usize = 24;
const LOGICAL_CPU_COLUMN_GAP: usize = 4;
const BAR_TICK: char = '|';
const FOOTER_HEIGHT: u16 = 3;
const TABLE_CHROME_HEIGHT: u16 = 3;

/// Renders the first-milestone dashboard from the newest completed snapshot.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if terminal_is_too_small(area.width, area.height) {
        render_minimum_size_message(frame);
        return;
    }

    let header_height = header_height(area, app);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(5),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);

    let inner_width = sections[0].width.saturating_sub(2);
    let header = Paragraph::new(header_lines(
        app.snapshot().map(|snapshot| &snapshot.system),
        app.snapshot().map(|snapshot| &snapshot.history),
        usize::from(inner_width),
        app.cpu_display_mode(),
    ))
    .alignment(Alignment::Left)
    .block(
        Block::default()
            .title(" wtop ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, sections[0]);

    let table = Table::new(
        app.viewport_process_rows()
            .into_iter()
            .map(|row| process_row(&row, app.view_mode())),
        [
            Constraint::Length(PID_COLUMN_WIDTH),
            Constraint::Length(USER_COLUMN_WIDTH),
            Constraint::Length(THREADS_COLUMN_WIDTH),
            Constraint::Length(CPU_COLUMN_WIDTH),
            Constraint::Length(MEMORY_COLUMN_WIDTH),
            Constraint::Min(8),
        ],
    )
    .column_spacing(COLUMN_SPACING)
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .header(
        Row::new(["PID", "USER", "THREADS", "CPU", "MEMORY", "NAME"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(process_table_title(app)),
    );
    let mut table_state = TableState::default();
    table_state.select(app.selected_viewport_index());
    frame.render_stateful_widget(table, sections[1], &mut table_state);

    let footer = Paragraph::new(footer_text(app))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, sections[2]);
}

fn terminal_is_too_small(width: u16, height: u16) -> bool {
    width < MINIMUM_WIDTH || height < MINIMUM_HEIGHT
}

/// Returns the number of process rows that fit beneath the table header.
///
/// The dashboard reserves a mode-dependent header and three rows for the footer.
/// The table itself uses a top and bottom border plus one header row.
pub fn process_table_row_capacity(area: Rect, app: &App) -> usize {
    if terminal_is_too_small(area.width, area.height) {
        0
    } else {
        usize::from(
            area.height
                .saturating_sub(header_height(area, app))
                .saturating_sub(FOOTER_HEIGHT + TABLE_CHROME_HEIGHT),
        )
    }
}

fn header_height(area: Rect, app: &App) -> u16 {
    let inner_width = usize::from(area.width.saturating_sub(2));
    let content_height = match app.cpu_display_mode() {
        CpuDisplayMode::Summary => 3,
        CpuDisplayMode::LogicalCpus => {
            let cpu_count = app
                .snapshot()
                .map(|snapshot| snapshot.system.logical_cpu_percentages.value.len())
                .unwrap_or(0);
            logical_cpu_grid_rows(cpu_count, inner_width).saturating_add(2)
        }
    };
    u16::try_from(content_height.saturating_add(2)).unwrap_or(u16::MAX)
}

fn render_minimum_size_message(frame: &mut Frame) {
    let message =
        format!("Terminal too small — minimum {MINIMUM_WIDTH} columns × {MINIMUM_HEIGHT} rows");
    let paragraph = Paragraph::new(message)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(Block::default().title(" wtop ").borders(Borders::ALL));
    frame.render_widget(paragraph, frame.area());
}

/// Renders either aggregate CPU history or a logical-CPU grid, then the memory
/// and Windows commit bars.
fn header_lines(
    system: Option<&SystemSnapshot>,
    history: Option<&History>,
    inner_width: usize,
    cpu_display_mode: CpuDisplayMode,
) -> Vec<Line<'static>> {
    let Some(system) = system else {
        return vec![
            Line::from(if cpu_display_mode == CpuDisplayMode::Summary {
                "CPU     --%"
            } else {
                "Logical CPUs  --"
            }),
            Line::from("Memory  -- / --"),
            Line::from("Commit  -- / --"),
        ];
    };

    let memory_readout = resource_readout(&system.used_memory_bytes, &system.total_memory_bytes);
    let commit_readout = resource_readout(&system.commit_charge_bytes, &system.commit_limit_bytes);
    let value_width = match cpu_display_mode {
        CpuDisplayMode::Summary => [
            display_width(&format_percent(&system.cpu_percent)),
            display_width(&memory_readout),
            display_width(&commit_readout),
        ]
        .into_iter()
        .max()
        .unwrap_or(0),
        CpuDisplayMode::LogicalCpus => [
            display_width(&memory_readout),
            display_width(&commit_readout),
        ]
        .into_iter()
        .max()
        .unwrap_or(0),
    };
    let chart_cells = chart_cell_budget(inner_width, value_width);

    let mut lines = match cpu_display_mode {
        CpuDisplayMode::Summary => vec![resource_line(
            "CPU",
            &format_percent(&system.cpu_percent),
            value_width,
            sparkline(history, chart_cells),
        )],
        CpuDisplayMode::LogicalCpus => {
            logical_cpu_lines(&system.logical_cpu_percentages.value, inner_width)
        }
    };
    lines.extend([
        resource_line(
            "Memory",
            &memory_readout,
            value_width,
            bar(
                percentage(&system.used_memory_bytes, &system.total_memory_bytes),
                chart_cells,
            ),
        ),
        resource_line(
            "Commit",
            &commit_readout,
            value_width,
            bar(
                percentage(&system.commit_charge_bytes, &system.commit_limit_bytes),
                chart_cells,
            ),
        ),
    ]);
    lines
}

fn logical_cpu_grid_rows(cpu_count: usize, inner_width: usize) -> usize {
    let columns = logical_cpu_columns(inner_width);
    cpu_count.div_ceil(columns).max(1)
}

fn logical_cpu_lines(percentages: &[f32], inner_width: usize) -> Vec<Line<'static>> {
    if percentages.is_empty() {
        return vec![Line::from("Logical CPUs  --")];
    }

    let columns = logical_cpu_columns(inner_width);
    let meter_width = logical_cpu_meter_width(inner_width, columns);
    percentages
        .chunks(columns)
        .enumerate()
        .map(|(row_index, row)| {
            let mut spans = Vec::new();
            for (column_index, percent) in row.iter().enumerate() {
                if column_index > 0 {
                    spans.push(Span::raw(" ".repeat(LOGICAL_CPU_COLUMN_GAP)));
                }
                spans.extend(logical_cpu_meter(
                    row_index * columns + column_index,
                    *percent,
                    meter_width,
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn logical_cpu_columns(inner_width: usize) -> usize {
    if inner_width >= MIN_LOGICAL_CPU_METER_WIDTH * 2 + LOGICAL_CPU_COLUMN_GAP {
        2
    } else {
        1
    }
}

fn logical_cpu_meter_width(inner_width: usize, columns: usize) -> usize {
    inner_width.saturating_sub(LOGICAL_CPU_COLUMN_GAP * columns.saturating_sub(1)) / columns
}

fn logical_cpu_meter(index: usize, percent: f32, width: usize) -> Vec<Span<'static>> {
    let percent = percent.clamp(0.0, 100.0);
    let label = format!("CPU {index:>2} ");
    let readout = format!("{percent:.1}%");
    let bar_width = width.saturating_sub(label.len());
    let inner_width = bar_width.saturating_sub(2);
    let tick_width = inner_width.saturating_sub(readout.len() + 1);
    let filled = ((f64::from(percent) / 100.0) * tick_width as f64).round() as usize;
    let color = usage_color(f64::from(percent) / 100.0);

    vec![
        Span::raw(label),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(
            BAR_TICK.to_string().repeat(filled.min(tick_width)),
            Style::default().fg(color),
        ),
        Span::styled(
            " ".repeat(tick_width.saturating_sub(filled)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(readout, Style::default().fg(Color::White)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
    ]
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn chart_cell_budget(inner_width: usize, value_width: usize) -> usize {
    inner_width.saturating_sub(LABEL_WIDTH + value_width + COLUMN_GAP)
}

fn resource_readout(used: &Metric<u64>, total: &Metric<u64>) -> String {
    let percent = percentage(used, total);
    format!(
        "{}/{} {:>3.0}%",
        format_metric_bytes(used),
        format_metric_bytes(total),
        percent * 100.0
    )
}

fn resource_line(
    label: &str,
    readout: &str,
    value_width: usize,
    chart: Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = vec![
        Span::raw(format!("{label:<LABEL_WIDTH$}")),
        Span::styled(
            format!("{readout:>value_width$}"),
            Style::default().fg(Color::White),
        ),
    ];
    if !chart.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(chart);
    }
    Line::from(spans)
}

fn sparkline(history: Option<&History>, width: usize) -> Vec<Span<'static>> {
    if width < MIN_BAR_CELLS as usize {
        return Vec::new();
    }
    let samples = history
        .map(|history| history.samples().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    vec![Span::styled(
        render_sparkline(&samples, width),
        Style::default().fg(Color::Cyan),
    )]
}

fn bar(fraction: f64, width: usize) -> Vec<Span<'static>> {
    if width < MIN_BAR_CELLS as usize {
        return Vec::new();
    }

    let inner_width = width - 2;
    let segment_count = inner_width;
    let filled_segments = (fraction * segment_count as f64).round() as usize;
    let filled_width = filled_segments.min(segment_count);
    let color = usage_color(fraction);
    vec![
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(
            BAR_TICK.to_string().repeat(filled_width),
            Style::default().fg(color),
        ),
        Span::styled(
            " ".repeat(inner_width - filled_width),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
    ]
}

fn usage_color(fraction: f64) -> Color {
    match fraction {
        fraction if fraction >= 0.9 => Color::Red,
        fraction if fraction >= 0.75 => Color::Yellow,
        _ => Color::Green,
    }
}

fn percentage(used: &Metric<u64>, total: &Metric<u64>) -> f64 {
    if total.value == 0 {
        0.0
    } else {
        (used.value as f64 / total.value as f64).clamp(0.0, 1.0)
    }
}

/// Renders a right-aligned CPU sparkline. The newest sample pins to the right
/// edge, older samples appear to its left, and a history shorter than the
/// budget is padded on the left.
fn render_sparkline(samples: &[HistorySample], width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    const SPARKLINE_LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let taken = samples.len().min(width);
    let mut rendered = String::with_capacity(width);
    rendered.push_str(&" ".repeat(width.saturating_sub(taken)));
    for sample in samples.iter().skip(samples.len() - taken) {
        let level = ((sample.cpu_percent.clamp(0.0, 100.0) / 100.0)
            * (SPARKLINE_LEVELS.len() - 1) as f32)
            .round() as usize;
        rendered.push(SPARKLINE_LEVELS[level]);
    }
    rendered
}

fn process_row(row: &ProcessRow<'_>, view_mode: ProcessViewMode) -> Row<'static> {
    let process = row.process;
    Row::new(vec![
        Cell::from(process.pid.to_string()),
        Cell::from(process.user_display().to_owned()),
        Cell::from(format_thread_count(&process.thread_count)),
        Cell::from(format_percent_value(process.cpu_percent)),
        Cell::from(format_bytes(process.memory_bytes)),
        Cell::from(process_name(row, view_mode)),
    ])
}

fn process_name(row: &ProcessRow<'_>, view_mode: ProcessViewMode) -> String {
    if view_mode == ProcessViewMode::Flat {
        return row.process.name.clone();
    }

    // Top-level processes have no parent relationship to draw. Starting their
    // names directly preserves space for the hierarchy that follows.
    if row.ancestor_has_next_siblings.is_empty() {
        return if row.has_children {
            let marker = if row.is_expanded { "▾ " } else { "▸ " };
            format!("{marker}{}", row.process.name)
        } else {
            row.process.name.clone()
        };
    }

    let ancestor_guides = row
        .ancestor_has_next_siblings
        .iter()
        .map(|has_next_sibling| if *has_next_sibling { "│   " } else { "    " })
        .collect::<String>();
    let branch = if row.is_last_sibling { '└' } else { '├' };
    let marker = if row.has_children {
        if row.is_expanded { "▾ " } else { "▸ " }
    } else {
        "─ "
    };
    format!("{ancestor_guides}{branch}─{marker}{}", row.process.name)
}

fn process_table_title(app: &App) -> String {
    let sort = app.sort();
    let filter = if app.filter().is_empty() {
        String::new()
    } else {
        format!(" / {}", app.filter())
    };
    format!(
        " Processes [{} · {}{}]{} ",
        view_mode_label(app.view_mode()),
        sort_column_label(sort.column),
        sort_direction_indicator(sort.direction),
        filter,
    )
}

fn footer_text(app: &App) -> String {
    let stale_prefix = if app
        .snapshot()
        .map(|snapshot| snapshot_is_stale(snapshot))
        .unwrap_or(false)
    {
        "~ Stale  "
    } else {
        ""
    };

    if app.is_filter_editing() {
        format!(
            "{stale_prefix}/{}  Enter apply  Esc cancel  Ctrl+U clear",
            app.filter()
        )
    } else if app.filter().is_empty() {
        if app.view_mode() == ProcessViewMode::Tree {
            format!("{stale_prefix}c CPU  t Flat  Enter/Space Collapse  s Sort  / Filter  q Quit")
        } else {
            format!("{stale_prefix}c CPU  t Tree  s Sort  S Reverse  / Filter  q Quit")
        }
    } else {
        format!(
            "{stale_prefix}Filter: {}  c CPU  t View  s Sort  S Reverse  / Edit  q Quit",
            app.filter()
        )
    }
}

fn view_mode_label(view_mode: ProcessViewMode) -> &'static str {
    match view_mode {
        ProcessViewMode::Flat => "Flat",
        ProcessViewMode::Tree => "Tree",
    }
}

fn sort_column_label(column: SortColumn) -> &'static str {
    match column {
        SortColumn::Pid => "PID",
        SortColumn::Name => "Name",
        SortColumn::User => "User",
        SortColumn::ThreadCount => "Threads",
        SortColumn::CpuPercent => "CPU",
        SortColumn::Memory => "Memory",
    }
}

fn sort_direction_indicator(direction: SortDirection) -> char {
    match direction {
        SortDirection::Ascending => '↑',
        SortDirection::Descending => '↓',
    }
}

fn snapshot_is_stale(snapshot: &Snapshot) -> bool {
    metric_is_stale(&snapshot.system.cpu_percent)
        || metric_is_stale(&snapshot.system.logical_cpu_percentages)
        || metric_is_stale(&snapshot.system.total_memory_bytes)
        || metric_is_stale(&snapshot.system.used_memory_bytes)
        || metric_is_stale(&snapshot.system.commit_charge_bytes)
        || metric_is_stale(&snapshot.system.commit_limit_bytes)
        || metric_is_stale(&snapshot.processes)
        || snapshot
            .processes
            .value
            .iter()
            .any(|process| metric_is_stale(&process.thread_count))
}

fn metric_is_stale<T>(metric: &Metric<T>) -> bool {
    matches!(&metric.freshness, Freshness::Stale { .. })
}

fn format_percent(metric: &Metric<f32>) -> String {
    format!(
        "{}{}",
        freshness_marker(metric),
        format_percent_value(metric.value)
    )
}

fn format_metric_bytes(metric: &Metric<u64>) -> String {
    format!("{}{}", freshness_marker(metric), format_bytes(metric.value))
}

fn format_thread_count(metric: &Metric<u64>) -> String {
    format!("{}{}", freshness_marker(metric), metric.value)
}

fn freshness_marker<T>(metric: &Metric<T>) -> &'static str {
    if metric_is_stale(metric) { "~" } else { "" }
}

fn format_percent_value(value: f32) -> String {
    format!("{value:.1}%")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MINIMUM_HEIGHT, MINIMUM_WIDTH, bar, chart_cell_budget, format_bytes, format_percent,
        header_lines, logical_cpu_grid_rows, logical_cpu_lines, percentage, process_name,
        process_table_row_capacity, render_sparkline, resource_readout, terminal_is_too_small,
    };
    use crate::app::{App, CpuDisplayMode, ProcessRow, ProcessViewMode};
    use crate::model::{
        CommandLine, Freshness, HistorySample, Metric, ProcessSnapshot, SystemSnapshot, UserSource,
    };
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    #[test]
    fn byte_values_use_compact_binary_units() {
        assert_eq!(format_bytes(42), "42B");
        assert_eq!(format_bytes(1_024), "1.0KiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0GiB");
    }

    #[test]
    fn stale_percentages_have_a_compact_marker() {
        let metric = Metric {
            value: 12.5,
            freshness: Freshness::Stale {
                reason: "sample failed".into(),
            },
        };

        assert_eq!(format_percent(&metric), "~12.5%");
    }

    #[test]
    fn minimum_terminal_size_is_enforced_in_both_dimensions() {
        assert!(terminal_is_too_small(MINIMUM_WIDTH - 1, MINIMUM_HEIGHT));
        assert!(terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT - 1));
        assert!(!terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT));
    }

    #[test]
    fn table_capacity_excludes_dashboard_and_table_chrome() {
        let app = App::new();
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 64, 12), &app), 1);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 64, 11), &app), 0);
        assert_eq!(
            process_table_row_capacity(Rect::new(0, 0, 64, 24), &app),
            13
        );
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 63, 24), &app), 0);
    }

    #[test]
    fn sparkline_is_right_aligned_and_drops_the_oldest_samples() {
        let samples = vec![
            history_sample(1.0),
            history_sample(50.0),
            history_sample(100.0),
        ];
        assert_eq!(render_sparkline(&samples, 5), "  ▁▅█");
        assert_eq!(render_sparkline(&samples, 2), "▅█");
        assert_eq!(render_sparkline(&samples, 0), "");
    }

    #[test]
    fn bars_use_semantic_colors_and_do_not_render_when_too_narrow() {
        let normal = bar(0.5, 6);
        assert_eq!(
            normal
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "[||  ]"
        );
        assert_eq!(normal[1].style.fg, Some(Color::Green));
        assert_eq!(bar(0.8, 6)[1].style.fg, Some(Color::Yellow));
        assert_eq!(bar(0.95, 6)[1].style.fg, Some(Color::Red));
        assert!(bar(0.5, 5).is_empty());
    }

    #[test]
    fn resource_readouts_include_usage_percentages() {
        let used = Metric::fresh(50_u64);
        let total = Metric::fresh(100_u64);
        assert_eq!(resource_readout(&used, &total), "50B/100B  50%");
        assert_eq!(percentage(&used, &total), 0.5);
    }

    #[test]
    fn charts_shrink_to_the_actual_readout_width_and_can_be_omitted() {
        assert_eq!(chart_cell_budget(58, 35), 13);
        assert_eq!(chart_cell_budget(40, 35), 0);
    }

    #[test]
    fn logical_cpu_mode_uses_a_responsive_grid_with_numbered_meters() {
        let lines = logical_cpu_lines(&[0.0, 50.0, 100.0, 25.0], 58);
        assert_eq!(logical_cpu_grid_rows(4, 58), 2);
        assert_eq!(lines.len(), 2);
        let first_line = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(first_line.contains(" 0 ["));
        assert!(first_line.contains(" 1 ["));
        let second_line = lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(second_line.contains(" 2 ["));
        assert!(second_line.contains(" 3 ["));
    }

    #[test]
    fn header_never_wraps_at_the_documented_minimum_width() {
        let system = SystemSnapshot {
            cpu_percent: Metric::fresh(100.0),
            logical_cpu_percentages: Metric::fresh(vec![0.0, 50.0, 100.0]),
            total_memory_bytes: Metric::fresh(u64::MAX),
            used_memory_bytes: Metric::stale(u64::MAX, "query failed"),
            commit_charge_bytes: Metric::stale(u64::MAX, "query failed"),
            commit_limit_bytes: Metric::fresh(u64::MAX),
        };

        for line in header_lines(Some(&system), None, 58, CpuDisplayMode::Summary)
            .into_iter()
            .chain(header_lines(
                Some(&system),
                None,
                58,
                CpuDisplayMode::LogicalCpus,
            ))
        {
            let width = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            assert!(width <= 58, "header line was {width} cells wide");
        }
    }

    fn history_sample(cpu_percent: f32) -> HistorySample {
        HistorySample { cpu_percent }
    }

    #[test]
    fn tree_names_use_branch_connectors_and_ancestor_guides() {
        let process = ProcessSnapshot {
            pid: 1,
            parent_pid: None,
            name: "worker.exe".into(),
            user: None,
            user_source: UserSource::Token,
            command_line: CommandLine::NotRequested,
            executable_path: None,
            cpu_percent: 0.0,
            thread_count: Metric::fresh(0),
            memory_bytes: 0,
        };
        let branch = ProcessRow {
            process: &process,
            ancestor_has_next_siblings: Vec::new(),
            is_last_sibling: false,
            has_children: true,
            is_expanded: true,
        };
        let leaf = ProcessRow {
            process: &process,
            ancestor_has_next_siblings: vec![true, false],
            is_last_sibling: true,
            has_children: false,
            is_expanded: false,
        };

        assert_eq!(process_name(&branch, ProcessViewMode::Tree), "▾ worker.exe");
        assert_eq!(
            process_name(&leaf, ProcessViewMode::Tree),
            "│       └── worker.exe"
        );
    }
}
