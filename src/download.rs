use crate::browser::Browser;
use crate::cli::CliArgs;
use crate::error::IgdlError;
use crate::gallerydl::{
    ExtractedMediaItem, ImageDownloadProgressUpdate, MediaDownloadRequest,
    download_image_items_with_detailed_progress, download_image_items_with_progress,
    download_media_items_with_progress, extract_media_items,
};
use crate::paths::resolve_output_dir_from;
use crate::progress::{
    ByteProgress, ImageProgressDisplay, ImageProgressState, ProgressOutputMode,
    VideoProgressDisplay, render_image_progress_rows, render_item_progress,
    render_overall_progress, render_video_progress, select_progress_output_mode,
};
use crate::url::{InstagramUrlKind, instagram_url_kind, validate_instagram_url};
use crate::ytdlp::{YtDlpProgressUpdate, run_download_with_progress};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::io::Write;
use std::path::{Path, PathBuf};

const INTERACTIVE_FRAME_PREFIX: &str = "__IGDL_INTERACTIVE_FRAME__\n";
const REEL_PROGRESS_BAR_WIDTH: usize = 20;

#[derive(Debug, Eq, PartialEq)]
pub struct DownloadPlan {
    pub url: String,
    pub output_dir: PathBuf,
    pub browsers: Vec<Browser>,
    pub verbose: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadBinaries<'a> {
    pub ytdlp_binary: Option<&'a Path>,
    pub gallerydl_binary: Option<&'a Path>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DownloadOutcome {
    pub browser: Browser,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ExtractedMediaOutcome {
    pub browser: Browser,
    pub items: Vec<ExtractedMediaItem>,
}

pub fn plan_download(args: &CliArgs, home: &Path) -> Result<DownloadPlan, IgdlError> {
    validate_instagram_url(&args.url)?;

    Ok(DownloadPlan {
        url: args.url.clone(),
        output_dir: resolve_output_dir_from(args.output.clone(), home)?,
        browsers: crate::browser::browser_attempts(args.selected_browser()),
        verbose: args.verbose,
    })
}

pub fn execute_download_plan(
    plan: &DownloadPlan,
    binaries: DownloadBinaries<'_>,
) -> Result<DownloadOutcome, IgdlError> {
    let stderr = std::io::stderr();
    let is_tty = stderr.is_terminal();
    let mut stderr = stderr.lock();
    let mut rendered_progress = false;
    let mut previous_frame_line_count = 0usize;

    let result = execute_download_plan_with_progress_mode(plan, binaries, is_tty, |message| {
        rendered_progress = true;

        if let Some(frame) = interactive_frame_body(&message) {
            let _ = render_interactive_progress_frame(
                &mut stderr,
                frame,
                &mut previous_frame_line_count,
            );
        } else {
            if previous_frame_line_count > 0 {
                let _ = writeln!(stderr);
                previous_frame_line_count = 0;
            }
            let _ = writeln!(stderr, "{message}");
        }
    });

    if previous_frame_line_count > 0 && rendered_progress {
        let _ = writeln!(stderr);
    }

    result
}

pub fn execute_download_plan_with_progress<F>(
    plan: &DownloadPlan,
    binaries: DownloadBinaries<'_>,
    on_progress: F,
) -> Result<DownloadOutcome, IgdlError>
where
    F: FnMut(String),
{
    execute_download_plan_with_progress_mode(plan, binaries, false, on_progress)
}

fn execute_download_plan_with_progress_mode<F>(
    plan: &DownloadPlan,
    binaries: DownloadBinaries<'_>,
    is_tty: bool,
    mut on_progress: F,
) -> Result<DownloadOutcome, IgdlError>
where
    F: FnMut(String),
{
    match instagram_url_kind(&plan.url)? {
        InstagramUrlKind::Reel => execute_reel_download_plan(
            plan,
            binaries
                .ytdlp_binary
                .ok_or(IgdlError::MissingDownloadBinary("yt-dlp"))?,
            is_tty,
            &mut on_progress,
        ),
        InstagramUrlKind::PostMedia => execute_post_download_plan(
            plan,
            binaries
                .gallerydl_binary
                .ok_or(IgdlError::MissingDownloadBinary("gallery-dl"))?,
            binaries.ytdlp_binary,
            is_tty,
            &mut on_progress,
        ),
    }
}

fn execute_reel_download_plan(
    plan: &DownloadPlan,
    ytdlp_binary: &Path,
    is_tty: bool,
    on_progress: &mut dyn FnMut(String),
) -> Result<DownloadOutcome, IgdlError> {
    let mut attempts = Vec::with_capacity(plan.browsers.len());

    if !plan.verbose && !is_tty {
        on_progress(render_reel_progress_message(
            render_item_progress(1, 1, None, None),
            is_tty,
        ));
    }

    for &browser in &plan.browsers {
        if plan.verbose {
            eprintln!("trying browser cookies from {browser}");
        }

        match run_download_with_progress(
            ytdlp_binary,
            browser,
            &plan.url,
            &plan.output_dir,
            |progress| {
                if !plan.verbose {
                    on_progress(render_reel_download_progress(progress, is_tty));
                }
            },
        ) {
            Ok(paths) => attempts.push((browser, Ok(paths))),
            Err(err) => attempts.push((browser, Err(err.to_string()))),
        }

        if let Ok(outcome) = choose_successful_browser(attempts.clone()) {
            return Ok(outcome);
        }
    }

    choose_successful_browser(attempts)
}

fn execute_post_download_plan(
    plan: &DownloadPlan,
    gallerydl_binary: &Path,
    ytdlp_binary: Option<&Path>,
    is_tty: bool,
    on_progress: &mut dyn FnMut(String),
) -> Result<DownloadOutcome, IgdlError> {
    let mut failures = Vec::with_capacity(plan.browsers.len());

    for &browser in &plan.browsers {
        if plan.verbose {
            eprintln!("trying browser cookies from {browser}");
        }

        match extract_media_items(gallerydl_binary, browser, &plan.url, ytdlp_binary) {
            Ok(items) if !items.is_empty() => {
                let request = MediaDownloadRequest {
                    binary: gallerydl_binary,
                    browser,
                    url: &plan.url,
                    items: &items,
                    output_dir: &plan.output_dir,
                    ytdlp_binary,
                    verbose: plan.verbose,
                };
                let download_result = if !plan.verbose && is_tty && is_image_only_post(&items) {
                    download_image_items_with_interactive_progress(request, &items, on_progress)
                } else if is_multi_image_post(&items) {
                    download_image_items_with_progress(request, |completed, total| {
                        if !plan.verbose {
                            on_progress(render_item_progress(completed, total, None, None));
                        }
                    })
                } else {
                    download_media_items_with_progress(request, |completed| {
                        if !plan.verbose {
                            on_progress(render_overall_progress(completed, items.len()));
                        }
                    })
                };

                match download_result {
                    Ok(paths) if !paths.is_empty() => {
                        return Ok(DownloadOutcome { browser, paths });
                    }
                    Ok(_) => failures.push(format_post_media_failure(
                        browser,
                        IgdlError::DownloadProducedNoFiles.to_string(),
                    )),
                    Err(err @ IgdlError::PostMediaDownloadPartial { .. }) => return Err(err),
                    Err(err) => failures.push(format_post_media_failure(browser, err.to_string())),
                }
            }
            Ok(_) => failures.push(format_post_media_failure(
                browser,
                IgdlError::DownloadProducedNoFiles.to_string(),
            )),
            Err(message) => failures.push(format_post_media_failure(browser, message)),
        }
    }

    Err(IgdlError::PostMediaDownloadFailed(failures))
}

pub fn choose_successful_browser(
    attempts: Vec<(Browser, Result<Vec<PathBuf>, String>)>,
) -> Result<DownloadOutcome, IgdlError> {
    let (browser, paths) = choose_first_non_empty_attempt(attempts)?;

    Ok(DownloadOutcome { browser, paths })
}

pub fn choose_successful_media_extraction(
    attempts: Vec<(Browser, Result<Vec<ExtractedMediaItem>, String>)>,
) -> Result<ExtractedMediaOutcome, IgdlError> {
    let mut failures = Vec::new();

    for (browser, attempt) in attempts {
        match attempt {
            Ok(items) if !items.is_empty() => {
                return Ok(ExtractedMediaOutcome { browser, items });
            }
            Ok(_) => failures.push(format_post_media_failure(
                browser,
                IgdlError::DownloadProducedNoFiles.to_string(),
            )),
            Err(message) => failures.push(format_post_media_failure(browser, message)),
        }
    }

    Err(IgdlError::PostMediaDownloadFailed(failures))
}

fn choose_first_non_empty_attempt<T>(
    attempts: Vec<(Browser, Result<Vec<T>, String>)>,
) -> Result<(Browser, Vec<T>), IgdlError> {
    let mut failures = Vec::new();

    for (browser, attempt) in attempts {
        match attempt {
            Ok(items) if !items.is_empty() => {
                return Ok((browser, items));
            }
            Ok(_) => failures.push(format!("{browser}: {}", IgdlError::DownloadProducedNoFiles)),
            Err(message) => failures.push(format!("{browser}: {message}")),
        }
    }

    Err(IgdlError::BrowserCookiesUnavailable(failures))
}

fn download_image_items_with_interactive_progress(
    request: MediaDownloadRequest<'_>,
    items: &[ExtractedMediaItem],
    on_progress: &mut dyn FnMut(String),
) -> Result<Vec<PathBuf>, IgdlError> {
    let ordered_item_ids = ordered_image_item_ids(items);
    let mut ordered_rows = HashMap::with_capacity(ordered_item_ids.len());

    download_image_items_with_detailed_progress(request, |update| {
        ordered_rows.insert(update.item_id.clone(), image_progress_display(update));

        let rows = ordered_item_ids
            .iter()
            .filter_map(|item_id| ordered_rows.get(item_id).cloned())
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return;
        }

        on_progress(interactive_frame(render_image_progress_rows(
            &rows,
            ProgressOutputMode::Interactive,
            REEL_PROGRESS_BAR_WIDTH,
        )));
    })
}

fn ordered_image_item_ids(items: &[ExtractedMediaItem]) -> Vec<String> {
    let mut ordered = items.to_vec();
    ordered.sort_by_key(|item| item.index);
    ordered
        .into_iter()
        .map(|item| format!("{}_{:02}.{}", item.shortcode, item.index, item.extension))
        .collect()
}

fn image_progress_display(update: ImageDownloadProgressUpdate) -> ImageProgressDisplay {
    let state = if update.completed {
        ImageProgressState::Completed
    } else {
        ImageProgressState::Active(VideoProgressDisplay {
            percentage: update.percentage,
            bytes: Some(ByteProgress {
                downloaded_bytes: update.downloaded_bytes,
                total_bytes: update.total_bytes,
            }),
            speed_bytes_per_second: update.speed_bytes_per_second,
            eta: update.eta,
        })
    };

    ImageProgressDisplay {
        item_id: update.item_id,
        label: update.label,
        state,
    }
}

fn format_post_media_failure(browser: Browser, message: impl AsRef<str>) -> String {
    format!(
        "{browser}: {}",
        normalize_post_media_failure(message.as_ref())
    )
}

fn normalize_post_media_failure(message: &str) -> String {
    let message = message.trim();

    if message.contains("No video formats found") || message.contains("No media extracted") {
        return "post media unavailable".to_string();
    }

    message.to_string()
}

fn is_image_only_post(items: &[ExtractedMediaItem]) -> bool {
    !items.is_empty() && items.iter().all(|item| is_image_extension(&item.extension))
}

fn is_multi_image_post(items: &[ExtractedMediaItem]) -> bool {
    items.len() > 1 && is_image_only_post(items)
}

fn is_image_extension(extension: &str) -> bool {
    ["jpg", "jpeg", "png", "webp"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn interactive_frame(lines: Vec<String>) -> String {
    format!("{INTERACTIVE_FRAME_PREFIX}{}", lines.join("\n"))
}

fn interactive_frame_body(message: &str) -> Option<&str> {
    message.strip_prefix(INTERACTIVE_FRAME_PREFIX)
}

fn render_interactive_progress_frame(
    stderr: &mut dyn Write,
    frame: &str,
    previous_line_count: &mut usize,
) -> std::io::Result<()> {
    let width = interactive_terminal_width();
    let lines = frame
        .split('\n')
        .map(|line| truncate_interactive_line(line, width))
        .collect::<Vec<_>>();

    if *previous_line_count > 0 {
        write!(stderr, "\r")?;
        if *previous_line_count > 1 {
            write!(stderr, "\x1b[{}A", *previous_line_count - 1)?;
        }

        for (index, line) in lines.iter().enumerate() {
            write!(stderr, "\r\x1b[2K{line}")?;
            if index + 1 < lines.len() {
                writeln!(stderr)?;
            }
        }
    } else {
        for (index, line) in lines.iter().enumerate() {
            write!(stderr, "{line}")?;
            if index + 1 < lines.len() {
                writeln!(stderr)?;
            }
        }
    }

    stderr.flush()?;
    *previous_line_count = lines.len();
    Ok(())
}

fn interactive_terminal_width() -> Option<usize> {
    terminal_width_from_env().or_else(platform_terminal_width)
}

fn terminal_width_from_env() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|width| *width > 0)
}

#[cfg(unix)]
fn platform_terminal_width() -> Option<usize> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let status = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) };
    (status == 0 && size.ws_col > 0).then_some(size.ws_col as usize)
}

#[cfg(not(unix))]
fn platform_terminal_width() -> Option<usize> {
    None
}

fn truncate_interactive_line(line: &str, width: Option<usize>) -> String {
    let Some(width) = width else {
        return line.to_owned();
    };

    let line_width = line.chars().count();
    if line_width <= width {
        return line.to_owned();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let suffix = line
        .chars()
        .rev()
        .take(width - 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{suffix}")
}

fn render_reel_progress_message(message: String, is_tty: bool) -> String {
    if is_tty {
        interactive_frame(vec![message])
    } else {
        message
    }
}

fn render_reel_download_progress(progress: YtDlpProgressUpdate, is_tty: bool) -> String {
    let message = render_video_progress(
        VideoProgressDisplay {
            percentage: progress
                .percentage
                .map(|percentage| percentage.round().clamp(0.0, 100.0) as u8),
            bytes: progress
                .downloaded_bytes
                .map(|downloaded_bytes| ByteProgress {
                    downloaded_bytes,
                    total_bytes: progress.total_bytes,
                }),
            speed_bytes_per_second: progress.speed_bytes_per_second,
            eta: progress.eta,
        },
        select_progress_output_mode(is_tty),
        REEL_PROGRESS_BAR_WIDTH,
    );

    render_reel_progress_message(message, is_tty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn interactive_frame_truncates_long_lines_to_terminal_width() {
        with_columns_env("60", || {
            let mut stderr = Vec::new();
            let mut previous_line_count = 0;
            let frame = "seju-hideaki-ichikawa25-seju-seju-tokyo-shunkanseju-nicopuchi-official-DWBOMNRka5z-01.jpg [########------------] 42% | 1.1 MB/s | ETA 00:01";

            render_interactive_progress_frame(&mut stderr, frame, &mut previous_line_count)
                .unwrap();

            let rendered = String::from_utf8(stderr).unwrap();
            assert!(
                rendered.starts_with("..."),
                "unexpected output: {rendered:?}"
            );
            assert!(
                rendered.ends_with("[########------------] 42% | 1.1 MB/s | ETA 00:01"),
                "unexpected output: {rendered:?}"
            );
            assert!(
                rendered.chars().count() <= 60,
                "unexpected output: {rendered:?}"
            );
        });
    }

    fn with_columns_env<T>(columns: &str, f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("columns env lock should not be poisoned");
        let previous = std::env::var_os("COLUMNS");
        unsafe {
            std::env::set_var("COLUMNS", columns);
        }

        let result = f();

        if let Some(previous) = previous {
            unsafe {
                std::env::set_var("COLUMNS", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("COLUMNS");
            }
        }

        result
    }
}
