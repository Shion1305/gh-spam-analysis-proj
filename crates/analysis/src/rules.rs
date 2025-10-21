use crate::features::{ContributionStats, FeatureSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RuleOutcome {
    pub score: f32,
    pub reasons: Vec<String>,
}

impl Default for RuleOutcome {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleOutcome {
    pub fn new() -> Self {
        Self {
            score: 0.0,
            reasons: Vec::new(),
        }
    }

    fn push(&mut self, delta: f32, reason: impl Into<String>) {
        self.score += delta;
        self.reasons.push(reason.into());
    }
}

pub struct RuleContext<'a> {
    pub body: &'a str,
    pub stats: &'a ContributionStats,
    pub dedupe_hits_last_48h: u32,
}

pub struct RuleEngine {
    version: &'static str,
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self {
            version: "rules_v1",
        }
    }
}

impl RuleEngine {
    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn evaluate(&self, features: &FeatureSet, ctx: RuleContext<'_>) -> RuleOutcome {
        let mut outcome = RuleOutcome::new();
        let body = ctx.body;

        if is_contact_only(body) {
            outcome.push(2.0, "contact_only");
        }

        if features.body_length < 40
            && (features.emoji_count > 5 || features.repeated_char_ratio > 0.2)
        {
            outcome.push(1.5, "short_with_noise");
        }

        if features.repeated_char_ratio > 0.2 {
            outcome.push(1.0, "repeated_chars");
        }

        if features.url_count > 5 || features.mention_count > 5 {
            outcome.push(1.0, "excessive_links_mentions");
        }

        if features.token_entropy < 1.5 {
            outcome.push(1.0, "low_entropy");
        }

        if features.default_template_hit {
            outcome.push(1.5, "template_phrase");
        }

        if let Some(age_days) = features.account_age_days {
            if age_days < 7.0 && ctx.stats.posts_last_24h >= 3 {
                outcome.push(2.5, "new_account_heavy_posting");
            }
        }

        if ctx.dedupe_hits_last_48h >= 3 {
            outcome.push(3.0, "dedupe_hash_reused");
        }

        outcome
    }
}

fn is_contact_only(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let trimmed = lower.trim();
    if trimmed.is_empty() {
        return false;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let contact_phrases = ["contact", "email", "reach", "whatsapp", "telegram"];
    let has_contact_word = contact_phrases.iter().any(|w| trimmed.contains(w));
    let non_contact_tokens = tokens
        .iter()
        .filter(|token| token.chars().any(|c| c.is_alphabetic()) && !token.contains('@'))
        .count();
    has_contact_word && non_contact_tokens <= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_account_rule_triggers() {
        let features = FeatureSet {
            account_age_days: Some(2.0),
            ..Default::default()
        };
        let stats = ContributionStats {
            posts_last_24h: 4,
            dedupe_hits_last_48h: 0,
        };
        let engine = RuleEngine::default();
        let outcome = engine.evaluate(
            &features,
            RuleContext {
                body: "",
                stats: &stats,
                dedupe_hits_last_48h: 0,
            },
        );
        assert!(outcome.score > 0.0);
    }
}
