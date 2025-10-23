# GitHub Spam Lab

Rust workspace for experimenting with spam detection on public GitHub activity.  
The project ingests issues/PRs/comments from a seed set of high-profile repositories, normalises them into Postgres, enriches user context, derives explainable features, and surfaces scoring results via a read-only API. Everything is built to be testable, observable, and extendable (e.g. to distributed workers later on).

---

## Objectives, Scope & Status

| Objective | Current State |
| --- | --- |
| Multi-token GitHub broker with priority queues, retries, caching, metrics | **Implemented** (`crates/gh_broker`) – production-ready builder with configurable weights/bounds, token rotation, caching, Prometheus metrics. |
| Data model & persistence layer with sqlx migrations | **Implemented** (`migrations/`, `crates/db`). |
| Normalisation utilities & dedupe hashing | **Implemented** (`crates/normalizer`). |
| Collector, analysis, API crates | **Implemented** (`crates/collector`, `crates/analysis`, `crates/api`) – ingestion loop, rule-based scoring primitives, Axum API. |
| Test harnesses (unit, integration, wiremock, DB fixtures) | **In progress** (`crates/db_test_fixture` ready; additional suites planned). |
| Tooling: Justfile, Aqua, Docker, Observability stack | **Implemented** (`Justfile`, `aqua.yaml`, `docker/`). |

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
└── scripts/                   # Seed data & helpers
```

---

## Quickstart

1. **Prerequisites**
   - Rust toolchain (`rustup`), `cargo fmt`, `cargo clippy`
   - Docker + docker-compose (for Postgres, Adminer, optional Redis/Kafka/obs stacks)
   - [Aqua](https://aquaproj.github.io/) for CLI dependency management
   - `just` command runner

2. **Clone & bootstrap**
   ```bash
   git clone https://github.com/<you>/github-spam-lab.git
   cd github-spam-lab
   # Create .env file with database URLs (used by the binaries & tests)
   {
     echo 'DATABASE_URL=postgres://postgres:postgres@localhost:5432/github_spam'
     echo 'TEST_ADMIN_URL=postgres://postgres:postgres@localhost:5432/postgres'
   } > .env
   just aqua-install            # installs just, sqlx-cli, nextest, redis-cli, kcat, promtool
   just up                      # docker compose up -d (auto-copies docker/.env.example on first run)

   # Postgres is seeded automatically (POSTGRES_DB), and the Rust apps run migrations on start-up.
   # No manual `just migrate` is required for local development.
   ```

   Configuration is loaded from `config/default.toml` or `config/local.toml` files, or via environment variables with `__` separator (e.g., `DATABASE__URL`, `API__BIND`).
   The collector no longer uses `seed_repos_path`; repositories are enqueued via collection jobs.

3. **Run components**
   ```bash
   # Configure GitHub tokens in config/local.toml or via environment:
   # export GITHUB__TOKEN_IDS="token1"
   # export GITHUB__TOKEN_SECRETS="ghp_xxxxx"
   just dev-collector           # collector connects to Postgres, runs migrations, and starts ingesting
   just dev-api                 # API auto-runs migrations before serving (default bind: 0.0.0.0:3000)
   just obs-up                  # (optional) spin up Prometheus + Grafana stack

   # Dockerfile + docker-compose
   # ---------------------------
   # `docker/docker-compose.yml` now builds the workspace image (see root Dockerfile) and starts:
   #   - Postgres 17 (with health checks)
   #   - Adminer (http://localhost:8081)
   #   - API service (http://localhost:3000)
   #   - Collector service
   # The collector requires GitHub REST tokens (`GITHUB__TOKEN_IDS` / `GITHUB__TOKEN_SECRETS`).
   # By default, requests use a generic, privacy-friendly User-Agent ("generic-http-client").
   # You can override it (optionally) via env: `GITHUB__USER_AGENT="your-app/1.0"`.
   # Edit docker/.env (auto-created from docker/.env.example) before running `just up`
   # or adjust the compose environment to disable the collector if you only need the API locally.
   ```

4. **Testing & linting**
   ```bash
   just format                  # cargo fmt
   just check                   # cargo fmt --check + cargo clippy -D warnings (uses SQLX_OFFLINE)
   just test                    # cargo nextest run (spins up a test Postgres container and auto-stops it)
   just test-unit               # cargo nextest run --lib
   just test-integration        # cargo nextest run --tests
   ```

---

## Data Flow (Planned)

1. **Broker (`gh_broker`)**  
   - Accepts HTTP requests via trait interface, classifies them into budgets (`core`, `search`, `graphql`).
   - Manages per-budget priority queues and token pools with fairness + backoff.
   - Provides ETag caching & coalescing for GETs and emits Prometheus metrics.

2. **Collector (`collector`)**
   - Seeds work from the `collection_jobs` table (create via API `POST /repos`).
   - Fetch modes (`collector.fetch_mode`): `rest` | `graphql` | `hybrid` (default: `hybrid`).
     - Hybrid: uses GraphQL for repositories, issues, and comments; falls back to REST for users that aren’t present in GraphQL responses.
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
   - Axum-based service exposing `/repos` (POST to create jobs, GET to list), `/issues`, `/actors`, `/top/spammy-users`, `/healthz`, `/metrics`.
   - Depends on trait objects (repositories, broker client, etc.) for testability.

---

## Database & Schema Management

- `repositories`, `users`, `issues`, `comments` tables mirror GitHub IDs and store raw JSONB blobs for reproducibility.
- `spam_flags` keeps versioned scores/reasons for issues/comments.
- `collector_watermarks` records per-repo `last_updated` for incremental fetches.
- Indexes: `dedupe_hash` on issues/comments, GIN full-text on bodies, queue-friendly indexes on `updated_at`, `repo_id`, etc.
- All migrations live in `migrations/` and are executed by the binaries on startup via `sqlx::migrate!()`; no manual intervention is required. Integration tests use `db_test_fixture` to provision isolated databases and apply migrations automatically.

### Local Workflow Tips

- Stop/start Postgres quickly with `just down` / `just up`. Compose seeds a fresh `github_spam` database whenever the container is recreated.
- For CI/Lint pipelines we run in SQLx offline mode (`SQLX_OFFLINE=true`); regenerate `.sqlx/` metadata with `cargo sqlx prepare --workspace -- --all-targets` whenever queries change.

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

- Prometheus metrics from broker, collector, and API (`/metrics`). Highlights:
  - Broker per-token and aggregated capacities by budget (REST/Core vs GraphQL):
    - `gh_broker_rate_limit{token,budget}`, `gh_broker_rate_remaining{token,budget}`
    - `gh_broker_budget_limit_total{budget}`, `gh_broker_budget_remaining_total{budget}`
  - Fetcher metrics split by backend and operation:
    - `collector_fetch_requests_total{fetcher,op,outcome}`
    - `collector_fetch_items_total{fetcher,op}`
    - `collector_fetch_latency_seconds_bucket{fetcher,op}`
  - Collector run/job gauges and histograms (runs, in-progress repos, last success/attempt, P95 repo duration, throughput).
- Docker compose stack under `docker/obs/` bundles Prometheus + Grafana with a dashboard covering:
  - REST vs GraphQL budget remaining and utilization
  - Queue lengths by priority; request rate/error/retries; P95 latency; cache hit ratio
  - Issues/sec, Comments/sec; Jobs status (pending/in_progress/completed/failed/error)
- Logs: `tracing` with env-driven filters (`RUST_LOG`), structured JSON logging optional.

## Continuous Integration & Release

- `.github/workflows/ci.yml` runs fmt, clippy (`-D warnings`), and `cargo nextest` against a Postgres service launched via Docker Compose. The workflow relies on the application's built-in migrations, so schema drift is caught automatically. The workflow uses `dtolnay/rust-toolchain@stable` and `Swatinem/rust-cache@v2`.
- `.github/workflows/helm-publish.yml` packages `charts/github-spam-lab` and publishes an OCI chart to `ghcr.io/<owner>/charts`. The same workflow attaches the `.tgz` as a build artifact for manual download.
- Set `DATABASE_URL`/`TEST_ADMIN_URL` secrets in self-hosted runners if you customize database credentials.

### Installing from GHCR

```bash
helm upgrade --install github-spam-lab \
  oci://ghcr.io/shion1305/charts/github-spam-lab \
  --version 0.1.0 \
  --set image.repository=ghcr.io/shion1305/gh-spam-analysis-proj \
  --namespace github-spam --create-namespace
```

Override `GITHUB__TOKEN_IDS`/`GITHUB__TOKEN_SECRETS` and database settings via `--set` or a custom values file before deploying in a real cluster.

---

## Roadmap

- [x] Generate and commit `.sqlx/` offline metadata for prepared statements.
- [x] All clippy warnings fixed (compiles with `-D warnings`).
- [x] Configuration system implemented (TOML + environment variables).
- [x] Core infrastructure complete (broker, collector, normalizer, analysis, API).
- [ ] Expand unit coverage for broker scheduling, collector loops, and API handlers.
- [ ] Add integration suites leveraging `db_test_fixture` and wiremock for HTTP scenarios.
- [ ] Provide distributed-mode prototypes (Redis/Kafka) wired into the broker queue interfaces.
- [ ] Harden rule engine (tunable thresholds, richer heuristics, ML hooks).

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
