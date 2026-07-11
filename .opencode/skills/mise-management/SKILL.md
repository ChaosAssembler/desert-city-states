---
name: mise-management
description: Use when managing development tool versions with mise — install, upgrade, remove tools and manage mise.toml configuration
---

# Mise Management

## Workflow

1. Installing: `mise use <tool>@<version>` (installs + records), verify with `mise current <tool>`
2. Upgrading: `mise upgrade <tool>`, confirm with `mise ls <tool>`
3. Removing: `mise unuse <tool>` (from config), `mise uninstall <tool>@<version>` (from disk), optionally `mise prune`
4. Investigating: `mise doctor` for diagnostics, `mise ls` for state, `mise trust` if config is untrusted
5. Querying: `mise outdated`, `mise ls-remote <tool>`, `mise search <keyword>`, `mise registry <tool>`

## Conventions

- Always specify version when installing: `<tool>@<version>`
- Use `mise latest <tool>` to discover available versions
- Reference: https://mise.jdx.dev/
