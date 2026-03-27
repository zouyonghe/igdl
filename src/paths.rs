use std::path::{Path, PathBuf};

use crate::error::IgdlError;

pub fn resolve_home_dir() -> Result<PathBuf, IgdlError> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(IgdlError::HomeDirectoryUnavailable)
}

pub fn managed_binary_path_from(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Caches")
        .join("igdl")
        .join("yt-dlp")
}

/// Resolves the output directory and creates the chosen directory on disk.
pub fn resolve_output_dir_from(
    override_dir: Option<PathBuf>,
    home: &Path,
) -> Result<PathBuf, IgdlError> {
    let output_dir = override_dir.unwrap_or_else(|| default_output_dir(home));
    std::fs::create_dir_all(&output_dir)?;
    Ok(output_dir)
}

fn default_output_dir(home: &Path) -> PathBuf {
    let videos_dir = home.join("Videos");
    if videos_dir.is_dir() {
        return videos_dir.join("instagram");
    }

    let movies_dir = home.join("Movies");
    if movies_dir.is_dir() {
        return movies_dir.join("instagram");
    }

    home.join("Videos").join("instagram")
}
