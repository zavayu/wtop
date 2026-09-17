use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

use crate::{
    app::{App, ProcessRow, ProcessViewMode, SortColumn, SortDirection},
    model::{Freshness, Metric, ProcessSnapshot, Snapshot, SystemSnapshot},
    text::scroll_text,
};

const MINIMUM_WIDTH: u16 = 60;
const MINIMUM_HEIGHT: u16 = 10;
const PID_COLUMN_WIDTH: u16 = 7;
const NAME_COLUMN_WIDTH: u16 = 16;
const CPU_COLUMN_WIDTH: u16 = 7;
const MEMORY_COLUMN_WIDTH: u16 = 10;
const COLUMN_SPACING: u16 = 1;
const TABLE_BORDER_WIDTH: u16 = 2;
const FIXED_COLUMN_WIDTH: u16 =
    PID_COLUMN_WIDTH + NAME_COLUMN_WIDTH + CPU_COLUMN_WIDTH + MEMORY_COLUMN_WIDTH;
const COLUMN_GAP_WIDTH: u16 = COLUMN_SPACING * 4;

/// Renders the first-milestone dashboard from the newest completed snapshot.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if terminal_is_too_small(area.width, area.height) {
        render_minimum_size_message(frame);
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(system_summary(
        app.snapshot().map(|snapshot| &snapshot.system),
    ))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .title(" wtop ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, sections[0]);

    let command_line_width = command_line_viewport_cells(area);
    let table = Table::new(
        app.viewport_process_rows().into_iter().map(|row| {
            process_row(
                &row,
                app.view_mode(),
                app.selected_pid(),
                app.command_line_offset_cells(),
                command_line_width,
            )
        }),
        [
            Constraint::Length(PID_COLUMN_WIDTH),
            Constraint::Length(NAME_COLUMN_WIDTH),
            Constraint::Length(CPU_COLUMN_WIDTH),
            Constraint::Length(MEMORY_COLUMN_WIDTH),
            Constraint::Min(8),
        ],
    )
    .column_spacing(COLUMN_SPACING)
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
    .header(
        Row::new(["PID", "NAME", "CPU", "MEMORY", "COMMAND LINE"]).style(
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
/// The dashboard reserves three rows each for the header and footer. The
/// table itself uses a top and bottom border plus one header row.
pub fn process_table_row_capacity(area: Rect) -> usize {
    if terminal_is_too_small(area.width, area.height) {
        0
    } else {
        usize::from(area.height.saturating_sub(9))
    }
}

/// Returns the command-line cell width after fixed columns, gaps, and table
/// borders have been allocated.
pub fn command_line_viewport_cells(area: Rect) -> u16 {
    if terminal_is_too_small(area.width, area.height) {
        0
    } else {
        area.width
            .saturating_sub(TABLE_BORDER_WIDTH + FIXED_COLUMN_WIDTH + COLUMN_GAP_WIDTH)
    }
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

fn system_summary(system: Option<&SystemSnapshot>) -> String {
    let Some(system) = system else {
        return "CPU: --%  Mem: -- / --  Commit: -- / --".into();
    };

    format!(
        "CPU: {}  Mem: {}/{}  Commit: {}/{}",
        format_percent(&system.cpu_percent),
        format_metric_bytes(&system.used_memory_bytes),
        format_metric_bytes(&system.total_memory_bytes),
        format_metric_bytes(&system.commit_charge_bytes),
        format_metric_bytes(&system.commit_limit_bytes),
    )
}

fn process_row(
    row: &ProcessRow<'_>,
    view_mode: ProcessViewMode,
    selected_pid: Option<u32>,
    selected_command_line_offset: u16,
    command_line_width: u16,
) -> Row<'static> {
    let process = row.process;
    let command_line_offset = if selected_pid == Some(process.pid) {
        selected_command_line_offset
    } else {
        0
    };
    Row::new(vec![
        Cell::from(process.pid.to_string()),
        Cell::from(process_name(row, view_mode)),
        Cell::from(format_percent_value(process.cpu_percent)),
        Cell::from(format_bytes(process.memory_bytes)),
        Cell::from(scroll_text(
            &format_command_line(process),
            command_line_offset,
            command_line_width,
        )),
    ])
}

fn process_name(row: &ProcessRow<'_>, view_mode: ProcessViewMode) -> String {
    if view_mode == ProcessViewMode::Flat {
        return row.process.name.clone();
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
            format!("{stale_prefix}t Flat  Enter/Space Collapse  s Sort  / Filter  q Quit")
        } else {
            format!("{stale_prefix}t Tree  s Sort  S Reverse  / Filter  q Quit")
        }
    } else {
        format!(
            "{stale_prefix}Filter: {}  t View  s Sort  S Reverse  / Edit  q Quit",
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
        || metric_is_stale(&snapshot.system.total_memory_bytes)
        || metric_is_stale(&snapshot.system.used_memory_bytes)
        || metric_is_stale(&snapshot.system.commit_charge_bytes)
        || metric_is_stale(&snapshot.system.commit_limit_bytes)
        || metric_is_stale(&snapshot.processes)
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

fn format_command_line(process: &ProcessSnapshot) -> String {
    process.command_line_display()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        MINIMUM_HEIGHT, MINIMUM_WIDTH, command_line_viewport_cells, format_bytes,
        format_command_line, format_percent, process_name, process_table_row_capacity,
        terminal_is_too_small,
    };
    use crate::app::{ProcessRow, ProcessViewMode};
    use crate::model::{CommandLine, Freshness, Metric, ProcessSnapshot};
    use ratatui::layout::Rect;

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
    fn unavailable_command_lines_fall_back_to_the_executable_path() {
        let process = ProcessSnapshot {
            pid: 1,
            parent_pid: None,
            name: "example.exe".into(),
            command_line: CommandLine::Unavailable,
            executable_path: Some(PathBuf::from(r"C:\Tools\example.exe")),
            cpu_percent: 0.0,
            memory_bytes: 0,
        };

        assert_eq!(
            format_command_line(&process),
            r"<unavailable> C:\Tools\example.exe"
        );
    }

    #[test]
    fn minimum_terminal_size_is_enforced_in_both_dimensions() {
        assert!(terminal_is_too_small(MINIMUM_WIDTH - 1, MINIMUM_HEIGHT));
        assert!(terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT - 1));
        assert!(!terminal_is_too_small(MINIMUM_WIDTH, MINIMUM_HEIGHT));
    }

    #[test]
    fn table_capacity_excludes_dashboard_and_table_chrome() {
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 80, 10)), 1);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 80, 24)), 15);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 59, 24)), 0);
    }

    #[test]
    fn command_line_viewport_leaves_the_fixed_columns_in_place() {
        assert_eq!(command_line_viewport_cells(Rect::new(0, 0, 60, 10)), 14);
        assert_eq!(command_line_viewport_cells(Rect::new(0, 0, 100, 24)), 54);
        assert_eq!(command_line_viewport_cells(Rect::new(0, 0, 59, 24)), 0);
    }

    #[test]
    fn tree_names_use_branch_connectors_and_ancestor_guides() {
        let process = ProcessSnapshot {
            pid: 1,
            parent_pid: None,
            name: "worker.exe".into(),
            command_line: CommandLine::NotRequested,
            executable_path: None,
            cpu_percent: 0.0,
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

        assert_eq!(
            process_name(&branch, ProcessViewMode::Tree),
            "├─▾ worker.exe"
        );
        assert_eq!(
            process_name(&leaf, ProcessViewMode::Tree),
            "│       └── worker.exe"
        );
    }
}
