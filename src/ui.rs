use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::{
    app::{
        App, CpuDisplayMode, GpuDisplayMode, ProcessRow, ProcessViewMode, SortColumn, SortDirection,
    },
    model::{Freshness, History, Metric, Snapshot, SystemSnapshot},
    theme::{Palette, Theme},
};

const MINIMUM_WIDTH: u16 = 64;
const MINIMUM_HEIGHT: u16 = 26;
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
const MAX_CPU_HISTORY_ROWS: usize = 8;
const MIN_CPU_HISTORY_ROWS: usize = 6;
const MIN_PROCESS_ROWS: u16 = 1;
const FOOTER_HEIGHT: u16 = 3;
const TABLE_CHROME_HEIGHT: u16 = 3;

/// Renders the first-milestone dashboard from the newest completed snapshot.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let palette = app.theme().palette();
    frame.render_widget(
        Block::default().style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        ),
        area,
    );
    if terminal_is_too_small(area.width, area.height) {
        render_minimum_size_message(frame, palette);
        return;
    }

    let cpu_height = cpu_pane_height(area, app);
    let history_rows = cpu_history_rows(area, app);
    let resource_height = resource_pane_height(area.width);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(cpu_height),
            Constraint::Length(resource_height),
            Constraint::Min(5),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);

    let inner_width = sections[0].width.saturating_sub(2);
    let cpu = Paragraph::new(cpu_lines(
        app.snapshot().map(|snapshot| &snapshot.system),
        app.snapshot().map(|snapshot| &snapshot.history),
        usize::from(inner_width),
        app.cpu_display_mode(),
        history_rows,
        palette,
    ))
    .alignment(Alignment::Left)
    .block(
        Block::default()
            .title(cpu_pane_title(
                app.snapshot().map(|snapshot| &snapshot.system),
            ))
            .borders(Borders::ALL)
            .style(
                Style::default()
                    .fg(palette.foreground)
                    .bg(palette.background),
            )
            .border_style(Style::default().fg(palette.accent)),
    );
    frame.render_widget(cpu, sections[0]);
    render_resource_panes(frame, sections[1], app, palette);

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
    .style(
        Style::default()
            .fg(palette.foreground)
            .bg(palette.background),
    )
    .row_highlight_style(
        Style::default()
            .fg(palette.selection_foreground)
            .bg(palette.selection_background)
            .add_modifier(Modifier::BOLD),
    )
    .header(
        Row::new(["PID", "USER", "THREADS", "CPU", "MEMORY", "NAME"]).style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(process_table_title(app))
            .style(
                Style::default()
                    .fg(palette.foreground)
                    .bg(palette.background),
            )
            .border_style(Style::default().fg(palette.border)),
    );
    let mut table_state = TableState::default();
    table_state.select(app.selected_viewport_index());
    frame.render_stateful_widget(table, sections[2], &mut table_state);

    let footer = Paragraph::new(footer_text(app))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(
                    Style::default()
                        .fg(palette.foreground)
                        .bg(palette.background),
                )
                .border_style(Style::default().fg(palette.border)),
        );
    frame.render_widget(footer, sections[3]);

    if let Some(target) = app.termination_confirmation() {
        render_termination_confirmation(frame, target.pid, &target.name, palette);
    }
    if let Some(selected_theme) = app.theme_menu_selection() {
        render_theme_menu(frame, selected_theme, palette);
    }
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
                .saturating_sub(cpu_pane_height(area, app))
                .saturating_sub(resource_pane_height(area.width))
                .saturating_sub(FOOTER_HEIGHT + TABLE_CHROME_HEIGHT),
        )
    }
}

fn cpu_pane_height(area: Rect, app: &App) -> u16 {
    let inner_width = usize::from(area.width.saturating_sub(2));
    let content_height = match app.cpu_display_mode() {
        CpuDisplayMode::Summary => {
            let history_rows = cpu_history_rows(area, app);
            if history_rows == 0 {
                1
            } else {
                history_rows + 1
            }
        }
        CpuDisplayMode::LogicalCpus => {
            let cpu_count = app
                .snapshot()
                .map(|snapshot| snapshot.system.logical_cpu_percentages.value.len())
                .unwrap_or(0);
            logical_cpu_grid_rows(cpu_count, inner_width)
        }
    };
    u16::try_from(content_height.saturating_add(2)).unwrap_or(u16::MAX)
}

/// Uses a larger graph only when doing so leaves at least one process row.
/// A compact summary is more useful than a tall dashboard with no table.
fn cpu_history_rows(area: Rect, app: &App) -> usize {
    if app.cpu_display_mode() != CpuDisplayMode::Summary {
        return 0;
    }
    let reserved_height = resource_pane_height(area.width)
        .saturating_add(FOOTER_HEIGHT)
        .saturating_add(TABLE_CHROME_HEIGHT)
        .saturating_add(MIN_PROCESS_ROWS)
        .saturating_add(3); // CPU borders plus the time-axis row.
    let available_rows = usize::from(area.height.saturating_sub(reserved_height));
    if available_rows >= MAX_CPU_HISTORY_ROWS {
        MAX_CPU_HISTORY_ROWS
    } else if available_rows >= MIN_CPU_HISTORY_ROWS {
        MIN_CPU_HISTORY_ROWS
    } else {
        0
    }
}

fn resource_pane_height(width: u16) -> u16 {
    if width >= 110 {
        5
    } else if width >= 80 {
        10
    } else {
        12
    }
}

fn render_minimum_size_message(frame: &mut Frame, palette: Palette) {
    let message =
        format!("Terminal too small — minimum {MINIMUM_WIDTH} columns × {MINIMUM_HEIGHT} rows");
    let paragraph = Paragraph::new(message)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
        .block(
            Block::default()
                .title(" wtop ")
                .borders(Borders::ALL)
                .style(
                    Style::default()
                        .fg(palette.foreground)
                        .bg(palette.background),
                )
                .border_style(Style::default().fg(palette.accent)),
        );
    frame.render_widget(paragraph, frame.area());
}

fn render_termination_confirmation(frame: &mut Frame, pid: u32, name: &str, palette: Palette) {
    let area = frame.area();
    let width = area.width.min(52);
    let height = 7;
    let dialog_area = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let prompt = format!("Terminate {name} (PID {pid})?\n\nThis cannot be undone.  y / n");
    let dialog = Paragraph::new(prompt)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
        .block(
            Block::default()
                .title(" Confirm process termination ")
                .borders(Borders::ALL)
                .style(
                    Style::default()
                        .fg(palette.foreground)
                        .bg(palette.background),
                )
                .border_style(Style::default().fg(palette.critical)),
        );
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(dialog, dialog_area);
}

fn render_theme_menu(frame: &mut Frame, selected_theme: Theme, palette: Palette) {
    let area = frame.area();
    let width = area.width.min(34);
    let height = 13;
    let dialog_area = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let lines = Theme::ALL
        .iter()
        .map(|theme| {
            let marker = if *theme == selected_theme { '›' } else { ' ' };
            let style = if *theme == selected_theme {
                Style::default()
                    .fg(palette.selection_foreground)
                    .bg(palette.selection_background)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(palette.foreground)
                    .bg(palette.background)
            };
            Line::from(Span::styled(format!(" {marker} {}", theme.label()), style))
        })
        .collect::<Vec<_>>();
    let dialog = Paragraph::new(lines)
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
        .block(
            Block::default()
                .title(" Themes ")
                .title_bottom(" Up/Down preview  Enter apply  Esc cancel ")
                .borders(Borders::ALL)
                .style(
                    Style::default()
                        .fg(palette.foreground)
                        .bg(palette.background),
                )
                .border_style(Style::default().fg(palette.accent)),
        );
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(dialog, dialog_area);
}

fn cpu_pane_title(system: Option<&SystemSnapshot>) -> String {
    let readout = system.map_or_else(
        || "--%".into(),
        |system| format_percent(&system.cpu_percent),
    );
    format!(" CPU · {readout} ")
}

/// Renders either aggregate CPU history or a logical-CPU grid.
fn cpu_lines(
    system: Option<&SystemSnapshot>,
    history: Option<&History>,
    inner_width: usize,
    cpu_display_mode: CpuDisplayMode,
    history_rows: usize,
    palette: Palette,
) -> Vec<Line<'static>> {
    let Some(system) = system else {
        return if cpu_display_mode == CpuDisplayMode::Summary {
            if history_rows == 0 {
                vec![Line::from("Collecting CPU history…")]
            } else {
                let mut lines = vec![Line::from("Collecting CPU history…")];
                lines.resize(history_rows + 1, Line::default());
                lines
            }
        } else {
            vec![Line::from("Logical CPUs  --")]
        };
    };

    match cpu_display_mode {
        CpuDisplayMode::Summary if history_rows > 0 => {
            cpu_history_lines(history, inner_width, history_rows, palette)
        }
        CpuDisplayMode::Summary => vec![resource_line(
            "CPU",
            &format_percent(&system.cpu_percent),
            display_width(&format_percent(&system.cpu_percent)),
            Vec::new(),
            palette,
        )],
        CpuDisplayMode::LogicalCpus => {
            logical_cpu_lines(&system.logical_cpu_percentages.value, inner_width, palette)
        }
    }
}

fn cpu_history_lines(
    history: Option<&History>,
    width: usize,
    height: usize,
    palette: Palette,
) -> Vec<Line<'static>> {
    let samples = history
        .map(|history| history.samples().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut lines = (1..=height)
        .rev()
        .map(|level| {
            let mut spans = Vec::with_capacity(width);
            spans.extend((0..width).map(|column| {
                let sample = samples.get(column * samples.len() / width.max(1));
                let Some(sample) = sample else {
                    return Span::raw(" ");
                };
                let filled_rows = ((sample.cpu_percent.clamp(0.0, 100.0) / 100.0) * height as f32)
                    .round() as usize;
                if filled_rows >= level {
                    Span::styled(
                        BAR_TICK.to_string(),
                        Style::default()
                            .fg(usage_color(f64::from(sample.cpu_percent) / 100.0, palette)),
                    )
                } else {
                    Span::raw(" ")
                }
            }));
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    lines.push(Line::from(vec![
        Span::styled("60s ago", Style::default().fg(palette.muted)),
        Span::raw(" ".repeat(width.saturating_sub(10))),
        Span::styled("now", Style::default().fg(palette.muted)),
    ]));
    lines
}

fn render_resource_panes(frame: &mut Frame, area: Rect, app: &App, palette: Palette) {
    let system = app.snapshot().map(|snapshot| &snapshot.system);
    let panes = if area.width >= 110 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(area)
            .to_vec()
    } else if area.width >= 80 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Length(5)])
            .split(area);
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        vec![top[0], top[1], rows[1]]
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(4),
                Constraint::Length(4),
            ])
            .split(area)
            .to_vec()
    };
    render_memory_pane(frame, panes[0], system, palette);
    render_gpu_pane(frame, panes[1], system, app.gpu_display_mode(), palette);
    render_network_pane(frame, panes[2], system, palette);
}

fn render_memory_pane(
    frame: &mut Frame,
    area: Rect,
    system: Option<&SystemSnapshot>,
    palette: Palette,
) {
    let lines = system.map_or_else(
        || vec![Line::from("RAM     -- / --"), Line::from("Commit  -- / --")],
        |system| {
            let ram = resource_readout(&system.used_memory_bytes, &system.total_memory_bytes);
            let commit = resource_readout(&system.commit_charge_bytes, &system.commit_limit_bytes);
            let value_width = display_width(&ram).max(display_width(&commit));
            let chart_width =
                chart_cell_budget(usize::from(area.width.saturating_sub(2)), value_width);
            vec![
                resource_line(
                    "RAM",
                    &ram,
                    value_width,
                    bar(
                        percentage(&system.used_memory_bytes, &system.total_memory_bytes),
                        chart_width,
                        palette,
                    ),
                    palette,
                ),
                resource_line(
                    "Commit",
                    &commit,
                    value_width,
                    bar(
                        percentage(&system.commit_charge_bytes, &system.commit_limit_bytes),
                        chart_width,
                        palette,
                    ),
                    palette,
                ),
            ]
        },
    );
    frame.render_widget(resource_block(" Memory ", lines, palette), area);
}

fn render_gpu_pane(
    frame: &mut Frame,
    area: Rect,
    system: Option<&SystemSnapshot>,
    display: GpuDisplayMode,
    palette: Palette,
) {
    let Some(system) = system else {
        frame.render_widget(
            resource_block(
                " GPU ",
                vec![Line::from("Collecting GPU adapters…")],
                palette,
            ),
            area,
        );
        return;
    };
    let adapters = &system.gpu.adapters.value;
    if adapters.is_empty() {
        frame.render_widget(
            resource_block(
                " GPU ",
                vec![Line::from("No hardware GPU detected")],
                palette,
            ),
            area,
        );
        return;
    }
    let selected = match display {
        GpuDisplayMode::Overview => None,
        GpuDisplayMode::Adapter(id) => adapters.iter().find(|adapter| adapter.id == id),
    };
    if let Some(adapter) = selected {
        let position = adapters
            .iter()
            .position(|candidate| candidate.id == adapter.id)
            .unwrap_or(0)
            + 1;
        let title = format!(" GPU — {} ({}/{}) ", adapter.name, position, adapters.len());
        let utilization = format_optional_percent(&adapter.utilization_percent);
        let usage = f64::from(adapter.utilization_percent.value.unwrap_or(0.0)) / 100.0;
        let width = chart_cell_budget_with_label(
            usize::from(area.width.saturating_sub(2)),
            12,
            display_width(&utilization),
        );
        let dedicated = optional_memory_readout(
            &adapter.dedicated_memory_used_bytes,
            &adapter.dedicated_memory_capacity_bytes,
        );
        let shared = optional_memory_readout(
            &adapter.shared_memory_used_bytes,
            &adapter.shared_memory_capacity_bytes,
        );
        frame.render_widget(
            resource_block(
                title,
                vec![
                    resource_line_with_label_width(
                        "Utilization",
                        12,
                        &utilization,
                        display_width(&utilization),
                        bar(usage, width, palette),
                        palette,
                    ),
                    Line::from(format!("Dedicated  {dedicated}")),
                    Line::from(format!("Shared     {shared}")),
                ],
                palette,
            ),
            area,
        );
    } else {
        let mut lines = adapters
            .iter()
            .take(usize::from(area.height.saturating_sub(2)))
            .map(|adapter| {
                Line::from(format!(
                    "{:<18.18} {:>6}",
                    adapter.name,
                    format_optional_percent(&adapter.utilization_percent)
                ))
            })
            .collect::<Vec<_>>();
        if adapters.len() > lines.len() {
            lines.push(Line::from(format!(
                "+{} more adapters",
                adapters.len() - lines.len()
            )));
        }
        let title = format!(" GPU · {} adapters ", adapters.len());
        frame.render_widget(resource_block(title, lines, palette), area);
    }
}

fn render_network_pane(
    frame: &mut Frame,
    area: Rect,
    system: Option<&SystemSnapshot>,
    palette: Palette,
) {
    let Some(system) = system else {
        frame.render_widget(
            resource_block(
                " Network ",
                vec![Line::from("Collecting interface counters…")],
                palette,
            ),
            area,
        );
        return;
    };

    let network = &system.network;
    let mut lines = vec![network_line(
        "Total",
        &network.total_transmit_bytes_per_second,
        &network.total_receive_bytes_per_second,
    )];
    let available_rows = usize::from(area.height.saturating_sub(3));
    lines.extend(
        network
            .interfaces
            .value
            .iter()
            .take(available_rows.saturating_sub(1))
            .map(|interface| {
                network_line(
                    &interface.alias,
                    &interface.transmit_bytes_per_second,
                    &interface.receive_bytes_per_second,
                )
            }),
    );
    if network.interfaces.value.is_empty() {
        lines.push(Line::from("No operational interfaces"));
    }
    frame.render_widget(resource_block(" Network ", lines, palette), area);
}

fn resource_block(
    title: impl Into<Line<'static>>,
    lines: Vec<Line<'static>>,
    palette: Palette,
) -> Paragraph<'static> {
    Paragraph::new(lines)
        .style(
            Style::default()
                .fg(palette.foreground)
                .bg(palette.background),
        )
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .style(
                    Style::default()
                        .fg(palette.foreground)
                        .bg(palette.background),
                )
                .border_style(Style::default().fg(palette.border)),
        )
        .wrap(Wrap { trim: true })
}

fn network_line(
    alias: &str,
    transmit: &Metric<Option<f64>>,
    receive: &Metric<Option<f64>>,
) -> Line<'static> {
    Line::from(format!(
        "{alias:<12.12} ↑ {:>9}  ↓ {:>9}",
        format_rate(transmit),
        format_rate(receive),
    ))
}

fn logical_cpu_grid_rows(cpu_count: usize, inner_width: usize) -> usize {
    let columns = logical_cpu_columns(inner_width);
    cpu_count.div_ceil(columns).max(1)
}

fn logical_cpu_lines(
    percentages: &[f32],
    inner_width: usize,
    palette: Palette,
) -> Vec<Line<'static>> {
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
                    palette,
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

fn logical_cpu_meter(
    index: usize,
    percent: f32,
    width: usize,
    palette: Palette,
) -> Vec<Span<'static>> {
    let percent = percent.clamp(0.0, 100.0);
    let label = format!("CPU {index:>2} ");
    let readout = format!("{percent:.1}%");
    let bar_width = width.saturating_sub(label.len());
    let inner_width = bar_width.saturating_sub(2);
    let tick_width = inner_width.saturating_sub(readout.len() + 1);
    let filled = ((f64::from(percent) / 100.0) * tick_width as f64).round() as usize;
    let color = usage_color(f64::from(percent) / 100.0, palette);

    vec![
        Span::raw(label),
        Span::styled("[", Style::default().fg(palette.muted)),
        Span::styled(
            BAR_TICK.to_string().repeat(filled.min(tick_width)),
            Style::default().fg(color),
        ),
        Span::styled(
            " ".repeat(tick_width.saturating_sub(filled)),
            Style::default().fg(palette.muted),
        ),
        Span::raw(" "),
        Span::styled(readout, Style::default().fg(palette.foreground)),
        Span::styled("]", Style::default().fg(palette.muted)),
    ]
}

fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn chart_cell_budget(inner_width: usize, value_width: usize) -> usize {
    chart_cell_budget_with_label(inner_width, LABEL_WIDTH, value_width)
}

fn chart_cell_budget_with_label(
    inner_width: usize,
    label_width: usize,
    value_width: usize,
) -> usize {
    inner_width.saturating_sub(label_width + value_width + COLUMN_GAP)
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
    palette: Palette,
) -> Line<'static> {
    resource_line_with_label_width(label, LABEL_WIDTH, readout, value_width, chart, palette)
}

fn resource_line_with_label_width(
    label: &str,
    label_width: usize,
    readout: &str,
    value_width: usize,
    chart: Vec<Span<'static>>,
    palette: Palette,
) -> Line<'static> {
    let mut spans = vec![
        Span::raw(format!("{label:<label_width$}")),
        Span::styled(
            format!("{readout:>value_width$}"),
            Style::default().fg(palette.foreground),
        ),
    ];
    if !chart.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(chart);
    }
    Line::from(spans)
}

fn bar(fraction: f64, width: usize, palette: Palette) -> Vec<Span<'static>> {
    if width < MIN_BAR_CELLS as usize {
        return Vec::new();
    }

    let inner_width = width - 2;
    let segment_count = inner_width;
    let filled_segments = (fraction * segment_count as f64).round() as usize;
    let filled_width = filled_segments.min(segment_count);
    let color = usage_color(fraction, palette);
    vec![
        Span::styled("[", Style::default().fg(palette.muted)),
        Span::styled(
            BAR_TICK.to_string().repeat(filled_width),
            Style::default().fg(color),
        ),
        Span::styled(
            " ".repeat(inner_width - filled_width),
            Style::default().fg(palette.muted),
        ),
        Span::styled("]", Style::default().fg(palette.muted)),
    ]
}

fn usage_color(fraction: f64, palette: Palette) -> Color {
    match fraction {
        fraction if fraction >= 0.9 => palette.critical,
        fraction if fraction >= 0.75 => palette.warning,
        _ => palette.good,
    }
}

fn percentage(used: &Metric<u64>, total: &Metric<u64>) -> f64 {
    if total.value == 0 {
        0.0
    } else {
        (used.value as f64 / total.value as f64).clamp(0.0, 1.0)
    }
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
        return row.process.name.clone();
    }

    let ancestor_guides = row
        .ancestor_has_next_siblings
        .iter()
        .map(|has_next_sibling| if *has_next_sibling { "│   " } else { "    " })
        .collect::<String>();
    let branch = if row.is_last_sibling { '└' } else { '├' };
    format!("{ancestor_guides}{branch}── {}", row.process.name)
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

    let status_prefix = app
        .status_message()
        .map(|message| format!("{message}  "))
        .unwrap_or_default();

    if app.is_filter_editing() {
        format!(
            "{stale_prefix}{status_prefix}/{}  Enter apply  Esc cancel  Ctrl+U clear",
            app.filter()
        )
    } else if app.filter().is_empty() {
        if app.view_mode() == ProcessViewMode::Tree {
            format!(
                "{stale_prefix}{status_prefix}c CPU  g GPU  t Flat  o Theme  x Kill  s Sort  / Filter  q Quit"
            )
        } else {
            format!(
                "{stale_prefix}{status_prefix}c CPU  g GPU  t Tree  o Theme  x Kill  s Sort  S Reverse  / Filter  q Quit"
            )
        }
    } else {
        format!(
            "{stale_prefix}{status_prefix}Filter: {}  c CPU  g GPU  t View  o Theme  x Kill  s Sort  S Reverse  / Edit  q Quit",
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
        || metric_is_stale(&snapshot.system.network.interfaces)
        || metric_is_stale(&snapshot.system.network.total_transmit_bytes_per_second)
        || metric_is_stale(&snapshot.system.network.total_receive_bytes_per_second)
        || snapshot
            .system
            .network
            .interfaces
            .value
            .iter()
            .any(|interface| {
                metric_is_stale(&interface.transmit_bytes_per_second)
                    || metric_is_stale(&interface.receive_bytes_per_second)
            })
        || metric_is_stale(&snapshot.system.gpu.adapters)
        || snapshot.system.gpu.adapters.value.iter().any(|adapter| {
            metric_is_stale(&adapter.utilization_percent)
                || metric_is_stale(&adapter.dedicated_memory_used_bytes)
                || metric_is_stale(&adapter.dedicated_memory_capacity_bytes)
                || metric_is_stale(&adapter.shared_memory_used_bytes)
                || metric_is_stale(&adapter.shared_memory_capacity_bytes)
        })
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

fn format_rate(metric: &Metric<Option<f64>>) -> String {
    let marker = freshness_marker(metric);
    let Some(rate) = metric.value.filter(|rate| rate.is_finite() && *rate >= 0.0) else {
        return format!("{marker}—");
    };
    format!("{marker}{}/s", format_bytes(rate.round() as u64))
}

fn format_optional_percent(metric: &Metric<Option<f32>>) -> String {
    metric.value.map_or_else(
        || format!("{}—", freshness_marker(metric)),
        |value| {
            format!(
                "{}{}",
                freshness_marker(metric),
                format_percent_value(value)
            )
        },
    )
}

fn optional_memory_readout(used: &Metric<Option<u64>>, capacity: &Metric<Option<u64>>) -> String {
    match (used.value, capacity.value) {
        (Some(used), Some(capacity)) => {
            format!("{}/{}", format_bytes(used), format_bytes(capacity))
        }
        (None, Some(capacity)) => format!("—/{}", format_bytes(capacity)),
        _ => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MINIMUM_HEIGHT, MINIMUM_WIDTH, bar, chart_cell_budget, cpu_history_lines, cpu_history_rows,
        cpu_lines, format_bytes, format_percent, logical_cpu_grid_rows, logical_cpu_lines,
        percentage, process_name, process_table_row_capacity, resource_readout,
        terminal_is_too_small,
    };
    use crate::app::{App, CpuDisplayMode, ProcessRow, ProcessViewMode};
    use crate::model::{
        CommandLine, Freshness, History, HistorySample, Metric, NetworkSnapshot, ProcessSnapshot,
        SystemSnapshot, UserSource,
    };
    use crate::theme::Theme;
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
        let mut app = App::new();
        app.toggle_cpu_display_mode();
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 64, 26), &app), 5);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 64, 25), &app), 0);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 64, 28), &app), 1);
        assert_eq!(process_table_row_capacity(Rect::new(0, 0, 63, 28), &app), 0);
    }

    #[test]
    fn cpu_history_grows_only_when_the_process_table_keeps_a_row() {
        let mut app = App::new();
        app.toggle_cpu_display_mode();
        assert_eq!(cpu_history_rows(Rect::new(0, 0, 64, 26), &app), 0);
        assert_eq!(cpu_history_rows(Rect::new(0, 0, 64, 28), &app), 6);
        assert_eq!(cpu_history_rows(Rect::new(0, 0, 64, 30), &app), 8);
    }

    #[test]
    fn cpu_history_stretches_tick_columns_across_the_available_width() {
        let mut history = History::with_capacity(3);
        history.push(history_sample(25.0));
        history.push(history_sample(50.0));
        history.push(history_sample(100.0));
        let lines = cpu_history_lines(Some(&history), 5, 4, Theme::Default.palette());
        assert_eq!(lines.len(), 5);
        let top_row = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(top_row, "    |");
        let bottom_row = lines[3]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(bottom_row, "|||||");
    }

    #[test]
    fn bars_use_semantic_colors_and_do_not_render_when_too_narrow() {
        let palette = Theme::Default.palette();
        let normal = bar(0.5, 6, palette);
        assert_eq!(
            normal
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "[||  ]"
        );
        assert_eq!(normal[1].style.fg, Some(Color::Green));
        assert_eq!(bar(0.8, 6, palette)[1].style.fg, Some(Color::Yellow));
        assert_eq!(bar(0.95, 6, palette)[1].style.fg, Some(Color::Red));
        assert!(bar(0.5, 5, palette).is_empty());
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
        let lines = logical_cpu_lines(&[0.0, 50.0, 100.0, 25.0], 58, Theme::Default.palette());
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
            network: NetworkSnapshot::default(),
            gpu: Default::default(),
        };

        let palette = Theme::Default.palette();
        for line in cpu_lines(Some(&system), None, 58, CpuDisplayMode::Summary, 8, palette)
            .into_iter()
            .chain(cpu_lines(
                Some(&system),
                None,
                58,
                CpuDisplayMode::LogicalCpus,
                0,
                palette,
            ))
        {
            let width = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            assert!(width <= 58, "header line was {width} cells wide: {line:?}");
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
        };
        let leaf = ProcessRow {
            process: &process,
            ancestor_has_next_siblings: vec![true, false],
            is_last_sibling: true,
            has_children: false,
        };

        assert_eq!(process_name(&branch, ProcessViewMode::Tree), "worker.exe");
        assert_eq!(
            process_name(&leaf, ProcessViewMode::Tree),
            "│       └── worker.exe"
        );
    }
}
