use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Screen, SetupField};

const LOGO: &str = r"
██████╗░██╗░░░░░░█████╗░░█████╗░██████╗░
██╔══██╗██║░░░░░██╔══██╗██╔══██╗██╔══██╗
██║░░██║██║░░░░░██║░░██║██║░░██║██████╔╝
██║░░██║██║░░░░░██║░░██║██║░░██║██╔══██╗
██████╔╝███████╗╚█████╔╝╚█████╔╝██║░░██║
╚═════╝░╚══════╝░╚════╝░░╚════╝░╚═╝░░╚═╝
";

pub fn render(frame: &mut Frame<'_>, app: &App) {
    match app.screen {
        Screen::Setup => render_setup(frame, app),
        Screen::Main => render_main(frame, app),
        Screen::HowToUse => render_how_to_use(frame),
        Screen::Format => render_choice(frame, "Format", &["Video", "Audio"], app.selected_format),
        Screen::Quality => render_choice(
            frame,
            "Quality",
            &[
                "Best - possible highest quality",
                "Compressed - share-friendly size",
            ],
            app.selected_quality,
        ),
        Screen::Download => render_download(frame, app),
        Screen::Complete => render_complete(frame, app),
        Screen::Error => render_error(frame, app),
    }
}

fn base_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area)
        .to_vec()
}

fn render_logo(frame: &mut Frame<'_>, area: Rect) {
    let logo = Paragraph::new(LOGO)
        .fg(Color::Cyan)
        .alignment(Alignment::Center);
    frame.render_widget(logo, area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, text: &str) {
    frame.render_widget(
        Paragraph::new(text)
            .fg(Color::DarkGray)
            .alignment(Alignment::Center),
        area,
    );
}

fn render_setup(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);

    let title = if app.first_run { "Setup" } else { "Settings" };
    let destination = if app.setup.cloud { "Cloud" } else { "Local" };
    let cloud_hint = if app.setup.rclone_available {
        "Left/Right toggles Local/Cloud"
    } else {
        "rclone not found; cloud upload is disabled until rclone is installed"
    };

    let mut lines = vec![
        field_line(
            "Destination",
            destination,
            app.setup.field == SetupField::Destination,
        ),
        Line::from(Span::styled(cloud_hint, Style::new().fg(Color::DarkGray))),
        Line::from(""),
    ];

    if app.setup.cloud {
        lines.push(field_line(
            "Remote",
            &app.setup.remote,
            app.setup.field == SetupField::Remote,
        ));
        lines.push(field_line(
            "Remote path",
            &app.setup.remote_path,
            app.setup.field == SetupField::RemotePath,
        ));
    } else {
        lines.push(field_line(
            "Local path",
            &app.setup.local_path,
            app.setup.field == SetupField::LocalPath,
        ));
    }

    lines.push(Line::from(""));
    lines.push(field_line(
        "Browser authentication",
        if app.setup.use_browser_cookies {
            "On"
        } else {
            "Off"
        },
        app.setup.field == SetupField::BrowserAuthentication,
    ));
    if app.setup.use_browser_cookies {
        lines.push(field_line(
            "Browser",
            dloor_core::Browser::ALL[app.setup.browser_index].label(),
            app.setup.field == SetupField::Browser,
        ));
        lines.push(Line::from(Span::styled(
            "Uses the selected browser's logged-in session; cookies are not copied by dloor",
            Style::new().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        centered(chunks[1], 82, 16),
    );
    render_footer(frame, chunks[2], "Tab: next field  Enter: save  Esc: back");
}

fn field_line(label: &str, value: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    Line::from(vec![
        Span::styled(format!("{label}: "), style),
        Span::styled(value.to_string(), style),
    ])
}

fn render_main(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 86, 21);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(LOGO)
            .fg(Color::Cyan)
            .alignment(Alignment::Center),
        body[0],
    );
    frame.render_widget(
        Paragraph::new("DLOOR")
            .alignment(Alignment::Center)
            .fg(Color::White)
            .bold(),
        body[2],
    );
    frame.render_widget(
        Paragraph::new("yt-dlp & ffmpeg powered multimedia downloadoor")
            .alignment(Alignment::Center)
            .fg(Color::Gray),
        body[3],
    );
    frame.render_widget(
        Paragraph::new(app.destination_label())
            .alignment(Alignment::Center)
            .fg(Color::Yellow),
        body[5],
    );
    frame.render_widget(
        Paragraph::new(app.authentication_label())
            .alignment(Alignment::Center)
            .fg(Color::DarkGray),
        body[6],
    );
    frame.render_widget(
        Paragraph::new(app.url_input.as_str())
            .block(Block::default().title("Input URL").borders(Borders::ALL)),
        body[7],
    );
    frame.render_widget(
        Paragraph::new("/howtouse  /settings  /quit")
            .alignment(Alignment::Center)
            .fg(Color::DarkGray),
        body[8],
    );
    render_footer(frame, footer_area(frame.area()), "Paste URL, then Enter");
}

fn render_how_to_use(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 58, 7);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("how to use:").centered(),
            Line::from("1. input URL (URL auto detect)").centered(),
            Line::from("2. select format & quality").centered(),
            Line::from("3. done, enjoy!").centered(),
            Line::from(""),
            Line::from("Enter to exit").centered().fg(Color::DarkGray),
        ])
        .alignment(Alignment::Center),
        area,
    );
}

fn render_choice(frame: &mut Frame<'_>, title: &str, options: &[&str], selected: usize) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let items: Vec<ListItem<'_>> = options
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let prefix = if index == selected { "> " } else { "  " };
            let style = if index == selected {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            ListItem::new(format!("{prefix}{item}")).style(style)
        })
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        centered(chunks[1], 74, 8),
    );
    render_footer(
        frame,
        chunks[2],
        "Arrow keys: select  Enter: continue  Esc: back",
    );
}

fn render_download(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let area = centered(chunks[1], 84, 12);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    let platform = app
        .selected_platform
        .map(|platform| platform.label())
        .unwrap_or("Detecting...");
    frame.render_widget(
        Paragraph::new(format!("{platform} | {}", app.status_text))
            .block(Block::default().title("Download").borders(Borders::ALL)),
        body[0],
    );

    let percent = app
        .progress
        .as_ref()
        .map_or(0.0, |progress| progress.percent);
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL))
            .gauge_style(Style::new().fg(Color::Green))
            .percent(percent.round() as u16),
        body[1],
    );

    let spinner = ["|", "/", "-", "\\"][app.spinner_index % 4];
    let detail = app.progress.as_ref().map_or_else(
        || format!("{spinner} {}", app.status_text),
        |progress| {
            format!(
                "{:.1}%  Speed: {}  ETA: {}",
                progress.percent, progress.speed, progress.eta
            )
        },
    );
    frame.render_widget(Paragraph::new(detail).alignment(Alignment::Center), body[2]);
    render_footer(frame, chunks[2], "Download runs asynchronously");
}

fn render_complete(frame: &mut Frame<'_>, app: &App) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Completed").centered().green().bold(),
            Line::from(""),
            Line::from(app.completed_path.clone()).centered(),
        ])
        .block(Block::default().title("Done").borders(Borders::ALL))
        .wrap(Wrap { trim: false }),
        centered(chunks[1], 84, 9),
    );
    render_footer(frame, chunks[2], "Enter: new download  q: quit");
}

fn render_error(frame: &mut Frame<'_>, app: &App) {
    let area = centered(frame.area(), 76, 10);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(app.error_message.as_str())
            .block(Block::default().title("Error").borders(Borders::ALL))
            .fg(Color::Red)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height.min(area.height)),
            Constraint::Min(0),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(width.min(area.width)),
            Constraint::Min(0),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn footer_area(area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(area)[1]
}
