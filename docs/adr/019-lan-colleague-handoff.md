# ADR 019: LAN Colleague Discovery and Handoff

> Formerly numbered ADR 015 (duplicate). Commercial OOXML remains [ADR 015](015-commercial-office-native-ooxml.md).

## Status

Accepted (2026-07)

## Context

Users work on the same local network and want to transfer Agent project experience (sessions, memories, artifacts, workspace) to a colleague's AnyCode Desktop without cloud upload.

The main Workbench binds to loopback (Desktop uses an ephemeral port) and is not reachable from LAN. Cloud device link (ADR 011) covers account auth, not project payload transfer.

## Decision

1. **Discovery**: mDNS service `_anycode._tcp.local` advertising instance id, display name, version, LAN port.
2. **LAN API**: Separate listener on `0.0.0.0:43181` exposing only `/api/lan/*` routes.
3. **Handoff flow**: Sender requests → recipient approves in UI → one-time token → sender uploads gzip tar bundle → recipient imports.
4. **Bundle (`handoff_v1`)**: manifest + optional full workspace + transcripts + reports + memories + artifact files. **Excludes** credentials and API keys.
5. **Kinds**:
   - **Project**: full workspace + project sessions/memories/artifacts.
   - **Session**: selected session experience + artifacts; receiver picks target project or creates new.

## Security

- LAN routes accept connections from private/link-local IPs only.
- Each handoff requires explicit UI approval on the receiver.
- Upload token expires after 5 minutes.
- Audit events: `lan_handoff_requested`, `approved`, `rejected`, `completed`.

## Non-goals (v1)

- Internet/WAN relay (use git/cloud for remote teams).
- Manual IP entry fallback (future).
- E2EE encryption of bundle (LAN trusted network assumption; optional later).

## Consequences

- Firewall must allow TCP 43181 on local network.
- mDNS may be blocked on some enterprise Wi‑Fi; discovery degrades gracefully (empty peer list).
- Large projects may hit default 500MB bundle cap; user can raise in Settings or sync workspace separately.
