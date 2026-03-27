pub mod browser;
pub mod cli;
pub mod download;
pub mod error;
pub mod paths;
pub mod url;
pub mod ytdlp;

use clap::Parser;

use crate::cli::CliArgs;
use crate::download::{execute_download_plan, plan_download};
use crate::paths::resolve_home_dir;
use crate::ytdlp::{bootstrap_managed_ytdlp, resolve_ytdlp_binary};

pub use error::IgdlError;

pub fn run() -> Result<(), IgdlError> {
    let args = CliArgs::parse();
    let home = resolve_home_dir()?;
    let plan = plan_download(&args, &home)?;
    let ytdlp_binary = match resolve_ytdlp_binary(&home) {
        Ok(path) => path,
        Err(IgdlError::YtDlpBootstrap(_)) => bootstrap_managed_ytdlp(&home)?,
        Err(err) => return Err(err),
    };
    let outcome = execute_download_plan(&plan, &ytdlp_binary)?;

    for path in outcome.paths {
        println!("{}", path.display());
    }

    Ok(())
}
