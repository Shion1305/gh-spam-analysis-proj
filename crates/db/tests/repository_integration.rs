use chrono::Utc;
use db::{pg::PgDatabase, Repositories, RepositoryRow};
use db_test_fixture::DbFixture;
use serde_json::json;

#[tokio::test]
async fn repository_upsert_roundtrip() -> anyhow::Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(fixture) => fixture,
        Err(err) => {
            eprintln!("skipping repository_upsert_roundtrip: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create_unmigrated("repo_upsert").await?;

    let database = PgDatabase::connect(handle.database_url()).await?;
    let repos = database.repos();

    let repo = RepositoryRow {
        id: 42,
        full_name: "owner/example".into(),
        is_fork: false,
        created_at: Utc::now(),
        pushed_at: None,
        raw: json!({"id": 42, "name": "example"}),
    };

    repos.upsert(repo.clone()).await?;
    let fetched = repos
        .get_by_full_name("owner/example")
        .await?
        .expect("repo fetched");
    assert_eq!(fetched.full_name, repo.full_name);
    assert_eq!(fetched.id, repo.id);

    drop(database);
    handle.cleanup().await?;
    Ok(())
}
