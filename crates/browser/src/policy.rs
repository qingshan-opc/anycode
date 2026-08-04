//! Navigation URL policy for the built-in workbench browser.
//!
//! Unlike WebFetch (strict SSRF), local Chromium must open loopback / LAN
//! apps under development (Cursor parity). Still block cloud metadata and
//! non-http(s) schemes.

use crate::error::{BrowserError, BrowserResult};
use url::Url;

fn parse_domain_as_ip_literal(name: &str) -> Option<std::net::IpAddr> {
    let name = name.trim();
    if let Ok(ip) = name.parse::<std::net::IpAddr>() {
        return Some(ip);
    }
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = name.parse::<u32>() {
            return Some(std::net::IpAddr::V4(std::net::Ipv4Addr::from(
                n.to_be_bytes(),
            )));
        }
    }
    None
}

/// Link-local / cloud metadata ranges that must never be navigated.
fn is_metadata_or_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // 169.254.0.0/16 — AWS/GCP metadata & link-local
            v4.octets()[0] == 169 && v4.octets()[1] == 254 || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => v6.is_unspecified(),
    }
}

pub fn validate_navigation_url(raw: &str) -> BrowserResult<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(BrowserError::InvalidUrl("empty URL".into()));
    }
    let url = Url::parse(raw).map_err(|e| BrowserError::InvalidUrl(e.to_string()))?;
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(BrowserError::NavigationBlocked(format!(
            "scheme `{scheme}` not allowed (http/https only)"
        )));
    }
    if let Some(host) = url.host() {
        match host {
            url::Host::Domain(name) => {
                let lower = name.to_ascii_lowercase();
                if lower == "metadata.google.internal"
                    || lower.ends_with(".metadata.google.internal")
                {
                    return Err(BrowserError::NavigationBlocked(
                        "metadata host not allowed".into(),
                    ));
                }
                // localhost / *.localhost allowed for local webdev.
                if lower == "localhost" || lower.ends_with(".localhost") {
                    return Ok(url);
                }
                if let Some(ip) = parse_domain_as_ip_literal(name) {
                    if is_metadata_or_link_local(ip) {
                        return Err(BrowserError::NavigationBlocked(
                            "link-local or metadata IP not allowed".into(),
                        ));
                    }
                }
            }
            url::Host::Ipv4(ip) => {
                if is_metadata_or_link_local(std::net::IpAddr::V4(ip)) {
                    return Err(BrowserError::NavigationBlocked(
                        "link-local or metadata IP not allowed".into(),
                    ));
                }
            }
            url::Host::Ipv6(ip) => {
                if is_metadata_or_link_local(std::net::IpAddr::V6(ip)) {
                    return Err(BrowserError::NavigationBlocked(
                        "link-local or metadata IP not allowed".into(),
                    ));
                }
            }
        }
    }
    Ok(url)
}

/// Whitelisted CDP methods for `BrowserCdp`.
pub fn cdp_method_allowed(method: &str) -> bool {
    matches!(
        method,
        "Runtime.evaluate"
            | "DOM.getDocument"
            | "DOM.querySelector"
            | "CSS.getComputedStyleForNode"
            | "Accessibility.getFullAXTree"
            | "Page.getLayoutMetrics"
            | "Page.captureScreenshot"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_file_scheme() {
        assert!(validate_navigation_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn allows_https_public() {
        assert!(validate_navigation_url("https://example.com").is_ok());
    }

    #[test]
    fn allows_localhost_and_loopback() {
        assert!(validate_navigation_url("http://localhost:43180").is_ok());
        assert!(validate_navigation_url("http://127.0.0.1:3000").is_ok());
        assert!(validate_navigation_url("http://[::1]:5173/").is_ok());
    }

    #[test]
    fn allows_private_lan() {
        assert!(validate_navigation_url("http://192.168.1.10:8080").is_ok());
        assert!(validate_navigation_url("http://10.0.0.2/").is_ok());
    }

    #[test]
    fn blocks_metadata() {
        assert!(validate_navigation_url("http://169.254.169.254/latest").is_err());
        assert!(validate_navigation_url("http://metadata.google.internal/").is_err());
    }
}
