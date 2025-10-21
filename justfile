set dotenv-load := true

database_url := env_var_or_default('DATABASE_URL', 'postgres://postgres:postgres@localhost:5432/github_spam')

alias fmt := format
alias lint := check

# Tooling

default:
  @echo "Available tasks:"
  @just --list

aqua-install:
	aqua i -l

up:
	docker compose up -d

down:
	docker compose down -v

obs-up:
	docker compose -f docker/obs/docker-compose.yml up -d

obs-down:
	docker compose -f docker/obs/docker-compose.yml down -v

dist-up:
	docker compose -f docker/dist/docker-compose.yml up -d

dist-down:
	docker compose -f docker/dist/docker-compose.yml down -v

# Database

db-create:
	psql '{{ database_url }}' -c 'select 1;' 2>/dev/null || \
	psql '{{ replace(database_url, "/github_spam", "/postgres") }}' -c 'create database github_spam;'

migrate:
	sqlx migrate run

migrate-revert:
	sqlx migrate revert -y

migrate-add name:
	sqlx migrate add -r {{name}}

# Code quality

format:
	cargo fmt

check:
	cargo fmt -- --check && cargo clippy --all-targets -- -D warnings

# Tests

test:
	if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace --exclude db --exclude db_test_fixture --exclude collector --exclude analysis --exclude api; \
	else \
		cargo test --workspace --exclude db --exclude db_test_fixture --exclude collector --exclude analysis --exclude api; \
	fi

test-unit:
	if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace --lib; \
	else \
		cargo test --lib --workspace; \
	fi

test-integration:
	if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace --tests; \
	else \
		cargo test --tests --workspace; \
	fi

# Apps

dev-collector:
	cargo run -p collector

dev-api:
	cargo run -p api

ingest-once:
	COLLECTOR_RUN_ONCE=true cargo run -p collector
