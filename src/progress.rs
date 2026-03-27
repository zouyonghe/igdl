#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
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
