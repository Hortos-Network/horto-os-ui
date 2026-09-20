# horto-os-ui-mcp

MCP server for Horto: status, backup, setup, doctor, docker.

Docs: [docs/tools/mcp.md](../../docs/tools/mcp.md).

```bash
# stdio (Cursor)
horto-os-ui-mcp

# Streamable HTTP (box / LAN)
MCP_HTTP=true HORTO_MCP_MODE=box HORTO_MCP_TOKEN=secret horto-os-ui-mcp --http
```
