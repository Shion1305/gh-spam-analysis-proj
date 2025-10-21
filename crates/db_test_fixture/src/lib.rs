use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn create_temp_db(admin_url: &str, prefix: &str) -> Result<(PgPool, String)> {
    let db_name = format!("{}_{}", prefix, Uuid::new_v4().to_simple());
    let create_sql = format!(r#"CREATE DATABASE "{}""#, db_name);
    let admin_pool = PgPool::connect(admin_url).await?;
    sqlx::query(&create_sql).execute(&admin_pool).await?;
    let db_url = format!("{}/{}", admin_url, db_name);
    let pool = PgPool::connect(&db_url).await?;
    Ok((pool, db_name))
}
