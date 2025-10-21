use common::text::dedupe_hash;
use serde_json::Value;

use crate::models::{NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser};
use crate::payloads::{CommentPayload, IssuePayload, RepoPayload, UserPayload};

pub fn normalize_repo(payload: &RepoPayload, raw: Value) -> NormalizedRepository {
    NormalizedRepository {
        id: payload.id,
        full_name: payload.full_name.clone(),
        is_fork: payload.fork,
        created_at: payload.created_at,
        pushed_at: payload.pushed_at,
        raw,
    }
}

pub fn normalize_user(payload: &UserPayload, raw: Value) -> NormalizedUser {
    NormalizedUser {
        id: payload.id,
        login: payload.login.clone(),
        user_type: payload.user_type.clone(),
        site_admin: payload.site_admin,
        created_at: payload.created_at,
        followers: payload.followers,
        following: payload.following,
        public_repos: payload.public_repos,
        raw,
    }
}

pub fn normalize_issue(payload: &IssuePayload, repo_id: i64, raw: Value) -> NormalizedIssue {
    let body = payload.body.clone();
    NormalizedIssue {
        id: payload.id,
        repo_id,
        number: payload.number,
        is_pull_request: payload.pull_request.is_some(),
        state: payload.state.clone(),
        title: payload.title.clone(),
        body: body.clone(),
        user_id: payload.user.as_ref().map(|u| u.id),
        comments_count: payload.comments,
        created_at: payload.created_at,
        updated_at: payload.updated_at,
        closed_at: payload.closed_at,
        dedupe_hash: dedupe_hash(&payload.title, &body.unwrap_or_default()),
        raw,
    }
}

pub fn normalize_comment(payload: &CommentPayload, issue_id: i64, raw: Value) -> NormalizedComment {
    let body = payload.body.clone();
    NormalizedComment {
        id: payload.id,
        issue_id,
        user_id: payload.user.as_ref().map(|u| u.id),
        body: body.clone(),
        created_at: payload.created_at,
        updated_at: payload.updated_at,
        dedupe_hash: dedupe_hash("", &body),
        raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn issue_normalization_hashes_body() {
        let payload = IssuePayload {
            id: 1,
            number: 10,
            pull_request: None,
            state: "open".into(),
            title: "Spam".into(),
            body: Some("Buy now!!!".into()),
            user: None,
            comments: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_at: None,
        };
        let normalized = normalize_issue(&payload, 42, json!({}));
        assert_eq!(normalized.repo_id, 42);
        assert!(normalized.dedupe_hash.len() == 64);
    }

    #[test]
    fn comment_normalization_uses_issue() {
        let payload = CommentPayload {
            id: 1,
            user: None,
            body: "Hi there".into(),
            created_at: Utc::now(),
            updated_at: None,
        };
        let normalized = normalize_comment(&payload, 55, json!({}));
        assert_eq!(normalized.issue_id, 55);
        assert!(normalized.dedupe_hash.len() == 64);
    }
}
