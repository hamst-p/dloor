use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{
    ActiveDownload, App, CompleteState, ErrorState, GenericConfirmState, HistoryState, MainState,
    PreviewLoadingState, PreviewState, QualityState, QueueState, Screen, SetupField, SetupState,
    SharedState, UpdateResultState,
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
        Screen::PreviewLoading(state) => {
            render_preview_loading(frame, state, app.shared.spinner_index)
        }
        Screen::Preview(state) => render_preview(frame, state),
        Screen::GenericConfirm(state) => render_generic_confirm(frame, state),
        Screen::Playlist(state) => render_choice(
            frame,
            "Download scope",
            &["Single item", "Entire playlist"],
            state.selected,
        ),
        Screen::Format(state) => {
            render_choice(frame, "Format", &["Video", "Audio"], state.selected)
        }
        Screen::Quality(state) => render_quality(frame, state),
        Screen::Download(state) => render_download(
            frame,
            app.shared.active_for(state.job_id),
            app.shared.queue.entry(state.job_id),
            app.shared.spinner_index,
        ),
        Screen::Queue(state) => render_queue(frame, state, &app.shared),
        Screen::History(state) => render_history(frame, state, &app.shared),
        Screen::Complete(state) => render_complete(frame, state),
        Screen::UpdateConfirm => render_update_confirm(frame),
        Screen::UpdateRunning => render_update_running(frame, app.shared.spinner_index),
        Screen::UpdateResult(state) => render_update_result(frame, state),
        Screen::ExitConfirm => render_exit_confirm(frame, &app.shared),
        Screen::Error(state) => render_error(frame, state),
    }
    render_queue_indicator(frame, &app.shared);
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
        "Cookies",
        match state.cookie_source_index {
            1 => "Browser",
            2 => "File",
            _ => "Do not use",
        },
        state.field == SetupField::CookieSource,
    ));
    if state.cookie_source_index == 1 {
        lines.push(field_line(
            "Browser",
            dloor_core::Browser::ALL[state.browser_index].label(),
            state.field == SetupField::Browser,
        ));
    } else if state.cookie_source_index == 2 {
        lines.push(field_line(
            "Cookie file",
            &state.cookie_file_path,
            state.field == SetupField::CookieFile,
        ));
    }
    lines.push(Line::from(Span::styled(
        "Use cookies only for content you own or are authorized to access.",
        Style::new().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "dloor passes the selection to yt-dlp and never logs cookie paths or contents.",
        Style::new().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(field_line(
        "Confirm generic URLs",
        if state.confirm_generic_urls {
            "Always"
        } else {
            "Do not ask"
        },
        state.field == SetupField::GenericConfirmation,
    ));
    lines.push(field_line(
        "Bandwidth limit",
        if state.bandwidth_limit.is_empty() {
            "Unlimited"
        } else {
            &state.bandwidth_limit
        },
        state.field == SetupField::BandwidthLimit,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from("Optional media").bold());
    lines.push(field_line(
        "Write subtitle files",
        on_off(state.write_subtitles),
        state.field == SetupField::WriteSubtitles,
    ));
    lines.push(field_line(
        "Embed subtitles",
        on_off(state.embed_subtitles),
        state.field == SetupField::EmbedSubtitles,
    ));
    lines.push(field_line(
        "Subtitle languages",
        if state.subtitle_languages.is_empty() {
            "all requested languages"
        } else {
            &state.subtitle_languages
        },
        state.field == SetupField::SubtitleLanguages,
    ));
    lines.push(field_line(
        "Include auto subtitles",
        on_off(state.include_auto_subtitles),
        state.field == SetupField::AutoSubtitles,
    ));
    lines.push(field_line(
        "Embed thumbnail",
        on_off(state.embed_thumbnail),
        state.field == SetupField::EmbedThumbnail,
    ));
    lines.push(field_line(
        "Embed chapters",
        on_off(state.embed_chapters),
        state.field == SetupField::EmbedChapters,
    ));
    lines.push(Line::from(Span::styled(
        "Optional embedding failures keep the downloaded media and appear as warnings.",
        Style::new().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .scroll((state.scroll_offset as u16, 0)),
        centered(chunks[1], 90, 22),
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

fn on_off(value: bool) -> &'static str {
    if value {
        "On"
    } else {
        "Off"
    }
}

fn render_main(frame: &mut Frame<'_>, state: &MainState, shared: &SharedState) {
    let area = centered(frame.area(), 96, 23);
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
            Constraint::Length(2),
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
        Paragraph::new(shared.dependency_warning().unwrap_or(""))
            .alignment(Alignment::Center)
            .fg(Color::Yellow)
            .wrap(Wrap { trim: false }),
        body[7],
    );
    frame.render_widget(
        Paragraph::new(state.url_input.as_str())
            .block(Block::default().title("Input URL").borders(Borders::ALL)),
        body[8],
    );
    frame.render_widget(
        Paragraph::new("/queue  /history  /update  /howtouse  /settings  /quit")
            .alignment(Alignment::Center)
            .fg(Color::DarkGray),
        body[9],
    );
    render_footer(frame, footer_area(frame.area()), "Paste URL, then Enter");
}

fn render_update_confirm(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 72, 9);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Run yt-dlp -U now?").centered().yellow().bold(),
            Line::from(""),
            Line::from("This modifies the installed yt-dlp executable when self-update"),
            Line::from("is supported. Package-managed installs may require another command."),
            Line::from(""),
            Line::from("Enter/y: update   Esc/n: cancel")
                .centered()
                .dark_gray(),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title("Update yt-dlp")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_update_running(frame: &mut Frame<'_>, spinner_index: usize) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let spinner = ["|", "/", "-", "\\"][spinner_index % 4];
    frame.render_widget(
        Paragraph::new(format!("{spinner} Running yt-dlp -U..."))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Updating yt-dlp")
                    .borders(Borders::ALL),
            ),
        centered(chunks[1], 72, 7),
    );
}

fn render_update_result(frame: &mut Frame<'_>, state: &UpdateResultState) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let mut lines = vec![
        Line::from(if state.outcome.success {
            "yt-dlp update completed"
        } else {
            "yt-dlp could not self-update"
        })
        .centered()
        .style(if state.outcome.success {
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
        }),
        Line::from(""),
        Line::from(state.outcome.output.clone()),
    ];
    if let Some(hint) = &state.outcome.update_hint {
        lines.extend([
            Line::from(""),
            Line::from("Suggested update method:").yellow(),
            Line::from(hint.clone()),
        ]);
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Update result")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        centered(chunks[1], 92, 15),
    );
    render_footer(frame, chunks[2], "Enter/Esc: return to main  q: quit");
}

fn render_how_to_use(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 58, 7);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("how to use:").centered(),
            Line::from("1. input URL (URL auto detect)").centered(),
            Line::from("2. review metadata and choose scope").centered(),
            Line::from("3. select format & quality to enqueue").centered(),
            Line::from(""),
            Line::from("Enter to exit").centered().fg(Color::DarkGray),
        ])
        .alignment(Alignment::Center),
        area,
    );
}

fn render_preview_loading(
    frame: &mut Frame<'_>,
    state: &PreviewLoadingState,
    spinner_index: usize,
) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let spinner = ["|", "/", "-", "\\"][spinner_index % 4];
    let message = if state.cancelling {
        format!("{spinner} Cancelling metadata preview...")
    } else {
        format!("{spinner} Loading metadata preview...")
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .block(Block::default().title("Preview").borders(Borders::ALL)),
        centered(chunks[1], 72, 7),
    );
    render_footer(frame, chunks[2], "Esc: cancel preview");
}

fn render_preview(frame: &mut Frame<'_>, state: &PreviewState) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let preview = &state.preview;
    let mut lines = vec![
        field_line("Title", &preview.title, false),
        field_line(
            "Uploader",
            preview.uploader.as_deref().unwrap_or("Unknown"),
            false,
        ),
        field_line(
            "Duration",
            &preview
                .duration_seconds
                .map_or_else(|| "Unknown".to_string(), format_duration),
            false,
        ),
        field_line(
            "Resolutions",
            &if preview.resolutions.is_empty() {
                "Not reported".to_string()
            } else {
                preview.resolutions.join(", ")
            },
            false,
        ),
    ];
    if let Some(playlist) = &preview.playlist {
        lines.push(Line::from(""));
        lines.push(field_line(
            "Playlist items",
            &playlist
                .total_items
                .map_or_else(|| "At least 5".to_string(), |total| total.to_string()),
            false,
        ));
        for (index, title) in playlist.sample_titles.iter().enumerate() {
            lines.push(Line::from(format!(
                "  {}. {}",
                index + 1,
                truncate_text(title, 68)
            )));
        }
        if playlist.truncated {
            lines.push(Line::from("  …more items are not fetched for preview").dark_gray());
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Metadata preview")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        centered(chunks[1], 88, 17),
    );
    render_footer(frame, chunks[2], "Enter: continue  Esc: back");
}

fn render_generic_confirm(frame: &mut Frame<'_>, state: &GenericConfirmState) {
    let area = centered(frame.area(), 78, 11);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("This host is not in dloor's tested platform list.").yellow(),
            Line::from(""),
            Line::from("The URL is syntactically valid and can be passed to yt-dlp,"),
            Line::from("but compatibility and output are not guaranteed."),
            Line::from(""),
            Line::from(truncate_text(&state.url, 68)).dark_gray(),
            Line::from(""),
            Line::from("Enter: continue once   a: continue and do not ask again   Esc: cancel")
                .dark_gray(),
        ])
        .block(
            Block::default()
                .title("Unverified host")
                .borders(Borders::ALL),
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
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

fn render_quality(frame: &mut Frame<'_>, state: &QualityState) {
    const VISIBLE_OPTIONS: usize = 6;
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let items = state
        .options
        .iter()
        .enumerate()
        .skip(state.scroll_offset)
        .take(VISIBLE_OPTIONS)
        .map(|(index, quality)| {
            let description = match quality {
                dloor_core::Quality::Best => "possible highest quality".to_string(),
                dloor_core::Quality::Compressed => "share-friendly size".to_string(),
                _ => format!("highest available stream at or below {}", quality.label()),
            };
            let prefix = if index == state.selected { "> " } else { "  " };
            let style = if index == state.selected {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            ListItem::new(format!("{prefix}{} - {description}", quality.label())).style(style)
        })
        .collect::<Vec<_>>();
    let mut title = "Quality".to_string();
    if state.options.len() > VISIBLE_OPTIONS {
        title.push_str(&format!(
            " ({}-{}/{})",
            state.scroll_offset + 1,
            (state.scroll_offset + VISIBLE_OPTIONS).min(state.options.len()),
            state.options.len()
        ));
    }
    let area = centered(chunks[1], 80, 12);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Length(2)])
        .split(area);
    frame.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        body[0],
    );
    if let Some(note) = &state.note {
        frame.render_widget(
            Paragraph::new(note.as_str())
                .alignment(Alignment::Center)
                .fg(Color::Yellow),
            body[1],
        );
    }
    render_footer(
        frame,
        chunks[2],
        "Arrow keys: select  Enter: enqueue  Esc: back",
    );
}

fn render_download(
    frame: &mut Frame<'_>,
    active: Option<&ActiveDownload>,
    queued: Option<&dloor_core::QueuedJob>,
    spinner_index: usize,
) {
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
        .map(|download| download.platform)
        .map(|platform| platform.label())
        .unwrap_or("Detecting...");
    let status_text = active.map_or_else(
        || {
            queued.map_or("No longer in queue", |job| match job.status {
                dloor_core::QueueStatus::Pending => "Waiting in queue",
                dloor_core::QueueStatus::Running => "Starting...",
                _ => job.status.label(),
            })
        },
        |download| download.status_text.as_str(),
    );
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
    for warning in state.summary.warnings.iter().take(3) {
        lines.push(
            Line::from(format!(
                "⚠ {}: {}",
                truncate_text(&warning.item.title, 24),
                truncate_text(&warning.message, 54)
            ))
            .yellow(),
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

fn render_queue(frame: &mut Frame<'_>, state: &QueueState, shared: &SharedState) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let jobs: Vec<_> = shared
        .queue
        .entries()
        .filter(|job| {
            matches!(
                job.status,
                dloor_core::QueueStatus::Pending | dloor_core::QueueStatus::Running
            )
        })
        .collect();
    let items: Vec<_> = jobs
        .iter()
        .enumerate()
        .map(|(index, job)| {
            let marker = if index == state.selected { "> " } else { "  " };
            let progress = job.progress.as_ref().map_or_else(String::new, |progress| {
                format!(" {:>5.1}%", progress.overall_percent)
            });
            let style = if index == state.selected {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            ListItem::new(format!(
                "{marker}#{:<3} {:<8}{progress} {}",
                job.id.0,
                job.status.label(),
                truncate_text(&job.title, 52)
            ))
            .style(style)
        })
        .collect();
    let list = if items.is_empty() {
        List::new(vec![ListItem::new("The queue is empty")])
    } else {
        List::new(items)
    };
    frame.render_widget(
        list.block(
            Block::default()
                .title("Download queue")
                .borders(Borders::ALL),
        ),
        centered(chunks[1], 92, 16),
    );
    render_footer(
        frame,
        chunks[2],
        "↑/↓ select  Ctrl+↑/↓ reorder  Enter monitor  c cancel  d remove  Esc back",
    );
}

fn render_history(frame: &mut Frame<'_>, state: &HistoryState, shared: &SharedState) {
    let chunks = base_layout(frame.area());
    render_logo(frame, chunks[0]);
    let entries: Vec<_> = shared.history.entries().iter().rev().collect();
    let items: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let marker = if index == state.selected { "> " } else { "  " };
            let style = if index == state.selected {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            let path = entry.destination_path.as_deref().unwrap_or("-");
            ListItem::new(format!(
                "{marker}{:<10} {:<16} {} | {}",
                entry.status.label(),
                truncate_text(&entry.recorded_at, 16),
                truncate_text(&entry.title, 36),
                truncate_text(path, 42)
            ))
            .style(style)
        })
        .collect();
    let list = if items.is_empty() {
        List::new(vec![ListItem::new("No download history yet")])
    } else {
        List::new(items)
    };
    frame.render_widget(
        list.block(Block::default().title("History").borders(Borders::ALL)),
        centered(chunks[1], 110, 18),
    );
    render_footer(
        frame,
        chunks[2],
        "↑/↓ select  r retry failed item  Esc back",
    );
}

fn render_exit_confirm(frame: &mut Frame<'_>, shared: &SharedState) {
    let area = centered(frame.area(), 68, 8);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!(
            "{} job is running and {} are waiting.\n\nCancel unfinished jobs and quit? (y/n)",
            usize::from(shared.active_download.is_some()),
            shared.queue.pending_count()
        ))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title("Unfinished downloads")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false }),
        area,
    );
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

fn render_queue_indicator(frame: &mut Frame<'_>, shared: &SharedState) {
    let width = 58u16.min(frame.area().width);
    let area = Rect {
        x: frame.area().right().saturating_sub(width),
        y: frame.area().y,
        width,
        height: 1.min(frame.area().height),
    };
    let running = usize::from(shared.active_download.is_some());
    let mut text = format!(
        "Running {running} | Queued {}",
        shared.queue.pending_count()
    );
    if let Some(notification) = &shared.notification {
        text.push_str(" | ");
        text.push_str(&truncate_text(notification, 32));
    }
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Right)
            .fg(Color::DarkGray),
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
    format!("auth: {}", shared.config.cookies.label())
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
