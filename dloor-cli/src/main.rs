use std::{
    io::{self, IsTerminal, Write},
    process::ExitCode,
};

use clap::Parser;
use dloor_cli::{
    load_config, progress_enabled, resolve_options, validate_urls, JsonReport, ResolvedOptions,
    UrlResult, ValidatedUrl, ValidationError,
};
use dloor_core::{check_dependency_presence, DownloadEvent, DownloadJob, DownloadRequest};

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_INTERRUPTED: u8 = 130;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match dloor_cli::Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = u8::try_from(error.exit_code()).unwrap_or(EXIT_USAGE);
            let _ = error.print();
            return ExitCode::from(code);
        }
    };

    let validated_urls = match validate_urls(&cli.urls, cli.allow_generic) {
        Ok(urls) => urls,
        Err(error) => return validation_failure(&cli, error),
    };
    let config = match load_config(&cli) {
        Ok(config) => config,
        Err(error) => {
            return fatal_failure(
                cli.json,
                format!("could not load configuration: {error}"),
                EXIT_FAILURE,
            )
        }
    };
    let options = match resolve_options(&cli, config) {
        Ok(options) => options,
        Err(error) => return validation_failure(&cli, error),
    };

    let dependencies = check_dependency_presence(Some(&options.config));
    if !dependencies.is_ready() {
        return fatal_failure(cli.json, dependencies.message(), EXIT_FAILURE);
    }

    let show_progress = progress_enabled(
        io::stdout().is_terminal(),
        io::stderr().is_terminal(),
        cli.json,
    );
    let mut results = Vec::with_capacity(validated_urls.len());
    let mut interrupted = false;

    for (index, url) in validated_urls.iter().enumerate() {
        let outcome = run_url(
            url,
            index + 1,
            validated_urls.len(),
            &options,
            show_progress,
        )
        .await;
        interrupted |= outcome.interrupted;
        results.push(outcome.result);
        if interrupted {
            break;
        }
    }

    let report = JsonReport::from_results(results);
    if let Err(error) = emit_report(&report, cli.json) {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return ExitCode::SUCCESS;
        }
        eprintln!("dloor-cli: could not write output: {error}");
        return ExitCode::from(EXIT_FAILURE);
    }

    if interrupted {
        ExitCode::from(EXIT_INTERRUPTED)
    } else if report.success {
        ExitCode::from(EXIT_SUCCESS)
    } else {
        ExitCode::from(EXIT_FAILURE)
    }
}

struct RunOutcome {
    result: UrlResult,
    interrupted: bool,
}

async fn run_url(
    url: &ValidatedUrl,
    url_index: usize,
    url_total: usize,
    options: &ResolvedOptions,
    show_progress: bool,
) -> RunOutcome {
    let request = DownloadRequest {
        url: url.value.clone(),
        format: options.format,
        quality: options.quality,
        playlist: options.playlist,
    };
    let job = DownloadJob::new(request, options.config.clone());
    let cancellation = job.cancellation_token();
    let mut receiver = job.spawn();
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut watch_interrupt = true;
    let mut interrupted = false;

    loop {
        let event = if watch_interrupt {
            tokio::select! {
                event = receiver.recv() => event,
                signal = &mut interrupt => {
                    watch_interrupt = false;
                    match signal {
                        Ok(()) => {
                            interrupted = true;
                            cancellation.cancel();
                            if show_progress {
                                clear_progress_line();
                            }
                            eprintln!("dloor-cli: cancellation requested; waiting for the active process to stop");
                        }
                        Err(error) => {
                            eprintln!("dloor-cli: Ctrl-C monitoring is unavailable: {error}");
                        }
                    }
                    continue;
                }
            }
        } else {
            receiver.recv().await
        };

        let Some(event) = event else {
            if show_progress {
                clear_progress_line();
            }
            let error = "download event stream closed before a terminal result".to_string();
            eprintln!("dloor-cli: URL {url_index} failed: {error}");
            return RunOutcome {
                result: UrlResult::failed(url.value.clone(), error),
                interrupted,
            };
        };

        match event {
            DownloadEvent::Resolving => {
                render_stage(show_progress, url_index, url_total, "resolving items");
            }
            DownloadEvent::ItemStarted { item, .. } => {
                render_stage(
                    show_progress,
                    url_index,
                    url_total,
                    &format!("starting item {}/{}", item.index, item.total),
                );
            }
            DownloadEvent::Progress { progress, item, .. } => {
                if show_progress {
                    let bar = text_progress_bar(progress.overall_percent, 24);
                    eprint!(
                        "\r\u{1b}[2K[{bar}] {:5.1}% | URL {url_index}/{url_total} | item {}/{} {:5.1}% | {} | ETA {}",
                        progress.overall_percent,
                        item.index,
                        item.total,
                        progress.item_percent,
                        display_or_dash(&progress.speed),
                        display_or_dash(&progress.eta),
                    );
                    let _ = io::stderr().flush();
                }
            }
            DownloadEvent::Converting { item } => {
                render_stage(
                    show_progress,
                    url_index,
                    url_total,
                    &format!("converting item {}/{}", item.index, item.total),
                );
            }
            DownloadEvent::Uploading { item } => {
                render_stage(
                    show_progress,
                    url_index,
                    url_total,
                    &format!("uploading item {}/{}", item.index, item.total),
                );
            }
            DownloadEvent::ItemCompleted { .. } => {}
            DownloadEvent::ItemFailed { failure } => {
                if show_progress {
                    clear_progress_line();
                }
                eprintln!(
                    "dloor-cli: URL {url_index}, item {}/{} failed: {}",
                    failure.item.index, failure.item.total, failure.error
                );
            }
            DownloadEvent::ItemWarning { warning } => {
                if show_progress {
                    clear_progress_line();
                }
                eprintln!(
                    "dloor-cli: URL {url_index}, item {}/{} warning: {}",
                    warning.item.index, warning.item.total, warning.message
                );
            }
            DownloadEvent::Finished { summary } => {
                if show_progress {
                    clear_progress_line();
                }
                return RunOutcome {
                    result: UrlResult::from_summary(url.value.clone(), summary, false),
                    interrupted,
                };
            }
            DownloadEvent::Failed { error } => {
                if show_progress {
                    clear_progress_line();
                }
                eprintln!("dloor-cli: URL {url_index} failed: {error}");
                return RunOutcome {
                    result: UrlResult::failed(url.value.clone(), error),
                    interrupted,
                };
            }
            DownloadEvent::Cancelled { summary } => {
                if show_progress {
                    clear_progress_line();
                }
                return RunOutcome {
                    result: UrlResult::from_summary(url.value.clone(), summary, true),
                    interrupted,
                };
            }
            DownloadEvent::DependenciesChecked { .. }
            | DownloadEvent::YtDlpUpdateFinished { .. }
            | DownloadEvent::PreviewReady { .. }
            | DownloadEvent::PreviewFailed { .. }
            | DownloadEvent::PreviewCancelled => {}
        }
    }
}

fn render_stage(show_progress: bool, url_index: usize, url_total: usize, stage: &str) {
    if show_progress {
        eprint!("\r\u{1b}[2K[URL {url_index}/{url_total}] {stage}");
        let _ = io::stderr().flush();
    }
}

fn clear_progress_line() {
    eprint!("\r\u{1b}[2K");
    let _ = io::stderr().flush();
}

fn display_or_dash(value: &str) -> &str {
    if value.trim().is_empty() || value == "NA" {
        "-"
    } else {
        value
    }
}

fn text_progress_bar(percent: f64, width: usize) -> String {
    let fraction = (percent / 100.0).clamp(0.0, 1.0);
    let filled = (fraction * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn emit_report(report: &JsonReport, json: bool) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    if json {
        serde_json::to_writer(&mut writer, report).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
    } else {
        for path in report
            .results
            .iter()
            .flat_map(|result| result.succeeded.iter().map(|success| &success.path))
        {
            writeln!(writer, "{path}")?;
        }
    }
    writer.flush()
}

fn validation_failure(cli: &dloor_cli::Cli, error: ValidationError) -> ExitCode {
    fatal_failure(cli.json, error.to_string(), EXIT_USAGE)
}

fn fatal_failure(json: bool, message: String, exit_code: u8) -> ExitCode {
    if json {
        let report = JsonReport::fatal(message);
        if let Err(error) = emit_report(&report, true) {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            eprintln!("dloor-cli: could not write JSON output: {error}");
            return ExitCode::from(EXIT_FAILURE);
        }
    } else {
        eprintln!("dloor-cli: {message}");
    }
    ExitCode::from(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_bar_clamps_and_fills_to_the_requested_width() {
        assert_eq!(text_progress_bar(-1.0, 5), "░░░░░");
        assert_eq!(text_progress_bar(40.0, 5), "██░░░");
        assert_eq!(text_progress_bar(100.0, 5), "█████");
        assert_eq!(text_progress_bar(120.0, 5), "█████");
    }
}
