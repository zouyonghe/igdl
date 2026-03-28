use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoProgressDisplay {
    pub percentage: Option<u8>,
    pub bytes: Option<ByteProgress>,
    pub speed_bytes_per_second: Option<u64>,
    pub eta: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageProgressState {
    Active(VideoProgressDisplay),
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageProgressDisplay {
    pub item_id: String,
    pub label: String,
    pub state: ImageProgressState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressOutputMode {
    Interactive,
    Plain,
}

pub fn select_progress_output_mode(is_tty: bool) -> ProgressOutputMode {
    if is_tty {
        ProgressOutputMode::Interactive
    } else {
        ProgressOutputMode::Plain
    }
}

pub fn render_video_progress(
    progress: VideoProgressDisplay,
    mode: ProgressOutputMode,
    bar_width: usize,
) -> String {
    match mode {
        ProgressOutputMode::Interactive => render_dynamic_progress_bar(progress, bar_width),
        ProgressOutputMode::Plain => render_non_tty_progress_line(progress),
    }
}

pub fn render_image_progress_row(
    progress: &ImageProgressDisplay,
    mode: ProgressOutputMode,
    label_width: usize,
    bar_width: usize,
) -> String {
    let detail = match (mode, progress.state) {
        (_, ImageProgressState::Completed) => "100% | done".to_owned(),
        (ProgressOutputMode::Interactive, ImageProgressState::Active(progress)) => {
            render_dynamic_progress_bar(progress, bar_width)
        }
        (ProgressOutputMode::Plain, ImageProgressState::Active(progress)) => {
            render_non_tty_progress_line(progress)
        }
    };

    render_labeled_progress_row(&progress.label, label_width, &detail)
}

pub fn render_image_progress_rows(
    progress: &[ImageProgressDisplay],
    mode: ProgressOutputMode,
    bar_width: usize,
) -> Vec<String> {
    let label_width = progress
        .iter()
        .map(|row| row.label.len())
        .max()
        .unwrap_or(0);

    progress
        .iter()
        .map(|row| render_image_progress_row(row, mode, label_width, bar_width))
        .collect()
}

pub fn render_dynamic_progress_bar(progress: VideoProgressDisplay, bar_width: usize) -> String {
    let bar = render_progress_bar(progress, bar_width);
    let mut detail = Vec::new();

    if let Some(percentage) = progress.percentage {
        detail.push(format!("{percentage}%"));
    }
    if let Some(speed) = progress.speed_bytes_per_second {
        detail.push(render_speed(speed));
    }
    if let Some(eta) = progress.eta {
        detail.push(format!("ETA {}", render_eta(eta)));
    }

    if detail.is_empty() {
        bar
    } else {
        format!("{bar} {}", detail.join(" | "))
    }
}

pub fn render_non_tty_progress_line(progress: VideoProgressDisplay) -> String {
    let mut detail = Vec::new();

    if let Some(percentage) = progress.percentage {
        detail.push(format!("{percentage}%"));
    }
    if let Some(bytes) = progress.bytes {
        detail.push(render_byte_progress(bytes));
    }
    if let Some(speed) = progress.speed_bytes_per_second {
        detail.push(render_speed(speed));
    }
    if let Some(eta) = progress.eta {
        detail.push(format!("ETA {}", render_eta(eta)));
    }

    detail.join(" | ")
}

pub fn render_overall_progress(completed: usize, total: usize) -> String {
    format!("{completed}/{total}")
}

pub fn render_item_progress(
    current: usize,
    total: usize,
    percentage: Option<u8>,
    bytes: Option<ByteProgress>,
) -> String {
    let prefix = format!("Downloading {}/{}", current, total);
    let detail = match (percentage, bytes) {
        (Some(percentage), Some(bytes)) => {
            format!("{percentage}% ({})", render_byte_progress(bytes))
        }
        (Some(percentage), None) => format!("{percentage}%"),
        (None, Some(bytes)) => render_byte_progress(bytes),
        (None, None) => String::new(),
    };

    if detail.is_empty() {
        prefix
    } else {
        format!("{prefix}: {detail}")
    }
}

fn render_byte_progress(bytes: ByteProgress) -> String {
    match bytes.total_bytes {
        Some(total_bytes) => {
            format!(
                "{} / {}",
                render_byte_count(bytes.downloaded_bytes),
                render_byte_count(total_bytes)
            )
        }
        None => render_byte_count(bytes.downloaded_bytes),
    }
}

fn render_labeled_progress_row(label: &str, label_width: usize, detail: &str) -> String {
    if label_width == 0 {
        return detail.to_owned();
    }

    format!("{label:<label_width$} {detail}")
}

fn render_progress_bar(progress: VideoProgressDisplay, bar_width: usize) -> String {
    if bar_width == 0 {
        return "[]".to_owned();
    }

    let filled = match progress_fraction(progress) {
        Some(fraction) if fraction > 0.0 => {
            let filled = (fraction * bar_width as f64).floor() as usize;
            filled.clamp(1, bar_width)
        }
        Some(_) | None => 0,
    };
    let empty = bar_width.saturating_sub(filled);

    format!("[{}{}]", "#".repeat(filled), "-".repeat(empty))
}

fn progress_fraction(progress: VideoProgressDisplay) -> Option<f64> {
    if let Some(percentage) = progress.percentage {
        return Some((percentage as f64 / 100.0).clamp(0.0, 1.0));
    }

    let bytes = progress.bytes?;
    let total_bytes = bytes.total_bytes?;
    if total_bytes == 0 {
        return None;
    }

    Some((bytes.downloaded_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0))
}

fn render_byte_count(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    if bytes < 1000 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1000.0 && unit_index < UNITS.len() - 1 {
        value /= 1000.0;
        unit_index += 1;
    }

    while round_to_one_decimal(value) >= 1000.0 && unit_index < UNITS.len() - 1 {
        value /= 1000.0;
        unit_index += 1;
    }

    format!("{value:.1} {}", UNITS[unit_index])
}

fn round_to_one_decimal(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn render_speed(bytes_per_second: u64) -> String {
    format!("{}/s", render_byte_count(bytes_per_second))
}

fn render_eta(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
