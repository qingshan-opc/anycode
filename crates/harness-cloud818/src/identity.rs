use anycode_harness_core::{Capabilities, Error, Result, Scope};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Personal,
    Enterprise,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TenantGrant {
    pub organization_id: Uuid,
    pub tenant_id: Uuid,
    pub external_tenant_id: String,
    pub role: Role,
    pub organization_version: i64,
    pub member_version: i64,
    pub tenant_version: i64,
    pub grant_version: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub active: bool,
    pub iss: String,
    pub sub: Uuid,
    pub aud: String,
    pub context: ContextKind,
    pub tenant: Option<TenantGrant>,
    pub scope: String,
}
impl Identity {
    pub fn validate(&self, issuer: &str, expected_tenant: Option<Uuid>) -> Result<()> {
        if !self.active || self.iss != issuer || self.aud != "anycode" || self.sub.is_nil() {
            return Err(Error::Denied("invalid account identity".into()));
        }
        match (&self.context, &self.tenant) {
            (ContextKind::Personal, None) if expected_tenant.is_none() => {}
            (ContextKind::Enterprise, Some(t)) => {
                if t.organization_id.is_nil()
                    || t.tenant_id.is_nil()
                    || t.external_tenant_id.trim().is_empty()
                    || t.external_tenant_id.len() > 256
                    || [
                        t.organization_version,
                        t.member_version,
                        t.tenant_version,
                        t.grant_version,
                    ]
                    .iter()
                    .any(|v| *v < 0)
                    || expected_tenant.is_some_and(|id| id != t.tenant_id)
                {
                    return Err(Error::Denied("tenant grant mismatch".into()));
                }
            }
            _ => return Err(Error::Denied("identity context/tenant mismatch".into())),
        }
        Ok(())
    }
}
pub struct LocalAuthorization {
    pub project: Uuid,
    pub capabilities: Capabilities,
}
#[async_trait]
pub trait ProductAcl: Send + Sync {
    /// Resolve the local user UUID + product tenant mapping + project membership.
    /// SSO success/enterprise role alone MUST NOT authorize a local project.
    async fn authorize_project(
        &self,
        identity: &Identity,
        project: Uuid,
    ) -> Result<LocalAuthorization>;
}
pub struct AuthorizedScope {
    pub scope: Scope,
    pub capabilities: Capabilities,
}
pub async fn authorize(
    identity: &Identity,
    issuer: &str,
    expected_tenant: Option<Uuid>,
    project: Uuid,
    device: Option<Uuid>,
    acl: &dyn ProductAcl,
) -> Result<AuthorizedScope> {
    identity.validate(issuer, expected_tenant)?;
    let local = acl.authorize_project(identity, project).await?;
    if local.project != project {
        return Err(Error::Denied("ACL returned wrong project".into()));
    }
    let scope = Scope {
        subject: identity.sub,
        organization: identity.tenant.as_ref().map(|t| t.organization_id),
        tenant: identity.tenant.as_ref().map(|t| t.tenant_id),
        project,
        device,
    };
    scope.validate()?;
    // Device enrollment and local computer approval are separate, required checks.
    Ok(AuthorizedScope {
        scope,
        capabilities: local.capabilities,
    })
}
pub struct AccountsClient {
    issuer: String,
    secret: String,
    http: reqwest::Client,
}
impl AccountsClient {
    pub fn new(issuer: &str, secret: String, explicit_loopback_dev: bool) -> Result<Self> {
        let url = reqwest::Url::parse(issuer).map_err(|_| Error::Invalid("issuer URL".into()))?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https"
                || explicit_loopback_dev && loopback && url.scheme() == "http")
            || !(32..=512).contains(&secret.len())
        {
            return Err(Error::Invalid(
                "strict origin and strong confidential client secret required".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| Error::Host("account HTTP client".into()))?;
        Ok(Self {
            issuer: url.origin().ascii_serialization(),
            secret,
            http,
        })
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub async fn introspect(&self, token: &str, expected_tenant: Option<Uuid>) -> Result<Identity> {
        if token.len() != 43
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(Error::Denied("opaque token required, not UUID/JWT".into()));
        }
        let mut response = self
            .http
            .post(format!("{}/api/v2/sso/introspect", self.issuer))
            .basic_auth("anycode", Some(&self.secret))
            .json(&serde_json::json!({"token":token}))
            .send()
            .await
            .map_err(|_| Error::Host("identity_service_unavailable".into()))?;
        if response.status().is_server_error() {
            return Err(Error::Host("identity_service_unavailable".into()));
        }
        if !response.status().is_success() {
            return Err(Error::Denied("account authentication denied".into()));
        }
        let mut body = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Host("identity response interrupted".into()))?
        {
            if body.len() + chunk.len() > 16384 {
                return Err(Error::Invalid("identity response limit".into()));
            }
            body.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&body)?;
        if value["active"] != true {
            return Err(Error::Denied("inactive or revoked identity".into()));
        }
        let identity: Identity = serde_json::from_value(value)?;
        identity.validate(&self.issuer, expected_tenant)?;
        Ok(identity)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn personal() -> Identity {
        Identity {
            active: true,
            iss: "https://accounts.818cloud.com".into(),
            sub: Uuid::new_v4(),
            aud: "anycode".into(),
            context: ContextKind::Personal,
            tenant: None,
            scope: "identity".into(),
        }
    }
    #[test]
    fn personal_login_does_not_imply_tenant_access() {
        let p = personal();
        assert!(p.validate(&p.iss, None).is_ok());
        assert!(p.validate(&p.iss, Some(Uuid::new_v4())).is_err());
    }
    #[test]
    fn product_and_issuer_are_exact() {
        let mut p = personal();
        p.aud = "eos".into();
        assert!(p.validate(&p.iss, None).is_err());
        let p = personal();
        assert!(p.validate("https://evil.test", None).is_err());
    }
    #[test]
    fn rejects_http_remote_or_credential_urls() {
        for url in [
            "http://remote.test",
            "https://x@y.test",
            "https://a.test/path",
            "https://a.test/?token=secret",
        ] {
            assert!(AccountsClient::new(url, "x".repeat(32), true).is_err());
        }
    }
    #[test]
    fn inactive_is_never_authenticated() {
        let mut p = personal();
        p.active = false;
        assert!(p.validate(&p.iss, None).is_err());
    }
}
