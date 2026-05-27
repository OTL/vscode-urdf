# Change Log

All notable changes to the "urdf" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [0.5.2] - 2026-05-22

### Added
- **Joint type dropdown**: typing `type="` inside a `<joint>` element now shows a completion dropdown with all 6 valid URDF joint types (`revolute`, `continuous`, `prismatic`, `fixed`, `floating`, `planar`).
- **Workspace cross-file analysis**: links, joints, and xacro properties defined in other open or workspace-scanned files no longer produce false-positive diagnostics. On startup the server scans all `.urdf`/`.xacro` files in the workspace; files opened later update the index incrementally. Cross-file references in `.xacro` fragment files are suppressed entirely when the entity is found in the workspace.

## [0.5.1] - 2026-05-22

### Fixed
- Tag-mismatch diagnostic now points at the unexpected closing tag (where the problem is), not the last open element on the stack.
- Undefined xacro properties inside complex `${...}` expressions (e.g. `${(1/12)*mass*(width*typo_var)}`) are now flagged; previously only single-identifier expressions like `${typo_var}` were checked.

## [0.5.0] - 2026-05-22

First marketplace release.

- Published to VS Code Marketplace and Open VSX under publisher `Roy-Pichifkes`.
- Platform-specific .vsix per target: `linux-x64`, `darwin-x64`, `darwin-arm64`. The two Darwin .vsix files share one lipo'd universal binary built on a single Apple Silicon CI runner.
- Native server binary bundled inside each .vsix; no Rust toolchain required for end users.

## [Unreleased]

### 0.4.0-dev

- Reworked from a snippets-only extension into a full language extension.
- Added Rust-based language server (`urdf-lsp`, built with `tower-lsp`) and a thin TypeScript client.
- Registered `urdf` as a first-class language id (no longer aliased to `xml`); kept `.urdf` and `.xacro` file associations.
- Linux x86_64 only for now.

## [0.0.1]

- Initial release