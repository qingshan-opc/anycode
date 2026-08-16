# MCP integration

Tools prefixed `mcp__` (e.g. `mcp__<server>__<tool>`) come from Model Context Protocol servers configured for this session.

- Treat MCP tools like native tools: read their schemas, pass arguments exactly, handle errors from the server.
- A server may be temporarily unreachable — report the failure instead of guessing. The host does **not** auto-reconnect dead stdio MCP processes (see ADR 007); the user must reconnect from Workbench Settings or restart the Desktop app.
- Resource URIs (`ReadMcpResource`) are scoped to the connected servers; do not fabricate URIs.
- When a tool reports `session_expired` or `authentication_required`, surface it and stop. OAuth for MCP is configured in Workbench / Desktop settings (credentials store), not via a terminal `anycode mcp` command.
