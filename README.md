# GitHub Spam Lab

Rust workspace for experimenting with spam detection on public GitHub activity.  
The project ingests issues/PRs/comments from a seed set of high-profile repositories, normalises them into Postgres, enriches user context, derives explainable features, and surfaces scoring results via a read-only API. Everything is built to be testable, observable, and extendable (e.g. to distributed workers later on).

---

## Objectives, Scope & Status

| Objective | Current State |
| --- | --- |
| Multi-token GitHub broker with priority queues, retries, caching, metrics | **Scaffolded** (`crates/gh_broker`) – core data structures, cache, metrics skeleton in place. |
| Data model & persistence layer with sqlx migrations | **Implemented** (`migrations/`, `crates/db`). |
| Normalisation utilities & dedupe hashing | **Drafted** (`crates/normalizer`). |
| Collector, analysis, API crates | **Stubs** (`crates/collector`, `crates/analysis`, `crates/api`) – wiring and tests still pending. |
| Test harnesses (unit, integration, wiremock, DB fixtures) | **Planned** (`crates/db_test_fixture` skeleton). |
| Tooling: Justfile, Aqua, Docker, Observability stack | **To Do** (tracked in roadmap). |

Non-goals for the MVP:
- Writing back to GitHub (strictly read-only).
- Fancy UI – JSON API + CLI/CSV exports are sufficient.
- ML training; we focus on explainable rule-based scoring with hooks for future ML models.

---

## Workspace Layout

```text
github-spam-lab/
├── Cargo.toml                 # Workspace definition & shared deps
├── README.md
├── crates/
│   ├── analysis/              # Feature extraction + rule scoring
│   ├── api/                   # Axum API server (handlers to be implemented)
│   ├── collector/             # Ingestion loop via GitHub broker
│   ├── common/                # Config, logging, error helpers
│   ├── db/                    # Postgres repositories & traits
│   ├── db_test_fixture/       # Temp database fixture for integration tests
│   ├── gh_broker/             # Multi-budget, multi-token broker
│   └── normalizer/            # Payload models → normalised DB rows
├── migrations/                # sqlx migrations (commit .sqlx metadata when generated)
└── scripts/                   # Seed data, helper scripts (pending)
```

---

## Quickstart

> **Note:** Tooling recipes (`just`, `aqua`, docker-compose) are not committed yet. The commands below outline the intended workflow once those artifacts land.

1. **Prerequisites**
   - Rust toolchain (`rustup`), `cargo fmt`, `cargo clippy`
   - Docker + docker-compose (for Postgres, Adminer, optional Redis/Kafka/obs stacks)
   - [Aqua](https://aquaproj.github.io/) for CLI dependency management
   - `just` command runner

2. **Clone & bootstrap**
   ```bash
   git clone https://github.com/<you>/github-spam-lab.git
   cd github-spam-lab
   cp .env.example .env
   just aqua-install            # installs just, sqlx-cli, nextest, redis-cli, kcat, promtool
   just up                      # docker compose up -d (Postgres + Adminer)
   just db-create               # create github_spam DB if missing
   just migrate                 # run sqlx migrations
   ```

3. **Run components**
   ```bash
   just dev-collector           # run collector against configured GitHub tokens
   just dev-api                 # start Axum API server on HTTP_BIND
   just obs-up                  # (optional) spin up Prometheus + Grafana stack
   ```

4. **Testing & linting**
   ```bash
   just fmt
   just lint                    # cargo fmt --check + cargo clippy -D warnings
   just test                    # cargo nextest run (unit + integration)
   ```

---

## Data Flow (Planned)

1. **Broker (`gh_broker`)**  
   - Accepts HTTP requests via trait interface, classifies them into budgets (`core`, `search`, `graphql`).
   - Manages per-budget priority queues and token pools with fairness + backoff.
   - Provides ETag caching & coalescing for GETs and emits Prometheus metrics.

2. **Collector (`collector`)**
   - Loads repositories from `scripts/seed_repos.json`.
   - Fetches issues (state=all, sorted by `updated`), uses watermarks to stop early.
   - Upserts repositories/issues/comments/users via `db` crate.
   - Memoizes user lookups and updates `collector_watermarks`.

3. **Normalizer (`normalizer`)**
   - Converts GitHub payloads to strongly typed rows (+ dedupe hashing strategy).
   - Ensures idempotence for repeated ingestion.

4. **Analysis (`analysis`)**
   - Computes feature vectors (length, URL count, entropy, account age, activity stats).
   - Applies rule engine (`rules_v1`) to assign spam scores and reasons.
   - Persists outcomes into `spam_flags` (versioned) for auditability.

5. **API (`api`)**
   - Axum-based read-only service exposing `/repos`, `/issues`, `/actors`, `/top/spammy-users`, `/healthz`, `/metrics`.
   - Depends on trait objects (repositories, broker client, etc.) for testability.

---

## Database Schema Highlights

- `repositories`, `users`, `issues`, `comments` tables mirror GitHub IDs and store raw JSONB blobs for reproducibility.
- `spam_flags` keeps versioned scores/reasons for issues/comments.
- `collector_watermarks` records per-repo `last_updated` for incremental fetches.
- Indexes: `dedupe_hash` on issues/comments, GIN full-text on bodies, queue-friendly indexes on `updated_at`, `repo_id`, etc.
- All migrations live in `migrations/` and are executed via `sqlx::migrate!()` (integration tests run against temp DBs created by `db_test_fixture` crate).

---

## Testing Strategy (Upcoming)

| Layer | Approach |
| --- | --- |
| Pure logic (normalizer, features, rules, broker math) | Table-driven unit tests. |
| API handlers | Tower + mockall trait mocks; snapshot JSON (insta) where useful. |
| Database repos | Integration tests using `db_test_fixture` (temp DB, migrations, cleanup). |
| Broker HTTP layer | Wiremock to simulate GitHub responses, rate limiting, Retry-After, ETag flows. |
| Collector end-to-end | Wiremock + temp DB to assert ingestion behaviour and request counts. |

`cargo nextest` is the default runner (`.config/nextest.toml` to be added with tuned parallelism/retries).

---

## Observability & Ops

- Prometheus metrics from broker & API (`/metrics`), including queue lengths, rate limits, retry counts, latency histograms.
- Docker compose stack under `docker/obs/` (pending) will bundle Prometheus + Grafana with a starter dashboard covering:
  - Per-budget remaining/limit per token
  - Queue lengths by priority
  - Request rate/error/retry counts
  - P95 latency
  - Cache hit ratio
- Logs: `tracing` with env-driven filters (`RUST_LOG`), structured JSON logging optional.

---

## Roadmap

- [ ] Finalise Justfile & Aqua configuration with pinned CLI tool versions.
- [ ] Complete GitHub client adapter + collector loop with tests.
- [ ] Add API routes, DTOs, and integration tests.
- [ ] Provide distributed-mode prototypes (Redis token buckets, Kafka work queues).
- [ ] Commit `.sqlx/` offline metadata and `.config/nextest.toml`.
- [ ] Fill in Grafana dashboard JSON + docker compose for obs/dist stacks.
- [ ] Harden rule engine (tunable thresholds, richer reasons, ML hooks).

Progress will be tracked through milestones; contributions welcome (see below).

---

## Contributing

1. Fork & clone the repository.
2. Keep commits small, use Gitmoji-prefixed messages (`git commit -m "✨ Add feature"`).
3. Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and the appropriate `just test-*` targets before opening a PR.
4. When touching DB queries, regenerate `.sqlx/` data if necessary (`SQLX_OFFLINE=true`).
5. For new endpoints or RuleEngine updates, add unit/integration coverage and update documentation.

---

## License

Dual-licensed under Apache 2.0 and MIT (see `LICENSE-{APACHE,MIT}` once added). You may contribute under either license.

