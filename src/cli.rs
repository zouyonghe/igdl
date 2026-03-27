use crate::browser::Browser;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(name = "igdl")]
pub struct CliArgs {
    pub url: String,

    #[arg(long)]
    pub browser: Option<BrowserArg>,

    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub verbose: bool,
}

impl CliArgs {
    pub fn selected_browser(&self) -> Option<Browser> {
        self.browser.map(Into::into)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum BrowserArg {
    Chrome,
    Edge,
    Brave,
    Firefox,
    Safari,
}
