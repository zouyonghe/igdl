use crate::browser::Browser;
use crate::cli::CliArgs;
use crate::error::IgdlError;
use crate::paths::resolve_output_dir_from;
use crate::url::validate_instagram_url;
use crate::ytdlp::{build_download_command, describe_command_failure, parse_downloaded_paths};
use std::path::{Path, PathBuf};

#[derive(Debug, Eq, PartialEq)]
pub struct DownloadPlan {
    pub url: String,
    pub output_dir: PathBuf,
    pub browsers: Vec<Browser>,
    pub verbose: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DownloadOutcome {
    pub browser: Browser,
    pub paths: Vec<PathBuf>,
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
    ytdlp_binary: &Path,
) -> Result<DownloadOutcome, IgdlError> {
    let mut attempts = Vec::with_capacity(plan.browsers.len());

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

pub fn choose_successful_browser(
    attempts: Vec<(Browser, Result<Vec<PathBuf>, String>)>,
) -> Result<DownloadOutcome, IgdlError> {
    let mut failures = Vec::new();

    for (browser, attempt) in attempts {
        match attempt {
            Ok(paths) if !paths.is_empty() => {
                return Ok(DownloadOutcome { browser, paths });
            }
            Ok(_) => failures.push(format!("{browser}: {}", IgdlError::DownloadProducedNoFiles)),
            Err(message) => failures.push(format!("{browser}: {message}")),
        }
    }

    Err(IgdlError::BrowserCookiesUnavailable(failures))
}
