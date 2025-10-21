use std::env;

use anyhow::{Context, Result};
use db::pg::run_migrations;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

pub struct DbFixture {
    admin_url: String,
}

impl DbFixture {
    pub fn from_env() -> Result<Self> {
        let admin_url = env::var("TEST_ADMIN_URL")
            .or_else(|_| env::var("DATABASE_URL"))
            .context("TEST_ADMIN_URL or DATABASE_URL must be set for tests")?;
        Ok(Self { admin_url })
    }

    pub async fn create(&self, prefix: &str) -> Result<DatabaseHandle> {
        let db_name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        let admin_pool = PgPool::connect(&self.admin_url).await?;
        let create_sql = format!("CREATE DATABASE \"{}\"", db_name);
        admin_pool.execute(create_sql.as_str()).await?;
        let db_url = format!("{}/{}", self.admin_url, db_name);
        let pool = PgPool::connect(&db_url).await?;
        run_migrations(&pool).await?;
        Ok(DatabaseHandle {
            pool,
            name: db_name,
            admin_url: self.admin_url.clone(),
        })
    }
}

pub struct DatabaseHandle {
    pool: PgPool,
    name: String,
    admin_url: String,
}

impl DatabaseHandle {
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn into_pool(self) -> PgPool {
        self.pool
    }

    pub async fn cleanup(self) -> Result<()> {
        drop(self.pool);
        let admin_pool = PgPool::connect(&self.admin_url).await?;
        let terminate_sql = format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
            self.name
        );
        admin_pool.execute(terminate_sql.as_str()).await?;
        let drop_sql = format!("DROP DATABASE IF EXISTS \"{}\"", self.name);
        admin_pool.execute(drop_sql.as_str()).await?;
        Ok(())
    }
}
