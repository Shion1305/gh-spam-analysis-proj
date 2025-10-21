use db::{CommentRow, IssueRow, UserRow};

use crate::features::{features_for_comment, features_for_issue, ContributionStats};
use crate::rules::{RuleContext, RuleEngine, RuleOutcome};

pub fn score_issue(
    issue: &IssueRow,
    user: Option<&UserRow>,
    stats: ContributionStats,
    dedupe_hits_last_48h: u32,
) -> RuleOutcome {
    let engine = RuleEngine::default();
    let features = features_for_issue(issue, user, stats.clone());
    engine.evaluate(
        &features,
        RuleContext {
            body: issue.body.as_deref().unwrap_or(""),
            stats: &stats,
            dedupe_hits_last_48h,
        },
    )
}

pub fn score_comment(
    comment: &CommentRow,
    user: Option<&UserRow>,
    stats: ContributionStats,
    dedupe_hits_last_48h: u32,
) -> RuleOutcome {
    let engine = RuleEngine::default();
    let features = features_for_comment(comment, user, stats.clone());
    engine.evaluate(
        &features,
        RuleContext {
            body: &comment.body,
            stats: &stats,
            dedupe_hits_last_48h,
        },
    )
}
