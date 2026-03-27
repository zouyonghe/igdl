pub mod browser;
pub mod cli;
pub mod download;
pub mod error;
pub mod gallerydl;
pub mod media;
pub mod paths;
pub mod progress;
pub mod url;
pub mod ytdlp;

use clap::Parser;

use crate::cli::CliArgs;
use crate::download::{execute_download_plan, plan_download, DownloadBinaries};
use crate::gallerydl::{bootstrap_managed_gallerydl, resolve_gallerydl_binary};
use crate::paths::resolve_home_dir;
use crate::url::{instagram_url_kind, InstagramUrlKind};
use crate::ytdlp::{bootstrap_managed_ytdlp, resolve_ytdlp_binary};

pub use error::IgdlError;

pub fn run() -> Result<(), IgdlError> {
    let args = CliArgs::parse();
    let home = resolve_home_dir()?;
    let plan = plan_download(&args, &home)?;
    let url_kind = instagram_url_kind(&plan.url)?;
    let ytdlp_binary = match url_kind {
        InstagramUrlKind::Reel => Some(match resolve_ytdlp_binary(&home) {
            Ok(path) => path,
            Err(IgdlError::YtDlpBootstrap(_)) => bootstrap_managed_ytdlp(&home)?,
            Err(err) => return Err(err),
        }),
        InstagramUrlKind::PostMedia => match resolve_ytdlp_binary(&home) {
            Ok(path) => Some(path),
            Err(IgdlError::YtDlpBootstrap(_)) => bootstrap_managed_ytdlp(&home).ok(),
            Err(err) => return Err(err),
        },
    };
    let gallerydl_binary = match url_kind {
        InstagramUrlKind::PostMedia => Some(match resolve_gallerydl_binary(&home) {
            Ok(path) => path,
            Err(IgdlError::GalleryDlBootstrap(_)) => bootstrap_managed_gallerydl(&home)?,
            Err(err) => return Err(err),
        }),
        InstagramUrlKind::Reel => None,
    };
    let outcome = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: ytdlp_binary.as_deref(),
            gallerydl_binary: gallerydl_binary.as_deref(),
        },
    )?;

    for path in outcome.paths {
        println!("{}", path.display());
    }

    Ok(())
}
