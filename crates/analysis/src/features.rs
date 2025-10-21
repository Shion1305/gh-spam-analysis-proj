use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use db::{CommentRow, IssueRow, UserRow};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ContributionStats {
    pub posts_last_24h: u32,
    pub dedupe_hits_last_48h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FeatureSet {
    pub body_length: usize,
    pub url_count: usize,
    pub email_count: usize,
    pub mention_count: usize,
    pub emoji_count: usize,
    pub repeated_char_ratio: f32,
    pub token_entropy: f32,
    pub title_body_similarity: Option<f32>,
    pub account_age_days: Option<f32>,
    pub posts_last_24h: u32,
    pub default_template_hit: bool,
}

pub fn features_for_issue(
    issue: &IssueRow,
    user: Option<&UserRow>,
    stats: ContributionStats,
) -> FeatureSet {
    let body = issue.body.as_deref().unwrap_or("");
    let base = base_features(body);
    FeatureSet {
        title_body_similarity: Some(title_body_similarity(&issue.title, body)),
        account_age_days: account_age_days(user),
        posts_last_24h: stats.posts_last_24h,
        default_template_hit: default_template_hit(body),
        ..base
    }
}

pub fn features_for_comment(
    comment: &CommentRow,
    user: Option<&UserRow>,
    stats: ContributionStats,
) -> FeatureSet {
    let base = base_features(&comment.body);
    FeatureSet {
        title_body_similarity: None,
        account_age_days: account_age_days(user),
        posts_last_24h: stats.posts_last_24h,
        default_template_hit: default_template_hit(&comment.body),
        ..base
    }
}

fn base_features(body: &str) -> FeatureSet {
    FeatureSet {
        body_length: body.chars().count(),
        url_count: count_urls(body),
        email_count: count_emails(body),
        mention_count: count_mentions(body),
        emoji_count: count_emojis(body),
        repeated_char_ratio: repeated_char_ratio(body),
        token_entropy: token_entropy(body),
        title_body_similarity: None,
        account_age_days: None,
        posts_last_24h: 0,
        default_template_hit: default_template_hit(body),
    }
}

fn count_urls(text: &str) -> usize {
    lazy_regex!(URL_RE = r"https?://[\w\-./?=&%#+]+");
    URL_RE.find_iter(text).count()
}

fn count_emails(text: &str) -> usize {
    lazy_regex!(EMAIL_RE = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}");
    EMAIL_RE.find_iter(text).count()
}

fn count_mentions(text: &str) -> usize {
    lazy_regex!(MENTION_RE = r"@[A-Za-z0-9][A-Za-z0-9\-]{0,38}");
    MENTION_RE.find_iter(text).count()
}

fn count_emojis(text: &str) -> usize {
    text.chars()
        .filter(|c| c.is_ascii_punctuation() == false && emojis::is_emoji(*c))
        .count()
}

fn repeated_char_ratio(text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut repeats = 0usize;
    let mut total = 0usize;
    let mut prev: Option<char> = None;
    let mut run = 0usize;
    for ch in text.chars() {
        total += 1;
        if Some(ch) == prev {
            run += 1;
            if run > 2 {
                repeats += 1;
            }
        } else {
            prev = Some(ch);
            run = 0;
        }
    }
    repeats as f32 / total as f32
}

fn token_entropy(text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    let words = text.split_whitespace().collect::<Vec<_>>();
    for word in &words {
        *counts.entry(word.to_ascii_lowercase()).or_insert(0usize) += 1;
    }
    let total = words.len() as f32;
    counts
        .values()
        .map(|&count| {
            let p = count as f32 / total;
            -p * p.log2()
        })
        .sum()
}

fn title_body_similarity(title: &str, body: &str) -> f32 {
    let title_tokens = tokenize(title);
    let body_tokens = tokenize(body);
    if title_tokens.is_empty() || body_tokens.is_empty() {
        return 0.0;
    }
    let title_set: std::collections::HashSet<_> = title_tokens.into_iter().collect();
    let body_set: std::collections::HashSet<_> = body_tokens.into_iter().collect();
    let intersection = title_set.intersection(&body_set).count() as f32;
    let union = title_set.union(&body_set).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn default_template_hit(body: &str) -> bool {
    const PHRASES: &[&str] = &[
        "thanks for submitting",
        "please fill out the template",
        "bug report",
        "feature request",
        "what happened",
    ];
    let lower = body.to_ascii_lowercase();
    PHRASES.iter().any(|p| lower.contains(p))
}

fn account_age_days(user: Option<&UserRow>) -> Option<f32> {
    let user = user?;
    let created_at = user.created_at?;
    let age = Utc::now() - created_at;
    Some(age.num_seconds().max(0) as f32 / 86_400.0)
}

macro_rules! lazy_regex {
    ($name:ident = $pattern:expr) => {
        static $name: once_cell::sync::Lazy<Regex> =
            once_cell::sync::Lazy::new(|| Regex::new($pattern).expect("invalid regex"));
    };
}

mod emojis {
    pub fn is_emoji(ch: char) -> bool {
        matches!(ch as u32,
            0x1F300..=0x1F6FF |
            0x1F900..=0x1F9FF |
            0x1FA70..=0x1FAFF |
            0x2600..=0x26FF |
            0x2700..=0x27BF)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_ratio_does_not_overflow() {
        assert_eq!(repeated_char_ratio(""), 0.0);
    }

    #[test]
    fn entropy_less_for_repeats() {
        let high = token_entropy("hello world unique words");
        let low = token_entropy("spam spam spam");
        assert!(low < high);
    }

    #[test]
    fn template_detects_phrase() {
        assert!(default_template_hit("Thanks for submitting the bug report"));
    }
}
