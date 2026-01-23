use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::bordered_block;
use crate::app::{App, Panel, PayloadMode};
use crate::mqtt::MqttMessage;

pub fn render_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == Panel::Messages;

    let title = match &app.selected_topic {
        Some(topic) => format!("Messages: {}", truncate_topic(topic, 30)),
        None => "Messages".to_string(),
    };

    let block = bordered_block(&title, focused);
    let inner = block.inner(area);

    frame.render_widget(block, area);

    // Split view: message list on top, payload detail below
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Percentage(40),
            ratatui::layout::Constraint::Percentage(60),
        ])
        .split(inner);

    // Update message scroll to keep selection visible (before borrowing messages)
    let message_count = app.get_current_messages().len();
    if message_count > 0 {
        let visible_height = chunks[0].height as usize;
        let selected = app
            .selected_message_index
            .min(message_count.saturating_sub(1));

        if selected < app.message_scroll {
            app.message_scroll = selected;
        } else if selected >= app.message_scroll + visible_height {
            app.message_scroll = selected.saturating_sub(visible_height.saturating_sub(1));
        }
    }

    let messages = app.get_current_messages();

    if messages.is_empty() {
        let empty_msg = if app.selected_topic.is_some() {
            "No messages for this topic"
        } else {
            "Select a topic to view messages"
        };

        let text = Paragraph::new(Span::styled(
            empty_msg,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ));
        frame.render_widget(text, inner);
        return;
    }

    // Message list
    render_message_list(frame, app, &messages, chunks[0]);

    // Payload detail
    if let Some(msg) = messages.get(app.selected_message_index) {
        render_payload_detail(frame, app, msg, chunks[1]);
    }
}

fn render_message_list(frame: &mut Frame, app: &App, messages: &[&MqttMessage], area: Rect) {
    let items: Vec<ListItem> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let is_selected = i == app.selected_message_index;
            create_message_item(msg, is_selected)
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected_message_index));
    *state.offset_mut() = app.message_scroll;

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, area, &mut state);
}

fn create_message_item(msg: &MqttMessage, _is_selected: bool) -> ListItem<'static> {
    let time = msg.timestamp.format("%H:%M:%S%.3f").to_string();

    // QoS color
    let qos_color = match msg.qos {
        0 => Color::Gray,
        1 => Color::Yellow,
        2 => Color::Green,
        _ => Color::White,
    };

    // Preview payload (first line, truncated)
    let preview = msg
        .payload_str()
        .map(|s| {
            let first_line = s.lines().next().unwrap_or("");
            if first_line.len() > 40 {
                format!("{}...", &first_line[..40])
            } else {
                first_line.to_string()
            }
        })
        .unwrap_or_else(|| format!("<{} bytes binary>", msg.payload_size()));

    let line = Line::from(vec![
        Span::styled(time, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format!("Q{}", msg.qos), Style::default().fg(qos_color)),
        Span::raw(" "),
        Span::styled(
            format!("{}B", msg.payload_size()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::raw(preview),
    ]);

    ListItem::new(line)
}

fn render_payload_detail(frame: &mut Frame, app: &App, msg: &MqttMessage, area: Rect) {
    let mode_indicator = match app.payload_mode {
        PayloadMode::Auto => "AUTO",
        PayloadMode::Raw => "RAW",
        PayloadMode::Hex => "HEX",
        PayloadMode::Json => "JSON",
    };

    let header = Line::from(vec![
        Span::styled("Payload ", Style::default().fg(Color::White)),
        Span::styled(
            format!("[{}]", mode_indicator),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{} bytes", msg.payload_size()),
            Style::default().fg(Color::DarkGray),
        ),
        if msg.retain {
            Span::styled(" RETAINED", Style::default().fg(Color::Yellow))
        } else {
            Span::raw("")
        },
    ]);

    let payload = app.format_payload(msg);

    // Color JSON syntax
    let styled_payload = if matches!(app.payload_mode, PayloadMode::Auto | PayloadMode::Json)
        && msg.payload_json_pretty().is_some()
    {
        syntax_highlight_json(&payload)
    } else if matches!(app.payload_mode, PayloadMode::Hex) {
        Text::styled(payload, Style::default().fg(Color::Gray))
    } else {
        Text::raw(payload)
    };

    let mut lines = vec![header];
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(Color::DarkGray),
    )));

    for line in styled_payload.lines {
        lines.push(line);
    }

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn syntax_highlight_json(json: &str) -> Text<'static> {
    let mut lines = Vec::new();

    for line in json.lines() {
        let mut spans = Vec::new();
        let mut chars = line.chars().peekable();
        let mut buffer = String::new();
        let mut in_string = false;
        let mut is_key = true;

        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    if !buffer.is_empty() {
                        spans.push(Span::raw(buffer.clone()));
                        buffer.clear();
                    }
                    buffer.push(c);
                    in_string = !in_string;

                    if !in_string {
                        // End of string
                        let color = if is_key { Color::Cyan } else { Color::Green };
                        spans.push(Span::styled(buffer.clone(), Style::default().fg(color)));
                        buffer.clear();

                        // Check if next non-whitespace is colon
                        is_key = false;
                    }
                }
                ':' if !in_string => {
                    if !buffer.is_empty() {
                        spans.push(Span::raw(buffer.clone()));
                        buffer.clear();
                    }
                    spans.push(Span::styled(":", Style::default().fg(Color::White)));
                    is_key = false;
                }
                ',' if !in_string => {
                    if !buffer.is_empty() {
                        spans.push(Span::raw(buffer.clone()));
                        buffer.clear();
                    }
                    spans.push(Span::styled(",", Style::default().fg(Color::White)));
                    is_key = true;
                }
                '{' | '}' | '[' | ']' if !in_string => {
                    if !buffer.is_empty() {
                        spans.push(Span::raw(buffer.clone()));
                        buffer.clear();
                    }
                    spans.push(Span::styled(
                        c.to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                    is_key = c == '{';
                }
                _ if in_string => {
                    buffer.push(c);
                }
                c if !in_string && (c.is_numeric() || c == '-' || c == '.') => {
                    buffer.push(c);
                    // Peek ahead to collect full number
                    while chars
                        .peek()
                        .map(|&c| {
                            c.is_numeric()
                                || c == '.'
                                || c == 'e'
                                || c == 'E'
                                || c == '-'
                                || c == '+'
                        })
                        .unwrap_or(false)
                    {
                        buffer.push(chars.next().unwrap());
                    }
                    spans.push(Span::styled(
                        buffer.clone(),
                        Style::default().fg(Color::Magenta),
                    ));
                    buffer.clear();
                }
                _ => {
                    buffer.push(c);
                }
            }
        }

        if !buffer.is_empty() {
            // Check for boolean/null
            let trimmed = buffer.trim();
            if trimmed == "true" || trimmed == "false" {
                spans.push(Span::styled(
                    buffer.clone(),
                    Style::default().fg(Color::Yellow),
                ));
            } else if trimmed == "null" {
                spans.push(Span::styled(
                    buffer.clone(),
                    Style::default().fg(Color::Red),
                ));
            } else {
                spans.push(Span::raw(buffer));
            }
        }

        lines.push(Line::from(spans));
    }

    Text::from(lines)
}

fn truncate_topic(topic: &str, max_len: usize) -> String {
    if topic.len() <= max_len {
        topic.to_string()
    } else {
        format!("...{}", &topic[topic.len() - max_len + 3..])
    }
}
