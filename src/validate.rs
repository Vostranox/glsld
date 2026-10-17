use crate::{
    bundled_validator, text,
    workspace::{Graph, Source},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Validation {
    pub uri: String,
    pub version: Option<i64>,
    pub diagnostics: BTreeMap<String, Vec<Value>>,
    pub warning: Option<String>,
}

pub fn validate(graph: &Graph) -> Validation {
    validate_cancellable(graph, &|| false)
}

pub fn validate_cancellable(graph: &Graph, cancelled: &dyn Fn() -> bool) -> Validation {
    let source = &graph.sources[0];
    let mut result = Validation {
        uri: source.uri.clone(),
        version: source.version,
        diagnostics: BTreeMap::new(),
        warning: None,
    };
    result.diagnostics.insert(source.uri.clone(), vec![]);
    for unit in &graph.sources {
        for problem in &unit.index.problems {
            result.diagnostics.entry(unit.uri.clone()).or_default().push(json!({"range":text::range(&unit.text,problem.start,problem.end),"severity":1,"source":"glsld","message":problem.message}));
        }
    }
    if let Some(error) = &graph.config.error {
        result
            .diagnostics
            .get_mut(&source.uri)
            .unwrap()
            .push(project_diagnostic(
                1,
                format!(
                    "Invalid {}: {error}",
                    graph
                        .config
                        .path
                        .as_ref()
                        .map_or("glsld.toml".into(), |p| p.display().to_string())
                ),
            ));
        return result;
    }
    if graph.truncated {
        result
            .diagnostics
            .get_mut(&source.uri)
            .unwrap()
            .push(project_diagnostic(
                2,
                "Include graph exceeds 256 files or 32 MiB; validation was skipped".into(),
            ));
        return result;
    }
    let config = &graph.config.config;
    if !config.glslang.enable {
        return result;
    }
    let Some(stage) = config.stage(&source.path) else {
        return result;
    };
    match run(graph, &stage, cancelled) {
        Ok(output) => {
            let mut count = 0;
            for line in output.log.lines() {
                let Some((severity, body)) = line
                    .strip_prefix("ERROR: ")
                    .map(|s| (1, s))
                    .or_else(|| line.strip_prefix("WARNING: ").map(|s| (2, s)))
                else {
                    continue;
                };
                if body.contains("compilation errors.") || body.contains("compilation terminated") {
                    continue;
                }
                let (unit, line_no, column, message) = parse_location(body, graph, &output.sources);
                let message =
                    output
                        .sources
                        .iter()
                        .fold(message.to_owned(), |message, mirrored| {
                            let message = message.replace(
                                &mirrored.path.to_string_lossy().to_string(),
                                &mirrored.source.path.display().to_string(),
                            );
                            message.replace(
                                &mirrored
                                    .path
                                    .file_name()
                                    .unwrap()
                                    .to_string_lossy()
                                    .to_string(),
                                &mirrored.source.path.display().to_string(),
                            )
                        });
                let start = diagnostic_offset(&unit.text, line_no, column, &message);
                let end = unit
                    .index
                    .tokens
                    .iter()
                    .find(|t| t.start <= start && start < t.end)
                    .map_or(start, |t| t.end);
                result.diagnostics.entry(unit.uri.clone()).or_default().push(json!({"range":text::range(&unit.text,start,end),"severity":severity,"source":"glslang","message":message.trim()}));
                count += 1;
            }
            if !output.success && count == 0 {
                let detail: String = output.log.chars().take(1600).collect();
                let message = format!(
                    "glslang validation failed without a source diagnostic: {}",
                    detail.trim()
                );
                result.warning = Some(message.clone());
                result
                    .diagnostics
                    .get_mut(&source.uri)
                    .unwrap()
                    .push(project_diagnostic(2, message));
            }
        }
        Err(error) => {
            result.warning = Some(error.clone());
            result
                .diagnostics
                .get_mut(&source.uri)
                .unwrap()
                .push(project_diagnostic(2, error));
        }
    }
    result
}

fn project_diagnostic(severity: u32, message: String) -> Value {
    json!({"range":text::Range::default(),"severity":severity,"source":"glsld","message":message})
}

struct MirroredSource {
    path: PathBuf,
    source: Arc<Source>,
}

struct ValidatorOutput {
    log: String,
    success: bool,
    sources: Vec<MirroredSource>,
}

fn run(
    graph: &Graph,
    stage: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<ValidatorOutput, String> {
    let scratch = tempfile::Builder::new()
        .prefix("glsld-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let sources = mirror_sources(graph, stage, scratch.path())?;
    let mut command = match &graph.config.config.glslang.path {
        Some(executable) => external_command(graph, stage, scratch.path(), &sources, executable),
        None => bundled_command(graph, stage, scratch.path(), &sources)?,
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.current_dir(scratch.path());
    let log_path = scratch.path().join("diagnostics.log");
    let log = File::create(&log_path).map_err(|error| error.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|error| error.to_string())?)
        .stderr(log);
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Cannot start validator {}: {error}",
            command.get_program().to_string_lossy()
        )
    })?;
    let timeout_ms = graph.config.config.glslang.timeout_ms;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let status = loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Validation cancelled after a document change or shutdown".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "glslang validation timed out after {timeout_ms} ms"
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Cannot wait for glslang: {error}"));
            }
        }
    };
    let mut log = String::new();
    File::open(&log_path)
        .map_err(|error| error.to_string())?
        .take(1024 * 1024)
        .read_to_string(&mut log)
        .map_err(|error| error.to_string())?;
    Ok(ValidatorOutput {
        log,
        success: status.success(),
        sources,
    })
}

fn mirror_sources(
    graph: &Graph,
    stage: &str,
    scratch: &Path,
) -> Result<Vec<MirroredSource>, String> {
    let mut mirrored = BTreeMap::new();
    let mut paths = Vec::new();
    for (i, source) in graph.sources.iter().enumerate() {
        let path = scratch.join(format!("source-{i}.{stage}"));
        mirrored.insert(source.path.clone(), path.clone());
        paths.push(MirroredSource {
            path,
            source: source.clone(),
        });
    }
    for MirroredSource { path, source } in &paths {
        let mut contents = source.text.clone();
        for include in source.index.includes.iter().rev() {
            if let Some(resolved) = graph
                .includes
                .get(&(source.path.clone(), include.start))
                .and_then(|p| mirrored.get(p))
            {
                let name = resolved.file_name().unwrap().to_string_lossy();
                contents.replace_range(include.start - 1..include.end + 1, &format!("\"{name}\""));
            }
        }
        std::fs::write(path, contents).map_err(|e| e.to_string())?;
    }
    Ok(paths)
}

fn bundled_command(
    graph: &Graph,
    stage: &str,
    scratch: &Path,
    sources: &[MirroredSource],
) -> Result<Command, String> {
    let config = &graph.config.config;
    let mut include_paths = config.include_paths.clone();
    if let Some(parent) = graph.sources[0].path.parent() {
        include_paths.push(parent.into());
    }
    let request = bundled_validator::Request {
        source: sources[0].path.clone(),
        stage: stage.into(),
        include_paths,
        macros: config.macros.clone(),
        target_env: config.glslang.target_env.clone(),
        glsl_version: config.glslang.glsl_version.clone(),
    };
    let request_path = scratch.join("request.json");
    let file = File::create(&request_path).map_err(|error| error.to_string())?;
    serde_json::to_writer(file, &request).map_err(|error| error.to_string())?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the glsld validation worker: {error}"))?;
    let mut command = Command::new(executable);
    command.arg("--validate-worker").arg(request_path);
    Ok(command)
}

fn external_command(
    graph: &Graph,
    stage: &str,
    scratch: &Path,
    sources: &[MirroredSource],
    executable: &Path,
) -> Command {
    let config = &graph.config.config;
    let mut command = Command::new(executable);
    command.args(["-S", stage, "-l", "--error-column"]);
    for path in &config.include_paths {
        command.arg(format!("-I{}", path.display()));
    }
    if let Some(parent) = graph.sources[0].path.parent() {
        command.arg(format!("-I{}", parent.display()));
    }
    for definition in &config.macros {
        command.args(["--define-macro", definition]);
    }
    if let Some(version) = &config.glslang.glsl_version {
        command.args(["--glsl-version", version]);
    }
    let mut binary = false;
    if let Some(target) = &config.glslang.target_env {
        command.args(["--target-env", target]);
        binary = true;
    }
    for arg in &config.glslang.extra_args {
        if arg.starts_with("-V") || arg.starts_with("-G") || arg.starts_with("--target-env") {
            binary = true;
        }
        command.arg(arg);
    }
    if binary {
        command.arg("-o").arg(scratch.join("shader.spv"));
    }
    command.arg(&sources[0].path);
    command
}

fn parse_location<'a>(
    body: &'a str,
    graph: &'a Graph,
    paths: &'a [MirroredSource],
) -> (&'a Source, usize, Option<usize>, &'a str) {
    let mut found = None;
    for MirroredSource { path, source } in paths {
        if let Some(rest) = body.strip_prefix(&format!("{}:", path.display())) {
            found = Some((source.as_ref(), rest));
            break;
        }
    }
    if found.is_none() {
        if let Some(rest) = body.strip_prefix("0:") {
            found = Some((graph.sources[0].as_ref(), rest));
        }
    }
    let Some((source, rest)) = found else {
        return (&graph.sources[0], 0, None, body);
    };
    let Some((line, rest)) = rest.split_once(':') else {
        return (source, 0, None, body);
    };
    let Ok(line) = line.parse::<usize>() else {
        return (source, 0, None, body);
    };
    if let Some((column, message)) = rest.split_once(':') {
        if let Ok(column) = column.parse::<usize>() {
            return (
                source,
                line.saturating_sub(1),
                Some(column.saturating_sub(1)),
                message.trim(),
            );
        }
    }
    (source, line.saturating_sub(1), None, rest.trim())
}

fn diagnostic_offset(source: &str, line: usize, column: Option<usize>, message: &str) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match source[start..].find('\n') {
            Some(n) => start += n + 1,
            None => return source.len(),
        }
    }
    let text = source[start..]
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    if let Some(column) = column {
        let mut column = column.min(text.len());
        while !text.is_char_boundary(column) {
            column = column.saturating_sub(1);
        }
        return start + column;
    }
    if let Some(name) = message
        .strip_prefix('\'')
        .and_then(|s| s.split_once('\''))
        .map(|s| s.0)
        .filter(|s| !s.is_empty())
    {
        if let Some(column) = text.find(name) {
            return start + column;
        }
    }
    start + text.len() - text.trim_start().len()
}

pub fn check_file(path: &Path, config: Option<PathBuf>) -> Result<bool, String> {
    let path = text::normalize(path);
    let workspace = crate::workspace::Workspace {
        explicit_config: config,
        ..Default::default()
    };
    let graph = workspace
        .graph(&text::path_to_uri(&path))
        .ok_or_else(|| format!("Cannot read {}", path.display()))?;
    if graph.config.config.glslang.enable && graph.config.config.stage(&path).is_none() {
        return Err("Cannot validate a standalone header without a shader stage. Check an entry shader that includes it, or add a stage_aliases entry in glsld.toml.".into());
    }
    let result = validate(&graph);
    let mut valid = result.warning.is_none() && graph.config.error.is_none() && !graph.truncated;
    let mut out = std::io::stdout().lock();
    for (uri, diagnostics) in result.diagnostics {
        let filename = text::uri_to_path(&uri).map_or(uri, |p| p.display().to_string());
        for diagnostic in diagnostics {
            let line = diagnostic["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
            let column = diagnostic["range"]["start"]["character"]
                .as_u64()
                .unwrap_or(0)
                + 1;
            let severity = if diagnostic["severity"] == 1 {
                valid = false;
                "error"
            } else {
                "warning"
            };
            writeln!(
                out,
                "{filename}:{line}:{column}: {severity}: {}",
                diagnostic["message"].as_str().unwrap_or("")
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if valid {
        writeln!(
            out,
            "{}: OK{}",
            path.display(),
            if graph.config.config.glslang.enable {
                ""
            } else {
                " (lexical checks only; glslang disabled)"
            }
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(valid)
}
