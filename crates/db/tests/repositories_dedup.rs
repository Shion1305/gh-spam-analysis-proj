use chrono::Utc;
use db::{pg::PgDatabase, Repositories, RepositoryRow};
use db_test_fixture::DbFixture;
use serde_json::json;
use sqlx::{query, query_scalar, PgPool};

#[tokio::test]
async fn repos_upsert_is_idempotent_by_id() -> anyhow::Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping repos_upsert_is_idempotent_by_id: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("repos_dedup").await?;
    let db = PgDatabase::connect(handle.database_url()).await?;
    let repos = db.repos();

    let repo = RepositoryRow {
        id: 1001,
        full_name: "Owner/Example".into(),
        is_fork: false,
        created_at: Utc::now(),
        pushed_at: None,
        raw: json!({"id": 1001, "name": "Example"}),
    };

    repos.upsert(repo.clone()).await?;
    // Upsert again with a changed pushed_at and raw
    let mut second = repo.clone();
    second.pushed_at = repo.created_at.into();
    second.raw = json!({"id": 1001, "name": "Example", "run": 2});
    repos.upsert(second).await?;

    let pool: &PgPool = db.pool();
    let count: i64 = query_scalar("SELECT COUNT(*) FROM repositories WHERE id = $1")
        .bind(1001_i64)
        .fetch_one(pool)
        .await?;
    assert_eq!(count, 1, "exactly one repo row after idempotent upsert");

    handle.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn repos_full_name_is_case_insensitive_unique() -> anyhow::Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping repos_full_name_is_case_insensitive_unique: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("repos_ci").await?;
    let db = PgDatabase::connect(handle.database_url()).await?;
    let pool: &PgPool = db.pool();

    // Insert first row
    query(
        r#"
        INSERT INTO repositories (id, full_name, is_fork, created_at, pushed_at, raw)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(2001_i64)
    .bind("Owner/Example")
    .bind(false)
    .bind(Utc::now())
    .bind::<Option<chrono::DateTime<Utc>>>(None)
    .bind(json!({"id": 2001}))
    .execute(pool)
    .await?;

    // Inserting with different case should violate the CI unique index
    let err = query(
        r#"
        INSERT INTO repositories (id, full_name, is_fork, created_at, pushed_at, raw)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(2002_i64)
    .bind("owner/example")
    .bind(false)
    .bind(Utc::now())
    .bind::<Option<chrono::DateTime<Utc>>>(None)
    .bind(json!({"id": 2002}))
    .execute(pool)
    .await
    .expect_err("expected case-insensitive uniqueness violation");

    // Make sure it's a unique-violation style database error
    if let sqlx::Error::Database(db_err) = &err {
        assert!(db_err.message().to_lowercase().contains("unique"));
    } else {
        panic!("unexpected error kind: {err}");
    }

    handle.cleanup().await?;
    Ok(())
}

