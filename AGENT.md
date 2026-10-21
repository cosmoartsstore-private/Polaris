# Polaris Agent Entry

The global Codex rules apply. This file contains only Polaris-specific routing and constraints.

## Read next

- Overview, commands, and installation: `README.md`
- Current runtime behavior: `docs/spec.md`
- Filesystem and registry persistence: `docs/database.md`
- Runtime architecture, source ownership, dependencies, commands, and packaging: `docs/tech-stack.md`

## Constraints

- Production behavior is defined in `src-tauri/src/main.rs`. Polaris has no frontend, WebView, IPC handler, or SQLite database.
- Tauri configuration and dependencies support building and NSIS packaging; do not describe them as an active runtime service.
- Installer hooks terminate the running process. Preserve `Data/archive` during ordinary uninstallation.
- Keep internal notes and verification artifacts under `.claude/`; use `README.md` and `docs/` for public information.
