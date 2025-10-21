use sha2::{Digest, Sha256};

pub fn collapse_repeats(input: &str, max_repeat: usize) -> String {
    if max_repeat == 0 || input.is_empty() {
        return String::new();
    }

    let mut buf = String::with_capacity(input.len());
    let mut prev: Option<char> = None;
    let mut count = 0usize;

    for ch in input.chars() {
        if Some(ch) == prev {
            count += 1;
            if count <= max_repeat {
                buf.push(ch);
            }
        } else {
            prev = Some(ch);
            count = 1;
            buf.push(ch);
        }
    }

    buf
}

pub fn normalize_body(input: &str) -> String {
    let trimmed = input.trim();
    let lowered = trimmed.to_lowercase();
    let collapsed = collapse_repeats(&lowered, 3);
    collapsed.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn dedupe_hash(title: &str, body: &str) -> String {
    let normalized_title = collapse_repeats(title.trim().to_lowercase().as_str(), 3);
    let normalized_body = normalize_body(body);
    let mut hasher = Sha256::new();
    hasher.update(normalized_title.as_bytes());
    hasher.update(b"\n");
    hasher.update(normalized_body.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_limits_repeats() {
        assert_eq!(collapse_repeats("heyyyy!!!", 3), "heyy!!!");
    }

    #[test]
    fn normalize_body_dedupes_whitespace() {
        assert_eq!(normalize_body("  Hello   There \n"), "hello there");
    }

    #[test]
    fn dedupe_hash_stable() {
        let first = dedupe_hash("Title", "Body");
        let second = dedupe_hash("Title", "Body");
        assert_eq!(first, second);
    }
}
