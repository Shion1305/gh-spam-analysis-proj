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
	cargo nextest run

test-unit:
	cargo nextest run --workspace --lib

test-integration:
	cargo nextest run --workspace --tests

# Apps

dev-collector:
	cargo run -p collector

dev-api:
	cargo run -p api

ingest-once:
	COLLECTOR_RUN_ONCE=true cargo run -p collector
