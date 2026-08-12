//! Local dashboard user session (UI login, distinct from API Bearer tokens).

use crate::db::DashboardDb;
use crate::schema::{LOCAL_ORG_ID, LOCAL_USER_ID};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub organization_id: String,
    pub auth_method: String,
}

#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionStore {
    pub fn create(&self, user_id: &str) -> String {
        let token = format!("sess_{}", Uuid::new_v4());
        self.inner
            .write()
            .unwrap()
            .insert(token.clone(), user_id.to_string());
        token
    }

    pub fn resolve(&self, token: &str) -> Option<String> {
        self.inner.read().unwrap().get(token).cloned()
    }

    pub fn revoke(&self, token: &str) {
        self.inner.write().unwrap().remove(token);
    }
}

pub async fn get_user_by_id(db: &DashboardDb, user_id: &str) -> Result<Option<AuthUser>> {
    let row = sqlx::query(
        r#"
        SELECT id, organization_id, email, display_name, role
        FROM users WHERE id = ?
        "#,
    )
    .bind(user_id)
    .fetch_optional(db.pool())
    .await?;
    Ok(row.map(|r| AuthUser {
        id: r.get("id"),
        email: r.get("email"),
        display_name: r.get("display_name"),
        role: r.get("role"),
        organization_id: r.get("organization_id"),
        auth_method: "session".into(),
    }))
}

pub async fn local_trusted_user(db: &DashboardDb) -> Result<AuthUser> {
    get_user_by_id(db, LOCAL_USER_ID)
        .await?
        .ok_or_else(|| anyhow::anyhow!("local user missing"))
        .map(|mut u| {
            u.auth_method = "local_trusted".into();
            u
        })
}

/// Outcome of a username/password login attempt.
#[derive(Debug)]
pub enum LoginOutcome {
    Success(AuthUser),
    InvalidCredentials,
    /// The account has no password configured, but the caller requires one
    /// (dashboard bound to a non-loopback address). Refused fail-closed
    /// instead of accepting any password (WEB-11).
    PasswordNotConfigured,
}

/// `password_required` must be true when the dashboard is reachable beyond
/// loopback (LAN bind). On a loopback-only bind the seeded local user
/// legitimately has no password, so passwordless login stays allowed there.
pub async fn login(
    db: &DashboardDb,
    email: &str,
    password: &str,
    password_required: bool,
) -> Result<LoginOutcome> {
    let row = sqlx::query(
        r#"
        SELECT id, organization_id, email, display_name, role, password_hash
        FROM users
        WHERE organization_id = ? AND email = ?
        "#,
    )
    .bind(LOCAL_ORG_ID)
    .bind(email.trim())
    .fetch_optional(db.pool())
    .await?;
    let Some(r) = row else {
        return Ok(LoginOutcome::InvalidCredentials);
    };
    let hash: Option<String> = r.get("password_hash");
    match hash.filter(|s| !s.trim().is_empty()) {
        Some(h) => {
            if !verify_password(password, &h) {
                return Ok(LoginOutcome::InvalidCredentials);
            }
        }
        None if password_required => {
            // Fail-closed: an account without a configured password must not
            // accept arbitrary credentials on a non-loopback bind.
            return Ok(LoginOutcome::PasswordNotConfigured);
        }
        None => {
            // Loopback-only local workbench: keep the historical passwordless
            // login for the seeded local user.
        }
    }
    Ok(LoginOutcome::Success(AuthUser {
        id: r.get("id"),
        email: r.get("email"),
        display_name: r.get("display_name"),
        role: r.get("role"),
        organization_id: r.get("organization_id"),
        auth_method: "session".into(),
    }))
}

fn verify_password(password: &str, hash: &str) -> bool {
    let hash = hash.trim();
    if hash.is_empty() {
        // Fail-closed: an empty stored hash never matches. Callers that allow
        // passwordless loopback login must bypass verification explicitly.
        return false;
    }

    if let Some((salt, expected)) = parse_sha256_password_hash(hash) {
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(b":");
        hasher.update(password.as_bytes());
        let actual = hex_lower(&hasher.finalize());
        return constant_time_eq(actual.as_bytes(), expected.as_bytes());
    }

    // Backward compatibility for existing local databases created before
    // password hashes were supported. New configured passwords should use
    // `sha256$<salt>$<hex_sha256(salt:password)>`.
    constant_time_eq(password.as_bytes(), hash.as_bytes())
}

pub const SESSION_COOKIE: &str = "dw_session";

fn parse_sha256_password_hash(hash: &str) -> Option<(&str, &str)> {
    let mut parts = hash.split('$');
    let scheme = parts.next()?;
    let salt = parts.next()?;
    let expected = parts.next()?;
    if parts.next().is_some()
        || scheme != "sha256"
        || salt.is_empty()
        || expected.len() != 64
        || !expected.as_bytes().iter().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some((salt, expected))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max_len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        diff |= (av ^ bv) as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{hex_lower, login, verify_password, LoginOutcome};
    use crate::db::DashboardDb;
    use sha2::{Digest, Sha256};

    fn sha256_password_hash(salt: &str, password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(b":");
        hasher.update(password.as_bytes());
        format!("sha256${salt}${}", hex_lower(&hasher.finalize()))
    }

    #[test]
    fn verifies_sha256_password_hashes() {
        let hash = sha256_password_hash("tenant-a", "correct horse");
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong horse", &hash));
    }

    #[test]
    fn keeps_legacy_plaintext_compatibility() {
        assert!(verify_password("old-password", "old-password"));
        assert!(!verify_password("old-password", "other-password"));
    }

    #[test]
    fn rejects_malformed_sha256_hashes() {
        assert!(!verify_password("pw", "sha256$salt$not-hex"));
        assert!(!verify_password("pw", "sha256$$0123456789abcdef"));
    }

    #[test]
    fn rejects_empty_hash_fail_closed() {
        assert!(!verify_password("anything", ""));
        assert!(!verify_password("", "   "));
    }

    async fn open_temp_db() -> (tempfile::TempDir, DashboardDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("test.db")).await.unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn seeded_user_without_password_login_allowed_on_loopback_only() {
        let (_dir, db) = open_temp_db().await;
        // Loopback-only workbench: historical passwordless login still works.
        let outcome = login(&db, "local@anycode", "any-password", false)
            .await
            .unwrap();
        assert!(matches!(outcome, LoginOutcome::Success(_)));
        // LAN bind: same account must be refused and told to set a password.
        let outcome = login(&db, "local@anycode", "any-password", true)
            .await
            .unwrap();
        assert!(matches!(outcome, LoginOutcome::PasswordNotConfigured));
        let outcome = login(&db, "local@anycode", "", true).await.unwrap();
        assert!(matches!(outcome, LoginOutcome::PasswordNotConfigured));
    }

    #[tokio::test]
    async fn configured_password_enforced_on_lan() {
        let (_dir, db) = open_temp_db().await;
        let hash = sha256_password_hash("local", "s3cret");
        sqlx::query("UPDATE users SET password_hash = ? WHERE email = 'local@anycode'")
            .bind(&hash)
            .execute(db.pool())
            .await
            .unwrap();
        let wrong = login(&db, "local@anycode", "wrong", true).await.unwrap();
        assert!(matches!(wrong, LoginOutcome::InvalidCredentials));
        let right = login(&db, "local@anycode", "s3cret", true).await.unwrap();
        assert!(matches!(right, LoginOutcome::Success(_)));
        let unknown = login(&db, "nobody@anycode", "s3cret", true).await.unwrap();
        assert!(matches!(unknown, LoginOutcome::InvalidCredentials));
    }

    #[tokio::test]
    async fn blank_password_hash_treated_as_unconfigured() {
        let (_dir, db) = open_temp_db().await;
        sqlx::query("UPDATE users SET password_hash = '  ' WHERE email = 'local@anycode'")
            .execute(db.pool())
            .await
            .unwrap();
        let outcome = login(&db, "local@anycode", "anything", true).await.unwrap();
        assert!(matches!(outcome, LoginOutcome::PasswordNotConfigured));
    }
}
