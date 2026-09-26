//! Disposable MySQL protocol regression; no billing, provider or device calls.
use anycode_account_service::{db::AccountDb, store};
use sqlx::Row;
use uuid::Uuid;
#[tokio::test]
#[ignore = "requires run-owned loopback MySQL anycode_platform_test"]
async fn code_is_browser_bound_expires_and_is_consumed_once() {
    assert_eq!(std::env::var("ALLOW_ISOLATED_TESTS").as_deref(), Ok("1"));
    let url = std::env::var("ANYCODE_TEST_DATABASE_URL").expect("disposable MySQL required");
    let options: sqlx::mysql::MySqlConnectOptions = url.parse().unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    assert_eq!(options.get_database(), Some("anycode_platform_test"));
    let db = AccountDb::connect(&url).await.unwrap();
    // Minimal synthetic identity/session schema; this is not full migration acceptance.
    sqlx::query("CREATE TABLE IF NOT EXISTS users(id VARCHAR(64) PRIMARY KEY,organization_id VARCHAR(64),email VARCHAR(255),display_name VARCHAR(255),role VARCHAR(32),status VARCHAR(32),lingxi_user_id VARCHAR(64)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci").execute(db.pool()).await.unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS sessions(id VARCHAR(64) PRIMARY KEY,user_id VARCHAR(64),token_hash VARCHAR(128),expires_at DATETIME(3))").execute(db.pool()).await.unwrap();
    sqlx::query(include_str!("../migrations/032_portal_login_codes.sql"))
        .execute(db.pool())
        .await
        .unwrap();
    let uid = format!("test_{}", Uuid::new_v4());
    sqlx::query("INSERT INTO users(id,organization_id,email,display_name,role,status) VALUES(?,?,?,'Test','owner','active')").bind(&uid).bind("test-org").bind(format!("{uid}@example.test")).execute(db.pool()).await.unwrap();
    let code = store::create_portal_login_code(&db, &uid, "browser-one")
        .await
        .unwrap();
    assert!(
        store::consume_portal_login_code(&db, &code, "other-browser")
            .await
            .unwrap()
            .is_none()
    );
    let (first, second) = tokio::join!(
        store::consume_portal_login_code(&db, &code, "browser-one"),
        store::consume_portal_login_code(&db, &code, "browser-one")
    );
    assert_eq!(
        usize::from(first.unwrap().is_some()) + usize::from(second.unwrap().is_some()),
        1
    );
    assert!(store::consume_portal_login_code(&db, &code, "browser-one")
        .await
        .unwrap()
        .is_none());
    let expired = store::create_portal_login_code(&db, &uid, "browser-one")
        .await
        .unwrap();
    sqlx::query("UPDATE portal_login_codes SET expires_at=DATE_SUB(NOW(),INTERVAL 1 SECOND) WHERE user_id=?").bind(&uid).execute(db.pool()).await.unwrap();
    assert!(
        store::consume_portal_login_code(&db, &expired, "browser-one")
            .await
            .unwrap()
            .is_none()
    );
    let row = sqlx::query("SELECT COUNT(*) AS n FROM sessions WHERE user_id=?")
        .bind(&uid)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("n"), 1);
    let conflict = store::resolve_lingxi_user(
        &db,
        "different-platform-id",
        Some(&format!("{uid}@example.test")),
        None,
    )
    .await
    .unwrap_err();
    assert!(conflict.is::<store::IdentityLinkRequired>());
    sqlx::query("DELETE FROM sessions WHERE user_id=?")
        .bind(&uid)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id=?")
        .bind(&uid)
        .execute(db.pool())
        .await
        .unwrap();
}
