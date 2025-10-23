use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::model::{Budget, RateLimitUpdate};

#[derive(Debug, Clone)]
pub struct GithubToken {
    pub id: String,
    pub secret: String,
}

#[derive(Debug, Clone)]
pub struct RateLimitState {
    pub limit: i64,
    pub remaining: i64,
    pub reset_at: DateTime<Utc>,
}

impl Default for RateLimitState {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimitState {
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            limit: 5000,
            remaining: 5000,
            reset_at: now,
        }
    }

    pub fn update(&mut self, update: RateLimitUpdate) {
        self.limit = update.limit;
        self.remaining = update.remaining;
        self.reset_at = update.reset;
    }

    pub fn consume(&mut self, cost: i64) {
        self.remaining = (self.remaining - cost).max(0);
    }
}

#[derive(Debug)]
pub struct TokenState {
    pub token: GithubToken,
    pub budgets: HashMap<Budget, RateLimitState>,
}

impl TokenState {
    pub fn new(token: GithubToken) -> Self {
        let mut budgets = HashMap::new();
        budgets.insert(Budget::Core, RateLimitState::new());
        budgets.insert(Budget::Search, RateLimitState::new());
        budgets.insert(Budget::Graphql, RateLimitState::new());
        Self { token, budgets }
    }

    pub fn state_for(&mut self, budget: Budget) -> &mut RateLimitState {
        self.budgets.entry(budget).or_default()
    }
}

#[derive(Clone)]
pub struct TokenPool {
    inner: Arc<Mutex<Vec<TokenState>>>,
}

pub enum TokenSelection {
    Token(GithubToken),
    Wait(std::time::Duration),
}

impl TokenPool {
    pub fn new(tokens: Vec<GithubToken>) -> Self {
        let states = tokens.into_iter().map(TokenState::new).collect();
        Self {
            inner: Arc::new(Mutex::new(states)),
        }
    }

    pub async fn pick_token(&self, budget: Budget) -> TokenSelection {
        let mut guard = self.inner.lock().await;
        let now = Utc::now();
        let mut best = None;
        let mut next_reset = None;

        for state in guard.iter_mut() {
            let rl = state.state_for(budget);
            if rl.remaining > 0 || rl.reset_at <= now {
                let score = rl.remaining as f64 / rl.limit.max(1) as f64;
                match best {
                    None => best = Some((score, state.token.clone())),
                    Some((best_score, _)) if score > best_score => {
                        best = Some((score, state.token.clone()))
                    }
                    _ => {}
                }
            } else {
                let wait = rl.reset_at - now;
                let wait = wait.to_std().unwrap_or_default();
                next_reset = match next_reset {
                    None => Some(wait),
                    Some(existing) => Some(existing.min(wait)),
                };
            }
        }

        if let Some((_, token)) = best {
            TokenSelection::Token(token)
        } else if let Some(wait) = next_reset {
            TokenSelection::Wait(wait)
        } else {
            // No tokens configured
            TokenSelection::Wait(std::time::Duration::from_secs(30))
        }
    }

    pub async fn update(&self, budget: Budget, token_id: &str, update: RateLimitUpdate) {
        let mut guard = self.inner.lock().await;
        for token in guard.iter_mut() {
            if token.token.id == token_id {
                token.state_for(budget).update(update);
                break;
            }
        }
    }

    pub async fn consume(&self, budget: Budget, token_id: &str, amount: i64) {
        let mut guard = self.inner.lock().await;
        for token in guard.iter_mut() {
            if token.token.id == token_id {
                token.state_for(budget).consume(amount);
                break;
            }
        }
    }

    pub async fn get_numbers(&self, budget: Budget, token_id: &str) -> Option<(i64, i64)> {
        let guard = self.inner.lock().await;
        for token in guard.iter() {
            if token.token.id == token_id {
                let st = token.budgets.get(&budget)?;
                return Some((st.limit, st.remaining));
            }
        }
        None
    }

    pub async fn totals(&self, budget: Budget) -> (i64, i64) {
        let guard = self.inner.lock().await;
        let mut limit_sum: i64 = 0;
        let mut remaining_sum: i64 = 0;
        for token in guard.iter() {
            if let Some(state) = token.budgets.get(&budget) {
                limit_sum += state.limit;
                remaining_sum += state.remaining;
            }
        }
        (limit_sum, remaining_sum)
    }
}
