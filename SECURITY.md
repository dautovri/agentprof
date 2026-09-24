# Security Policy

agentprof edits files that control what AI agents may read
(`.claude/settings.json`, `.cursorignore`), can modify your shell rc file
(`fix --shell`, opt-in), and starts MCP servers when you run
`mcp --probe`. Security reports are taken seriously.

## Supported versions

Only the latest release receives fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub:
**Security → Report a vulnerability** on
<https://github.com/dautovri/agentprof/security/advisories/new>.
Do not open a public issue for a security problem.

Include the agentprof version, your OS and shell, the command you ran and
what happened. You can expect an acknowledgement within a week.

## Scope

In scope, for example:

- agentprof reporting a secret as protected when an agent can still read it
- `fix` weakening or discarding existing protection rules
- `mcp --probe` executing commands without the documented opt-in
- the installer or GitHub Action installing an unverified binary

Out of scope: bugs in the agents themselves (for example, Claude Code not
honouring a deny rule). Report those to the vendor.
