//! Additional TUI view components

#![allow(dead_code)]

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

pub fn draw_progress(f: &mut Frame, area: Rect, title: &str, progress: f64, label: &str) {
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(progress.clamp(0.0, 1.0))
        .label(label);

    f.render_widget(gauge, area);
}

pub fn draw_notification(f: &mut Frame, area: Rect, title: &str, message: &str, is_error: bool) {
    let style = if is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let text = vec![
        Line::from(Span::styled(title, style.add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(message),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(style)
            .title("Notice"))
        .style(Style::default().bg(Color::Black));

    f.render_widget(paragraph, area);
}

pub fn draw_switch_countdown(
    f: &mut Frame,
    area: Rect,
    from_tool: &str,
    to_tool: &str,
    countdown: u8,
) {
    let text = vec![
        Line::from(Span::styled(
            format!("{} hit rate limit", from_tool),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Switching to {} in {}s...", to_tool, countdown)),
        Line::from(""),
        Line::from(Span::styled(
            "Press Ctrl+C to cancel",
            Style::default().fg(Color::Gray),
        )),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title("Tool Switch"));

    f.render_widget(paragraph, area);
}

pub fn draw_connection_status(f: &mut Frame, area: Rect, connected: bool, server: &str) {
    let (status_text, style) = if connected {
        ("Connected", Style::default().fg(Color::Green))
    } else {
        ("Disconnected", Style::default().fg(Color::Red))
    };

    let text = vec![
        Line::from(vec![
            Span::raw("Status: "),
            Span::styled(status_text, style.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("Server: "),
            Span::styled(server, Style::default().fg(Color::Cyan)),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Connection"));

    f.render_widget(paragraph, area);
}

pub fn draw_sync_status(
    f: &mut Frame,
    area: Rect,
    mode: &str,
    files_synced: u32,
    bytes_transferred: u64,
) {
    let bytes_str = if bytes_transferred > 1024 * 1024 {
        format!("{:.2} MB", bytes_transferred as f64 / (1024.0 * 1024.0))
    } else if bytes_transferred > 1024 {
        format!("{:.2} KB", bytes_transferred as f64 / 1024.0)
    } else {
        format!("{} B", bytes_transferred)
    };

    let text = vec![
        Line::from(vec![
            Span::raw("Mode: "),
            Span::styled(mode, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Files: "),
            Span::styled(files_synced.to_string(), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("Transferred: "),
            Span::styled(bytes_str, Style::default().fg(Color::Yellow)),
        ]),
    ];

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Sync Status"));

    f.render_widget(paragraph, area);
}

pub fn center_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect::new(
        x,
        y,
        width.min(area.width),
        height.min(area.height),
    )
}
