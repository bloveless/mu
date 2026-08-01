use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::app::{AppState, ToolStatus};

pub fn render_messages(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut lines: Vec<Line> = Vec::new();

    // Render committed messages
    for msg in &state.messages {
        let (label, color) = match msg.role.as_str() {
            "user" => ("You", Color::Blue),
            "assistant" => ("Assistant", Color::Green),
            _ => ("System", Color::Gray),
        };

        lines.push(Line::from(vec![Span::styled(
            format!("› {label}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )]));

        for content_line in msg.content.lines() {
            lines.push(Line::from(format!("  {content_line}")));
        }

        lines.push(Line::from("")); // spacing
    }

    // Render streaming text
    if !state.streaming_text.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "› Assistant",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]));

        for content_line in state.streaming_text.lines() {
            lines.push(Line::from(format!("  {content_line}")));
        }
    }

    // Render active tool
    if let Some(ref tool) = state.active_tool {
        let status_text = match &tool.status {
            ToolStatus::Running => "...".to_string(),
            ToolStatus::Complete(result) => {
                let preview = &result[..result.len().min(80)];
                format!("✓ {preview}")
            }
        };

        lines.push(Line::from(vec![
            Span::styled("  ⚡ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                &tool.name,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {status_text}")),
        ]));
    }

    // Render approval request
    if let Some(ref approval) = state.pending_approval {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  ⚠ Approval Required: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&approval.tool_name, Style::default().fg(Color::Cyan)),
        ]));
        lines.push(Line::from(format!("    {}", approval.args_preview)));
        lines.push(Line::from(vec![Span::styled(
            "    [Y]es / [N]o",
            Style::default().fg(Color::Yellow),
        )]));
    }

    // Loading indicator
    if state.loading && state.streaming_text.is_empty() && state.active_tool.is_none() {
        lines.push(Line::from(vec![Span::styled(
            "  Thinking...",
            Style::default().fg(Color::Gray),
        )]))
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title(" Chat "))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_offset, 0));

    frame.render_widget(paragraph, area);
}
