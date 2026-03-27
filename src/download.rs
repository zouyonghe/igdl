use crate::browser::Browser;
use crate::cli::CliArgs;
use crate::error::IgdlError;
use crate::gallerydl::{
    download_media_items_with_progress, extract_media_items, ExtractedMediaItem,
    MediaDownloadRequest,
};
use crate::paths::resolve_output_dir_from;
use crate::progress::{render_item_progress, render_overall_progress};
use crate::url::{instagram_url_kind, validate_instagram_url, InstagramUrlKind};
use crate::ytdlp::{build_download_command, describe_command_failure, parse_downloaded_paths};
use std::path::{Path, PathBuf};

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
    execute_download_plan_with_progress(plan, binaries, |message| eprintln!("{message}"))
}

pub fn execute_download_plan_with_progress<F>(
    plan: &DownloadPlan,
    binaries: DownloadBinaries<'_>,
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
            &mut on_progress,
        ),
        InstagramUrlKind::PostMedia => execute_post_download_plan(
            plan,
            binaries
                .gallerydl_binary
                .ok_or(IgdlError::MissingDownloadBinary("gallery-dl"))?,
            binaries.ytdlp_binary,
            &mut on_progress,
        ),
    }
}

fn execute_reel_download_plan(
    plan: &DownloadPlan,
    ytdlp_binary: &Path,
    on_progress: &mut dyn FnMut(String),
) -> Result<DownloadOutcome, IgdlError> {
    let mut attempts = Vec::with_capacity(plan.browsers.len());

    if !plan.verbose {
        on_progress(render_item_progress(1, 1, None, None));
    }

    for &browser in &plan.browsers {
        if plan.verbose {
            eprintln!("trying browser cookies from {browser}");
        }

        let output =
            build_download_command(ytdlp_binary, browser, &plan.url, &plan.output_dir).output()?;

        if output.status.success() {
            attempts.push((
                browser,
                Ok(parse_downloaded_paths(&String::from_utf8_lossy(
                    &output.stdout,
                ))),
            ));
        } else {
            attempts.push((
                browser,
                Err(describe_command_failure(&output.status, &output.stderr)),
            ));
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
    on_progress: &mut dyn FnMut(String),
) -> Result<DownloadOutcome, IgdlError> {
    let mut failures = Vec::with_capacity(plan.browsers.len());

    for &browser in &plan.browsers {
        if plan.verbose {
            eprintln!("trying browser cookies from {browser}");
        }

        match extract_media_items(gallerydl_binary, browser, &plan.url, ytdlp_binary) {
            Ok(items) if !items.is_empty() => match download_media_items_with_progress(
                MediaDownloadRequest {
                    binary: gallerydl_binary,
                    browser,
                    url: &plan.url,
                    items: &items,
                    output_dir: &plan.output_dir,
                    ytdlp_binary,
                    verbose: plan.verbose,
                },
                |completed| {
                    if !plan.verbose {
                        on_progress(render_overall_progress(completed, items.len()));
                    }
                },
            ) {
                Ok(paths) if !paths.is_empty() => return Ok(DownloadOutcome { browser, paths }),
                Ok(_) => failures.push(format_post_media_failure(
                    browser,
                    IgdlError::DownloadProducedNoFiles.to_string(),
                )),
                Err(err @ IgdlError::PostMediaDownloadPartial { .. }) => return Err(err),
                Err(err) => failures.push(format_post_media_failure(browser, err.to_string())),
            },
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
