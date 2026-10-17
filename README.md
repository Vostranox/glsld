# glsld

A GLSL language server in Rust.

## Install or build


```sh
cargo build --release --locked
./target/release/glsld --version
```

Use `cargo install --path . --locked` to install into Cargo's binary directory.
`glsld` starts the server over stdin/stdout. Use `glsld --check path/to/shader.frag`
to validate a shader from the command line.

## Editors

### VS Code

The [VS Code extension](https://github.com/Vostranox/glsld/tree/main/editors/vscode)
bundles the server and GLSL syntax highlighting. Get the `.vsix` for your platform
from [GitHub Releases](https://github.com/Vostranox/glsld/releases), or
[build a package locally](#local-packages).

```sh
code --install-extension /path/to/glsld.vsix
```

To uninstall:

```sh
code --uninstall-extension Vostranox.glsld
```

For VSCodium, use `codium` instead of `code` in both commands.

### Emacs

```elisp
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs
               '((glsl-mode glsl-ts-mode) . ("glsld"))))
```

### Neovim

```lua
vim.lsp.config("glsld", {
  cmd = { "glsld" },
  filetypes = { "glsl" },
  root_markers = { "glsld.toml", ".git" },
})
vim.lsp.enable("glsld")
```

### Helix

Add to `~/.config/helix/languages.toml`:

```toml
[language-server.glsld]
command = "glsld"

[[language]]
name = "glsl"
language-servers = ["glsld"]
roots = ["glsld.toml", ".git"]
```

## Configuration

Create `glsld.toml` in your project. All settings are optional. The nearest
config between the shader and workspace root wins. Outside a workspace, glsld
searches ancestor directories. Use `--config PATH` to override discovery.

```toml
include_paths = ["shaders/include"]
macros = ["MAX_LIGHTS=4"]
format_on_save = false

[stage_aliases]
".ps" = "frag"

[glslang]
on_change = true
timeout_ms = 5000
target_env = "vulkan1.0"
glsl_version = "450"
```

Paths resolve from the config directory. See the [example config](examples/glsld.toml).

| Setting | Behavior |
| --- | --- |
| `glslang.enable` | Defaults to `true`; `false` disables compiler diagnostics while retaining editor features and lexical checks. |
| `glslang.on_change` | Defaults to `true`; `false` validates on opening and saving. |
| `glslang.timeout_ms` | Compiler timeout, 1–60000 ms; defaults to 5000. |
| `glslang.target_env` | Optional `opengl` or `vulkan1.0` through `vulkan1.4`. |
| `glslang.glsl_version` | Optional version override, such as `450` or `310es`; otherwise uses the source version and compiler defaults. |
| `format_on_save` | Defaults to `false`; requires client support for `willSaveWaitUntil` and uses four-space indentation. |

To use an external validator, add these keys under `[glslang]`:

```toml
path = "glslangValidator"
extra_args = ["--auto-map-bindings"]
```

### Local packages

```sh
cargo build --release --locked --target x86_64-unknown-linux-gnu
python scripts/release.py package --target x86_64-unknown-linux-gnu
cd editors/vscode
npm ci
npm run package -- --target linux-x64 --out ../../dist/glsld-v0.1.0-linux-x64.vsix
```

Use your release version and matching native targets on other platforms:

| Rust target | VSIX target |
| --- | --- |
| `x86_64-unknown-linux-gnu` | `linux-x64` |
| `aarch64-unknown-linux-gnu` | `linux-arm64` |
| `x86_64-apple-darwin` | `darwin-x64` |
| `aarch64-apple-darwin` | `darwin-arm64` |
| `x86_64-pc-windows-msvc` | `win32-x64` |
| `aarch64-pc-windows-msvc` | `win32-arm64` |
