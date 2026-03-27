const MAX_FILENAME_COMPONENT_LEN: usize = 255;

pub fn build_media_filename(
    base: &str,
    shortcode: &str,
    index: Option<usize>,
    ext: &str,
) -> String {
    let ext = ext.trim_start_matches('.');
    let suffix = match (index, ext.is_empty()) {
        (Some(index), false) => format!("-{shortcode}-{index:02}.{ext}"),
        (Some(index), true) => format!("-{shortcode}-{index:02}"),
        (None, false) => format!("-{shortcode}.{ext}"),
        (None, true) => format!("-{shortcode}"),
    };
    let fragment = truncate_fragment(
        &sanitize_fragment(base),
        MAX_FILENAME_COMPONENT_LEN.saturating_sub(suffix.len()),
    );

    format!("{fragment}{suffix}")
}

fn sanitize_fragment(input: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_was_separator = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            sanitized.push('-');
            previous_was_separator = true;
        }
    }

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "media".to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn truncate_fragment(fragment: &str, max_len: usize) -> String {
    if fragment.len() <= max_len {
        return fragment.to_owned();
    }

    let mut truncated = fragment.chars().take(max_len).collect::<String>();
    while truncated.ends_with('-') {
        truncated.pop();
    }

    if truncated.is_empty() {
        "media".chars().take(max_len).collect()
    } else {
        truncated
    }
}
