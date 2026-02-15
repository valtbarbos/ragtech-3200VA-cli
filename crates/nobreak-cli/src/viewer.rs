use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use nobreak_core::{Monitor, Snapshot, UpsDriver};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, Paragraph};
use ratatui::Terminal;

const METRIC_KEYS: [(&str, &str, Color); 7] = [
    ("vInput", "VInput (V)", Color::Yellow),
    ("vOutput", "VOutput (V)", Color::Cyan),
    ("vBattery", "VBattery (V)", Color::Green),
    ("cBattery", "CBattery (%)", Color::Magenta),
    ("fOutput", "FOutput (Hz)", Color::Blue),
    ("temperature", "Temp (C)", Color::Red),
    ("pOutput", "POutput (%)", Color::LightYellow),
];

struct MetricSeries {
    label: &'static str,
    color: Color,
    points: VecDeque<(f64, f64)>,
}

impl MetricSeries {
    fn new(label: &'static str, color: Color) -> Self {
        Self {
            label,
            color,
            points: VecDeque::new(),
        }
    }

    fn push(&mut self, x: f64, y: f64, window_sec: f64) {
        self.points.push_back((x, y));
        while let Some((old_x, _)) = self.points.front() {
            if x - old_x > window_sec {
                self.points.pop_front();
            } else {
                break;
            }
        }
    }

    fn bounds(&self) -> [f64; 2] {
        if self.points.is_empty() {
            return [0.0, 1.0];
        }
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for (_, y) in &self.points {
            min = min.min(*y);
            max = max.max(*y);
        }
        if (max - min).abs() < f64::EPSILON {
            [min - 1.0, max + 1.0]
        } else {
            let pad = (max - min) * 0.12;
            [min - pad, max + pad]
        }
    }
}

struct ViewerState {
    start: Instant,
    latest: Option<Snapshot>,
    series: Vec<MetricSeries>,
}

impl ViewerState {
    fn new() -> Self {
        let series = METRIC_KEYS
            .iter()
            .map(|(_, label, color)| MetricSeries::new(label, *color))
            .collect();

        Self {
            start: Instant::now(),
            latest: None,
            series,
        }
    }

    fn update(&mut self, snapshot: Snapshot, window_sec: f64) {
        let t = self.start.elapsed().as_secs_f64();
        for (idx, (key, _, _)) in METRIC_KEYS.iter().enumerate() {
            if let Some(value) = snapshot.vars.get(*key).and_then(|v| v.as_f64()) {
                self.series[idx].push(t, value, window_sec);
            }
        }
        self.latest = Some(snapshot);
    }
}

pub async fn run_viewer<D: UpsDriver>(monitor: &mut Monitor<D>, window_sec: f64) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = ViewerState::new();
    let mut next_tick = Instant::now();
    let mut command_buffer = String::new();

    let run_result = async {
        loop {
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char(c) => {
                            command_buffer.push(c.to_ascii_lowercase());
                            if command_buffer.len() > 8 {
                                let drain = command_buffer.len() - 8;
                                command_buffer.drain(0..drain);
                            }
                            if command_buffer.ends_with("exit") {
                                break;
                            }
                        }
                        KeyCode::Backspace => {
                            command_buffer.pop();
                        }
                        _ => {}
                    }
                }
            }

            if Instant::now() >= next_tick {
                let snapshot = monitor.tick().await;
                let interval = monitor.effective_interval();
                state.update(snapshot, window_sec);
                next_tick = Instant::now() + interval;
            }

            terminal.draw(|frame| draw_ui(frame.size(), frame, &state, window_sec))?;
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    run_result
}

fn draw_ui(area: Rect, frame: &mut ratatui::Frame<'_>, state: &ViewerState, window_sec: f64) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let header = render_header(state, window_sec);
    frame.render_widget(header, rows[0]);

    let chart_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[1]);

    let mut idx = 0;
    for row_area in chart_rows.iter().copied() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(row_area);

        for col in cols.iter().copied() {
            if idx < state.series.len() {
                render_metric_chart(frame, col, &state.series[idx], state.start.elapsed().as_secs_f64(), window_sec);
            } else {
                let empty = Paragraph::new(Line::from(" "));
                frame.render_widget(empty, col);
            }
            idx += 1;
        }
    }
}

fn render_header(state: &ViewerState, window_sec: f64) -> Paragraph<'static> {
    let mut lines = Vec::new();
    if let Some(snapshot) = &state.latest {
        let status = format!(
            "connected={} stale={} age_ms={} rtt_ms={} status={} confidence={}",
            snapshot.device.connected,
            snapshot.freshness.stale,
            snapshot.freshness.age_ms,
            snapshot.freshness.rtt_ms,
            snapshot.status.code,
            snapshot
                .vars
                .get("metricsConfidence")
                .and_then(|v| v.as_str())
                .unwrap_or("n/a")
        );
        let device = format!(
            "{} {} [{}:{}]  window={}s  (press 'q' to quit)",
            snapshot.device.model,
            snapshot.device.transport.path,
            snapshot.device.transport.vid,
            snapshot.device.transport.pid,
            window_sec as u64
        );
        lines.push(Line::from(vec![
            Span::styled("Nobreak Graph Viewer  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(status),
        ]));
        lines.push(Line::from(device));
    } else {
        lines.push(Line::from("Waiting first snapshot..."));
    }

    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Status"))
}

fn render_metric_chart(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    series: &MetricSeries,
    now_sec: f64,
    window_sec: f64,
) {
    let points: Vec<(f64, f64)> = series.points.iter().copied().collect();

    let x_min = (now_sec - window_sec).max(0.0);
    let x_max = now_sec.max(window_sec);
    let y_bounds = series.bounds();

    let dataset = Dataset::default()
        .name(series.label)
        .marker(symbols::Marker::Braille)
        .graph_type(ratatui::widgets::GraphType::Line)
        .style(Style::default().fg(series.color))
        .data(&points);

    let x_mid = (x_min + x_max) / 2.0;

    let chart = Chart::new(vec![dataset])
        .block(Block::default().borders(Borders::ALL).title(series.label))
        .x_axis(
            Axis::default()
                .title("time (s)")
                .style(Style::default().fg(Color::Gray))
                .bounds([x_min, x_max])
                .labels(vec![
                    Span::raw(format!("{x_min:.0}")),
                    Span::raw(format!("{x_mid:.0}")),
                    Span::raw(format!("{x_max:.0}")),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("value")
                .style(Style::default().fg(Color::Gray))
                .bounds(y_bounds)
                .labels(vec![
                    Span::raw(format!("{:.1}", y_bounds[0])),
                    Span::raw(format!("{:.1}", (y_bounds[0] + y_bounds[1]) / 2.0)),
                    Span::raw(format!("{:.1}", y_bounds[1])),
                ]),
        );

    frame.render_widget(chart, area);
}
