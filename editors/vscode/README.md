# glsld for VS Code

GLSL syntax highlighting, completion, hover, navigation, signature help,
formatting and compiler diagnostics. The server and compiler are bundled.

## Get started

Get the `.vsix` for your platform from
[GitHub Releases](https://github.com/Vostranox/glsld/releases), or
[build one locally](https://github.com/Vostranox/glsld#local-packages).
In VS Code, run **Extensions: Install from VSIX…** and select the file.
Then open a saved shader in a trusted workspace.

To uninstall, find **glsld** in the Extensions view and select **Uninstall**.

Common extensions such as `.glsl`, `.vert`, `.frag` and `.comp` are recognized
automatically. Associate other extensions with GLSL in VS Code settings:

```json
{
  "files.associations": { "*.ps": "glsl" }
}
```

## Project configuration

Create `glsld.toml` to configure include paths, macros and shader stages.
The nearest config between a shader and its workspace root applies;
paths resolve from the config directory.

```toml
include_paths = ["shaders/include"]
macros = ["MAX_LIGHTS=8"]

[stage_aliases]
".ps" = "frag"
```

See [configuration](https://github.com/Vostranox/glsld#configuration).
Use VS Code's `editor.formatOnSave`, or set `format_on_save = true` in TOML.
Enable one to avoid duplicate formatting requests.

## Extension settings

- `glsld.path`: absolute path to a custom executable. Leave empty to use the
  bundled server.
- `glsld.watchPatterns`: additional workspace-relative globs for include files.
  `glsld.toml`, common shader extensions, `.h` and `.inc` are already watched.
- `glsld.trace.server`: set to `verbose` to inspect protocol messages in the
  **glsld** output channel.
