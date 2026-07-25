use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{
    ActiveDownload, App, CompleteState, ErrorState, MainState, Screen, SetupField, SetupState,
    SharedState,
};

const LOGO: &str = r"
██████╗░██╗░░░░░░█████╗░░█████╗░██████╗░
██╔══██╗██║░░░░░██╔══██╗██╔══██╗██╔══██╗
██║░░██║██║░░░░░██║░░██║██║░░██║██████╔╝
██║░░██║██║░░░░░██║░░██║██║░░██║██╔══██╗
██████╔╝███████╗╚█████╔╝╚█████╔╝██║░░██║
╚═════╝░╚══════╝░╚════╝░░╚════╝░╚═╝░░╚═╝
";

pub fn render(frame: &mut Frame<'_>, app: &App) {
    match &app.navigation.current {
        Screen::Setup(state) => render_setup(
            frame,
            state,
            app.shared.first_run,
            app.shared.rclone_available,
        ),
        Screen::Main(state) => render_main(frame, state, &app.shared),
        Screen::HowToUse => render_how_to_use(frame),
        Screen::Playlist(state) => render_choice(
            frame,
            "Download scope",
            &["Single item", "Entire playlist"],
            state.selected,
        ),
        Screen::Format(state) => {
            render_choice(frame, "Format", &["Video", "Audio"], state.selected)
        }
        Screen::Quality(state) => render_choice(
            frame,
            "Quality",
            &[
                "Best - possible highest quality",
                "Compressed - share-friendly size",
            ],
            state.selected,
        ),
        Screen::Download(_) => render_download(
            frame,
            app.shared.active_download.as_ref(),
            app.shared.spinner_index,
        ),
        Screen::Complete(state) => render_complete(frame, state),
        Screen::Error(state) => render_error(frame, state),
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

fn render_setup(
    frame: &mut Frame<'_>,
    state: &SetupState,
    first_run: bool,
    rclone_available: bool,
) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);

    let title = if first_run { "Setup" } else { "Settings" };
    let destination = if state.cloud { "Cloud" } else { "Local" };
    let cloud_hint = if rclone_available {
        "Left/Right toggles Local/Cloud"
    } else {
        "rclone not found; cloud upload is disabled until rclone is installed"
    };

    let mut lines = vec![
        field_line(
            "Destination",
            destination,
            state.field == SetupField::Destination,
        ),
        Line::from(Span::styled(cloud_hint, Style::new().fg(Color::DarkGray))),
        Line::from(""),
    ];

    if state.cloud {
        lines.push(field_line(
            "Remote",
            &state.remote,
            state.field == SetupField::Remote,
        ));
        lines.push(field_line(
            "Remote path",
            &state.remote_path,
            state.field == SetupField::RemotePath,
        ));
    } else {
        lines.push(field_line(
            "Local path",
            &state.local_path,
            state.field == SetupField::LocalPath,
        ));
    }

    lines.push(Line::from(""));
    lines.push(field_line(
        "Browser authentication",
        if state.use_browser_cookies {
            "On"
        } else {
            "Off"
        },
        state.field == SetupField::BrowserAuthentication,
    ));
    if state.use_browser_cookies {
        lines.push(field_line(
            "Browser",
            dloor_core::Browser::ALL[state.browser_index].label(),
            state.field == SetupField::Browser,
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

fn render_main(frame: &mut Frame<'_>, state: &MainState, shared: &SharedState) {
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
        Paragraph::new(destination_label(shared))
            .alignment(Alignment::Center)
            .fg(Color::Yellow),
        body[5],
    );
    frame.render_widget(
        Paragraph::new(authentication_label(shared))
            .alignment(Alignment::Center)
            .fg(Color::DarkGray),
        body[6],
    );
    frame.render_widget(
        Paragraph::new(state.url_input.as_str())
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

fn render_download(frame: &mut Frame<'_>, active: Option<&ActiveDownload>, spinner_index: usize) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let area = centered(chunks[1], 84, 17);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    let platform = active
        .and_then(|download| download.platform)
        .map(|platform| platform.label())
        .unwrap_or("Detecting...");
    let status_text = active.map_or("Finishing...", |download| download.status_text.as_str());
    let item_label = active
        .and_then(|download| download.item.as_ref())
        .map_or_else(
            || "Resolving download items".to_string(),
            |item| {
                format!(
                    "{}/{} | {}",
                    item.index,
                    item.total,
                    truncate_text(&item.title, 56)
                )
            },
        );
    frame.render_widget(
        Paragraph::new(format!("{platform} | {status_text}\n{item_label}"))
            .block(Block::default().title("Download").borders(Borders::ALL)),
        body[0],
    );

    let item_percent = active
        .and_then(|download| download.progress.as_ref())
        .map_or(0.0, |progress| progress.item_percent);
    frame.render_widget(
        Gauge::default()
            .block(Block::default().title("Current item").borders(Borders::ALL))
            .gauge_style(Style::new().fg(Color::Green))
            .percent(item_percent.round() as u16),
        body[1],
    );

    let overall_percent = active
        .and_then(|download| download.progress.as_ref())
        .map_or(0.0, |progress| progress.overall_percent);
    frame.render_widget(
        Gauge::default()
            .block(Block::default().title("Overall").borders(Borders::ALL))
            .gauge_style(Style::new().fg(Color::Cyan))
            .percent(overall_percent.round() as u16),
        body[2],
    );

    let spinner = ["|", "/", "-", "\\"][spinner_index % 4];
    let detail = active
        .and_then(|download| download.progress.as_ref())
        .map_or_else(
            || format!("{spinner} {status_text}"),
            |progress| {
                format!(
                    "Item {:.1}% | Overall {:.1}% | Speed: {} | ETA: {}",
                    progress.item_percent, progress.overall_percent, progress.speed, progress.eta
                )
            },
        );
    frame.render_widget(Paragraph::new(detail).alignment(Alignment::Center), body[3]);
    render_footer(frame, chunks[2], "Esc: cancel download");
}

fn render_complete(frame: &mut Frame<'_>, state: &CompleteState) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let mut lines = vec![
        Line::from("Completed").centered().green().bold(),
        Line::from(""),
        Line::from(format!(
            "Succeeded: {}  Failed: {}  Total: {}",
            state.summary.succeeded.len(),
            state.summary.failed.len(),
            state.summary.total
        ))
        .centered(),
    ];
    for success in state.summary.succeeded.iter().take(3) {
        lines.push(Line::from(format!(
            "✓ {} → {}",
            truncate_text(&success.item.title, 28),
            truncate_text(&success.path, 42)
        )));
    }
    for failure in state.summary.failed.iter().take(5) {
        lines.push(
            Line::from(format!(
                "✗ {}: {}",
                truncate_text(&failure.item.title, 30),
                truncate_text(&failure.error, 44)
            ))
            .red(),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title("Done").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        centered(chunks[1], 88, 14),
    );
    render_footer(frame, chunks[2], "Enter: new download  q: quit");
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn render_error(frame: &mut Frame<'_>, state: &ErrorState) {
    let area = centered(frame.area(), 76, 10);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(state.message.as_str())
            .block(Block::default().title("Error").borders(Borders::ALL))
            .fg(Color::Red)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn destination_label(shared: &SharedState) -> String {
    match &shared.config.destination {
        dloor_core::Destination::Local { path } => {
            format!("local: {}", path.to_string_lossy())
        }
        dloor_core::Destination::Cloud { remote, .. } => {
            format!("cloud: Google Drive ({remote})")
        }
    }
}

fn authentication_label(shared: &SharedState) -> String {
    shared.config.browser.map_or_else(
        || "auth: public content only".to_string(),
        |browser| format!("auth: {} browser session", browser.label()),
    )
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
