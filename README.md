# URDF / xacro for VS Code

Language server for [URDF](http://wiki.ros.org/urdf) and [xacro](http://wiki.ros.org/xacro) robot description files, providing diagnostics, completions, hover, go-to-definition, refactoring, inlay hints, quick-fixes, color picking, and folding.

Forked from [OTL/vscode-urdf](https://github.com/OTL/vscode-urdf) (which provided XML highlighting and snippets) and rebuilt around a Rust LSP server.

> **Platforms:** Linux x64, macOS x64 (Intel), macOS arm64 (Apple Silicon).

## Features

### Diagnostics

- **XML parse errors** — positioned at the actual misspelled token, not the closing tag or top of file.
- **Mismatched / unclosed XML tags** — the error lands on the opening tag, not where the parser noticed.
- **Undefined link / joint references** — in `<parent link="…"/>`, `<child link="…"/>`, `<mimic joint="…"/>`, and `<gazebo reference="…"/>`.
- **Duplicate link / joint names**.
- **Self-referential joints** (parent == child).
- **Kinematic tree validation** — flags isolated links, multiple roots, and cycles.
- **Undefined xacro `${…}` properties** — math expressions like `${pi/2}` and `${chassis_length/2}` are recognised and not flagged.
- **Unclosed `${…}` expressions** — `radius="${asaso"` is caught at the right place.
- **URDF schema** — unknown elements, unknown attributes, missing required attributes, attribute value type checks (xyz/rpy = 3 floats, rgba = 4 floats in [0,1], etc.).
- **Gazebo schema** — `<gazebo reference>` validated against known links/joints; child properties (`mu1`, `mu2`, `kp`, `kd`, `selfCollide`, `maxVel`, etc.) type-checked.

In `.xacro` files, undefined-reference errors are demoted to warnings since the symbol may come from an included file.

### IDE features

- **Hover** — type, range, and details for links, joints, and xacro properties.
- **Go to Definition** (F12) — jump from `link="…"` / `joint="…"` to the corresponding `<link>` / `<joint>`.
- **Find All References** (Shift+F12).
- **Rename Symbol** (F2) — atomically renames the definition and every reference.
- **Document Symbols** — outline view of links, joints, materials, xacro properties.
- **Completion** — link/joint names inside `link=` / `joint=` / `reference=`, xacro property names after `${`, Gazebo property element names inside `<gazebo>`.
- **Inlay hints** — shows the evaluated value of every `${…}` (`${chassis_length/2}` → `0.1675`). Math evaluator supports `+ - * / %`, parentheses, `pi`, `sin/cos/tan/abs/sqrt/radians/degrees`, and recursive variable resolution.
- **Quick-fixes** (lightbulb / Ctrl+.) — typo corrections for undefined refs (`base_lin` → `base_link`), insert missing required attributes.
- **Folding ranges** — fold any multi-line `<link>`, `<joint>`, `<gazebo>`, `<material>`, `<visual>`, `<collision>`, plugin, etc.
- **Color decorators** — colored swatches next to `<color rgba="…"/>`; click to open VS Code's color picker.
- **Enhanced syntax highlighting** — distinct theme colors for URDF containers, structure tags, geometry primitives, material elements, inertial elements, Gazebo physics properties, and xacro elements.

## Requirements

Linux x64, macOS x64, or macOS arm64. The .vsix bundles a pre-built native language server, so end users do **not** need a Rust toolchain or ROS installation — only VS Code.

## Installation

From the VS Code Marketplace:

```sh
code --install-extension Roy-Pichifkes.urdf
```

Or from Open VSX (VSCodium / Cursor / code-server):

```sh
codium --install-extension Roy-Pichifkes.urdf
```

Or sideload a `.vsix` from the [GitHub Releases](https://github.com/pichifkes/vscode-urdf/releases) page:

```sh
code --install-extension urdf-*.vsix
```

## Building from source

Requires a Rust toolchain (stable) and Node 18+.

```sh
git clone https://github.com/pichifkes/vscode-urdf.git
cd vscode-urdf
npm --prefix client install
npm --prefix client run compile
cargo build --release --manifest-path server/Cargo.toml
mkdir -p server/bin && cp server/target/release/urdf-lsp server/bin/
npx @vscode/vsce package --no-dependencies
code --install-extension urdf-*.vsix
```

## Development

```sh
# Build everything
npm run build

# F5 inside VS Code → opens an Extension Development Host with the
# extension loaded; the preLaunchTask rebuilds client and server.
```

Run the server's tests:

```sh
cd server && cargo test
```

## Architecture

- `client/` — TypeScript thin client that spawns the Rust binary over stdio (`vscode-languageclient`).
- `server/` — Rust LSP server using `tower-lsp` + `roxmltree`.
  - `document.rs` — XML parsing, tag-balance fallback scanner, model of links/joints/materials/xacro properties/gazebo refs.
  - `diagnostics.rs` — semantic checks and schema validation.
  - `features.rs` — hover, definition, completion, references, rename, document symbols, inlay hints, code actions, folding ranges, document colors.
  - `xacro_eval.rs` — recursive-descent expression evaluator for `${…}` substitution.
- `syntaxes/urdf.tmLanguage.json` — TextMate grammar with per-category tag scopes.
- `language-configuration.json` — bracket pairs, auto-closing for `${`.

## Credits

Original snippets and XML-highlighting extension by:
- Olcina
- Fabio Capasso
- Trimple

LSP rewrite, diagnostics, and IDE features added afterwards.

## License

MIT.
