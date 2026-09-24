#!/usr/bin/env bash
# Creates a small demo workspace that exercises every agentprof check, so the
# examples in the README and on the website can be reproduced exactly:
#
#   scripts/demo-workspace.sh /tmp/agentprof-demo
#   cd /tmp/agentprof-demo && HOME="$PWD/.home" agentprof scan
#
# Nothing in it is a real secret.
set -euo pipefail

dir="${1:?usage: demo-workspace.sh <directory>}"
mkdir -p "$dir"
cd "$dir"

mkdir -p .home .claude/skills/pr-review .claude/skills/security-review \
  .claude/skills/design-review certs node_modules/left-pad src

cat > CLAUDE.md <<'EOF'
# Project rules

- Keep pull requests under 400 changed lines.
- Follow best practices.
- Never commit generated files unless the build requires them.

## Swift Style

- Use `@Observable` for view models.
- Prefer value types; mark classes `final`.
- Use `NavigationStack` for navigation.

## Testing

- Every bug fix ships with a regression test.
- Run `swift test --parallel` before pushing.
- Snapshot tests live in `Tests/Snapshots`.

## Release

- Tag releases from `main` only.
EOF

cat > AGENTS.md <<'EOF'
# Agent notes

- Use ObservableObject for view models.
- Run the full test suite before opening a pull request.
EOF

cat > .gitignore <<'EOF'
.build/
.home/
EOF

printf 'API_KEY=example-not-a-real-key\n' > .env
printf 'API_KEY=\n' > .env.example
printf -- '-----BEGIN EXAMPLE KEY-----\nnot a real key\n-----END EXAMPLE KEY-----\n' > certs/dev.pem
printf 'module.exports = (s, n) => s.padStart(n);\n' > node_modules/left-pad/index.js
printf 'print("hello")\n' > src/main.py

skill() {
  cat > ".claude/skills/$1/SKILL.md" <<EOF
---
name: $1
description: $2
---

# $1

$(for i in $(seq 1 "$3"); do printf -- '- Step %s: check the change carefully and report findings.\n' "$i"; done)
EOF
}
skill pr-review "Review pull requests for correctness and style." 40
skill security-review "Security review of code changes before merge." 620
skill design-review "Review UI design changes against the design system." 25

# A tiny stdio MCP server with a handful of tools, so `mcp --probe` has
# something real to measure.
cat > mcp-demo-server.sh <<'EOF'
#!/bin/sh
tools='[{"name":"search_issues","description":"Search issues with filters for labels, state, assignee and milestone.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"labels":{"type":"array","items":{"type":"string"}},"state":{"type":"string","enum":["open","closed","all"]}}}},{"name":"create_issue","description":"Create an issue with a title, body and labels.","inputSchema":{"type":"object","properties":{"title":{"type":"string"},"body":{"type":"string"},"labels":{"type":"array","items":{"type":"string"}}},"required":["title"]}},{"name":"list_pull_requests","description":"List pull requests, optionally filtered by author and base branch.","inputSchema":{"type":"object","properties":{"author":{"type":"string"},"base":{"type":"string"}}}}]'
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"demo","version":"1.0"}}}' ;;
    *'tools/list'*) echo "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":$tools}}" ;;
  esac
done
EOF
chmod +x mcp-demo-server.sh
printf '{"mcpServers":{"issues":{"command":"sh","args":["%s/mcp-demo-server.sh"]}}}\n' "$PWD" > .home/.claude.json

echo "Demo workspace ready in $PWD"
echo "Try:  cd $PWD && HOME=\"\$PWD/.home\" agentprof scan"
