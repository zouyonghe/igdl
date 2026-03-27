use std::path::{Path, PathBuf};

use crate::error::IgdlError;

pub fn resolve_home_dir() -> Result<PathBuf, IgdlError> {
    resolve_home_dir_from(dirs::home_dir(), env_path("HOME"))
}

pub fn managed_binary_path_from(home: &Path) -> PathBuf {
    managed_binary_path_for(
        std::env::consts::OS,
        dirs::cache_dir().as_deref(),
        dirs::home_dir().as_deref(),
        home,
    )
}

/// Resolves the output directory and creates the chosen directory on disk.
///
/// ```compile_fail
/// use igdl::paths::default_output_dir_for;
/// ```
pub fn resolve_output_dir_from(
    override_dir: Option<PathBuf>,
    home: &Path,
) -> Result<PathBuf, IgdlError> {
    let native_home_dir = dirs::home_dir();
    let native_video_dir = dirs::video_dir();
    let xdg_videos_dir = env_path("XDG_VIDEOS_DIR");
    let output_dir = override_dir.unwrap_or_else(|| {
        default_output_dir_for(
            std::env::consts::OS,
            native_video_dir.as_deref(),
            xdg_videos_dir.as_deref(),
            native_home_dir.as_deref(),
            home,
        )
    });
    std::fs::create_dir_all(&output_dir)?;
    Ok(output_dir)
}

fn resolve_home_dir_from(
    native_home_dir: Option<PathBuf>,
    env_home_dir: Option<PathBuf>,
) -> Result<PathBuf, IgdlError> {
    native_home_dir
        .or(env_home_dir)
        .ok_or(IgdlError::HomeDirectoryUnavailable)
}

fn managed_binary_path_for(
    os: &str,
    native_cache_dir: Option<&Path>,
    native_home_dir: Option<&Path>,
    home: &Path,
) -> PathBuf {
    default_cache_dir_for(os, native_cache_dir, native_home_dir, home).join("yt-dlp")
}

fn default_output_dir_for(
    os: &str,
    native_video_dir: Option<&Path>,
    xdg_videos_dir: Option<&Path>,
    native_home_dir: Option<&Path>,
    home: &Path,
) -> PathBuf {
    match os {
        "macos" => {
            let movies_dir = native_dir_for_home(native_video_dir, native_home_dir, home)
                .unwrap_or_else(|| home.join("Movies"));
            if movies_dir.is_dir() {
                return movies_dir.join("instagram");
            }

            let videos_dir = home.join("Videos");
            if videos_dir.is_dir() {
                return videos_dir.join("instagram");
            }

            movies_dir.join("instagram")
        }
        "linux" => {
            if let Some(xdg_videos_dir) = native_dir_for_home(xdg_videos_dir, native_home_dir, home)
            {
                return xdg_videos_dir.join("instagram");
            }

            if let Some(native_video_dir) =
                native_dir_for_home(native_video_dir, native_home_dir, home)
            {
                return native_video_dir.join("instagram");
            }

            home.join("Videos").join("instagram")
        }
        "windows" => native_dir_for_home(native_video_dir, native_home_dir, home)
            .unwrap_or_else(|| home.join("Videos"))
            .join("instagram"),
        _ => native_dir_for_home(native_video_dir, native_home_dir, home)
            .unwrap_or_else(|| home.join("Videos"))
            .join("instagram"),
    }
}

fn default_cache_dir_for(
    os: &str,
    native_cache_dir: Option<&Path>,
    native_home_dir: Option<&Path>,
    home: &Path,
) -> PathBuf {
    if let Some(native_cache_dir) = native_dir_for_home(native_cache_dir, native_home_dir, home) {
        return native_cache_dir.join("igdl");
    }

    match os {
        "macos" => home.join("Library").join("Caches").join("igdl"),
        "windows" => home.join("AppData").join("Local").join("igdl"),
        _ => home.join(".cache").join("igdl"),
    }
}

fn native_dir_for_home(
    native_dir: Option<&Path>,
    native_home_dir: Option<&Path>,
    home: &Path,
) -> Option<PathBuf> {
    match (native_dir, native_home_dir) {
        (Some(native_dir), Some(native_home_dir)) if native_home_dir == home => {
            Some(native_dir.to_path_buf())
        }
        _ => None,
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{
        default_output_dir_for, managed_binary_path_for, resolve_home_dir, resolve_home_dir_from,
    };

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_home_dir_uses_native_lookup_when_home_env_missing() {
        let _lock = HOME_LOCK.lock().unwrap();
        let expected = dirs::home_dir().expect("native home directory should be discoverable");
        let _guard = HomeGuard::unset();

        let resolved = resolve_home_dir().unwrap();

        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_home_dir_falls_back_to_home_env_when_native_lookup_is_unavailable() {
        let env_home = PathBuf::from("/tmp/igdl-home");

        let resolved = resolve_home_dir_from(None, Some(env_home.clone())).unwrap();

        assert_eq!(resolved, env_home);
    }

    #[test]
    fn resolve_home_dir_prefers_native_lookup_over_home_env() {
        let native_home = PathBuf::from("/tmp/native-home");
        let env_home = PathBuf::from("/tmp/env-home");

        let resolved = resolve_home_dir_from(Some(native_home.clone()), Some(env_home)).unwrap();

        assert_eq!(resolved, native_home);
    }

    #[test]
    fn default_output_dir_prefers_movies_on_macos() {
        let home = tempdir().unwrap();
        std::fs::create_dir(home.path().join("Videos")).unwrap();
        std::fs::create_dir(home.path().join("Movies")).unwrap();

        let dir = default_output_dir_for("macos", None, None, None, home.path());

        assert_eq!(dir, home.path().join("Movies").join("instagram"));
    }

    #[test]
    fn default_output_dir_uses_videos_on_macos_when_movies_missing() {
        let home = tempdir().unwrap();
        std::fs::create_dir(home.path().join("Videos")).unwrap();

        let dir = default_output_dir_for("macos", None, None, None, home.path());

        assert_eq!(dir, home.path().join("Videos").join("instagram"));
    }

    #[test]
    fn default_output_dir_falls_back_to_movies_on_macos_when_neither_exists() {
        let home = tempdir().unwrap();

        let dir = default_output_dir_for("macos", None, None, None, home.path());

        assert_eq!(dir, home.path().join("Movies").join("instagram"));
    }

    #[test]
    fn default_output_dir_prefers_native_videos_dir_on_linux() {
        let home = tempdir().unwrap();
        let native_videos_dir = home.path().join("xdg-videos");

        let dir = default_output_dir_for(
            "linux",
            Some(native_videos_dir.as_path()),
            None,
            Some(home.path()),
            home.path(),
        );

        assert_eq!(dir, native_videos_dir.join("instagram"));
    }

    #[test]
    fn default_output_dir_prefers_xdg_videos_dir_on_linux_for_native_home() {
        let home = tempdir().unwrap();
        let xdg_videos_dir = home.path().join("xdg-videos");

        let dir = default_output_dir_for(
            "linux",
            None,
            Some(xdg_videos_dir.as_path()),
            Some(home.path()),
            home.path(),
        );

        assert_eq!(dir, xdg_videos_dir.join("instagram"));
    }

    #[test]
    fn default_output_dir_ignores_xdg_videos_dir_when_home_differs_from_native_home() {
        let home = tempdir().unwrap();
        let native_home = tempdir().unwrap();
        let xdg_videos_dir = native_home.path().join("xdg-videos");

        let dir = default_output_dir_for(
            "linux",
            None,
            Some(xdg_videos_dir.as_path()),
            Some(native_home.path()),
            home.path(),
        );

        assert_eq!(dir, home.path().join("Videos").join("instagram"));
    }

    #[test]
    fn default_output_dir_falls_back_to_videos_on_linux_without_xdg() {
        let home = tempdir().unwrap();

        let dir = default_output_dir_for("linux", None, None, None, home.path());

        assert_eq!(dir, home.path().join("Videos").join("instagram"));
    }

    #[test]
    fn default_output_dir_uses_native_videos_on_windows() {
        let home = tempdir().unwrap();
        let native_videos_dir = home.path().join("KnownFolders").join("Videos");

        let dir = default_output_dir_for(
            "windows",
            Some(native_videos_dir.as_path()),
            None,
            Some(home.path()),
            home.path(),
        );

        assert_eq!(dir, native_videos_dir.join("instagram"));
    }

    #[test]
    fn default_output_dir_falls_back_to_videos_on_windows_when_native_lookup_misses() {
        let home = tempdir().unwrap();

        let dir = default_output_dir_for("windows", None, None, None, home.path());

        assert_eq!(dir, home.path().join("Videos").join("instagram"));
    }

    #[test]
    fn managed_binary_path_prefers_native_cache_dir_when_home_matches() {
        let home = tempdir().unwrap();
        let native_cache_dir = home.path().join("AppData").join("Local");

        let path = managed_binary_path_for(
            "windows",
            Some(native_cache_dir.as_path()),
            Some(home.path()),
            home.path(),
        );

        assert_eq!(path, native_cache_dir.join("igdl").join("yt-dlp"));
    }

    #[test]
    fn managed_binary_path_uses_linux_cache_fallback_when_native_lookup_misses() {
        let home = tempdir().unwrap();

        let path = managed_binary_path_for("linux", None, None, home.path());

        assert_eq!(path, home.path().join(".cache").join("igdl").join("yt-dlp"));
    }

    #[test]
    fn managed_binary_path_uses_windows_cache_fallback_when_native_lookup_misses() {
        let home = tempdir().unwrap();

        let path = managed_binary_path_for("windows", None, None, home.path());

        assert_eq!(
            path,
            home.path()
                .join("AppData")
                .join("Local")
                .join("igdl")
                .join("yt-dlp")
        );
    }

    struct HomeGuard {
        original: Option<OsString>,
    }

    impl HomeGuard {
        fn unset() -> Self {
            let original = std::env::var_os("HOME");
            unsafe {
                std::env::remove_var("HOME");
            }
            Self { original }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.original.as_ref() {
                Some(value) => unsafe {
                    std::env::set_var("HOME", value);
                },
                None => unsafe {
                    std::env::remove_var("HOME");
                },
            }
        }
    }
}
