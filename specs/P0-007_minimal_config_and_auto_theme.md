---

### `P0-007_minimal_config_and_auto_theme.md`
```markdown
# PRD P0-007 – Minimal Config & Auto-Theme

## Principle
Zero config to start (Ghostty style) but optional TOML with live reload.

## Defaults
| Setting | Value |
|---------|-------|
| Font    | SF Mono |
| Font-size | 14 pt |
| Theme   | `system` (auto light/dark) |
| Size    | 90 × 25 |

## Features
* Reload on `SIGUSR1` or `:config-reload`.
* Hard-coded system light/dark themes.
* `include = ["~/.pachyterm/extra.toml"]`.

## Success Metrics
* 90 % dog-fooders use defaults for 1 week with zero complaints.
* Reload latency ≤ 50 ms.

## Out-of-Scope
* Theme marketplace – post-GA.
