set dotenv-load := true

database_url := env_var_or_default('DATABASE_URL', 'postgres://postgres:postgres@localhost:5432/github_spam')

test_db_name := env_var_or_default('TEST_DB_CONTAINER', 'gh-spam-test-db')
test_db_port := env_var_or_default('TEST_DB_PORT', '55432')
test_db_user := env_var_or_default('TEST_DB_USER', 'postgres')
test_db_password := env_var_or_default('TEST_DB_PASSWORD', 'postgres')
test_admin_url := 'postgres://' + test_db_user + ':' + test_db_password + '@localhost:' + test_db_port + '/postgres'
test_database_url := 'postgres://' + test_db_user + ':' + test_db_password + '@localhost:' + test_db_port + '/github_spam'

alias fmt := format
alias lint := check

# Tooling

default:
  @echo "Available tasks:"
  @just --list

aqua-install:
	aqua i -l

ensure-docker-env:
	@if [ ! -f docker/.env ]; then \
		cp docker/.env.example docker/.env; \
	fi

up: ensure-docker-env
	docker compose -f docker/docker-compose.yml --project-directory docker up -d --build

down:
	docker compose -f docker/docker-compose.yml --project-directory docker down -v

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

test-db-up:
	@if docker inspect -f '{{"{{"}}.State.Running{{"}}"}}' {{test_db_name}} 2>/dev/null | grep -q 'true'; then \
		echo "Test database container '{{test_db_name}}' already running."; \
	else \
		if docker inspect {{test_db_name}} >/dev/null 2>&1; then \
			docker rm -f {{test_db_name}} >/dev/null; \
		fi; \
		docker run -d --rm --name {{test_db_name}} \
			-e POSTGRES_USER={{test_db_user}} \
			-e POSTGRES_PASSWORD={{test_db_password}} \
			-p {{test_db_port}}:5432 \
			postgres:17-alpine >/dev/null || { echo "Failed to start test Postgres container"; exit 1; }; \
		echo "Waiting for test Postgres to become ready..."; \
		until docker exec {{test_db_name}} pg_isready -U {{test_db_user}} >/dev/null 2>&1; do \
			sleep 1; \
		done; \
		if ! docker exec {{test_db_name}} psql -U {{test_db_user}} -tc "SELECT 1 FROM pg_database WHERE datname = 'github_spam'" | grep -q 1; then \
			docker exec {{test_db_name}} createdb -U {{test_db_user}} github_spam; \
		fi; \
	fi

test-db-down:
	@if docker ps --format '{{"{{"}}.Names{{"}}"}}' | grep -q '^{{test_db_name}}$$'; then \
		docker stop {{test_db_name}} >/dev/null; \
	elif docker ps -a --format '{{"{{"}}.Names{{"}}"}}' | grep -q '^{{test_db_name}}$$'; then \
		docker rm -f {{test_db_name}} >/dev/null; \
	fi

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

test: test-db-up
    trap 'docker stop {{test_db_name}} >/dev/null 2>&1 || docker rm -f {{test_db_name}} >/dev/null 2>&1 || true' EXIT; \
    if command -v cargo-nextest >/dev/null 2>&1; then \
        TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo nextest run --workspace; \
    else \
        TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo test --workspace; \
    fi

test-unit: test-db-up
	if command -v cargo-nextest >/dev/null 2>&1; then \
		TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo nextest run --workspace --lib; \
	else \
		TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo test --lib --workspace; \
	fi

test-integration: test-db-up
	if command -v cargo-nextest >/dev/null 2>&1; then \
		TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo nextest run --workspace --tests; \
	else \
		TEST_ADMIN_URL='{{test_admin_url}}' DATABASE_URL='{{test_database_url}}' cargo test --tests --workspace; \
	fi

# Apps

dev-collector:
	cargo run -p collector

dev-api:
	cargo run -p api

ingest-once:
	COLLECTOR_RUN_ONCE=true cargo run -p collector
