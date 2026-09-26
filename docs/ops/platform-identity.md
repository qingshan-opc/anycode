# Accounts identity handoff

The SSO v1 ticket compatibility entry remains `/api/auth/hop/login` and
`/api/auth/hop/callback`. The callback now redirects with `lx_code`, never a
12-hour session bearer. Migration `032_portal_login_codes` stores only hashed
90-second exchange codes and browser proofs, referring to the existing user.
The code is consumed atomically with session creation under a row lock.

The portal POSTs the code to `/api/auth/hop/exchange`. Exchange requires the exact
configured portal Origin and host-only HttpOnly `anycode_hop_proof` cookie.
Callback/exchange must be proxied through the same portal host. Never add a shared
parent-domain cookie. Responses are no-store; callback uses no-referrer. Publish
portal and account-service together after the additive migration has been
reviewed. Old `lx_token` URLs are no longer consumed by the portal.

Existing `lingxi_user_id` is the canonical identity. An email match, including an
unbound historical synthetic email, cannot establish ownership. Such conflicts
return 409 and require verified linking/migration. This change does not implement
a user-facing legacy linking workflow or move organization/billing data.

The existing anyCode device linking, local project access and tool approval stay
independent of cloud login. This patch does not wire the platform device/run
adapter or replace local AgentRuntime.

Validation includes account-service unit tests, a disposable MySQL exchange
regression (browser mismatch, expiry, two concurrent consumers, one resulting
session, email conflict), portal TypeScript/build and browser HTTP fixtures for
success/expired-code recovery. The MySQL test uses a minimal synthetic identity/
session schema and migration 032; it is not full historical migration acceptance.

The root workspace fmt and clippy checks pass. Root workspace tests at the clean
baseline encounter `desktop_bootstrap_mints_one_shot_local_session` (303 vs 401)
after excluding loopback from the local system proxy. The primary checkout has
parallel dashboard changes; they were not imported into this task. No production
database, provider, device, model, payment or release was exercised.
