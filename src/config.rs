use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub include_paths: Vec<PathBuf>,
    pub macros: Vec<String>,
    pub format_on_save: bool,
    pub stage_aliases: BTreeMap<String, String>,
    pub glslang: Glslang,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Glslang {
    pub enable: bool,
    pub path: Option<PathBuf>,
    pub on_change: bool,
    pub extra_args: Vec<String>,
    pub target_env: Option<String>,
    pub glsl_version: Option<String>,
    pub timeout_ms: u64,
}

impl Default for Glslang {
    fn default() -> Self {
        Self {
            enable: true,
            path: None,
            on_change: true,
            extra_args: Vec::new(),
            target_env: None,
            glsl_version: None,
            timeout_ms: 5000,
        }
    }
}

pub const STAGES: &[&str] = &[
    "vert", "frag", "comp", "geom", "tesc", "tese", "mesh", "task", "rgen", "rint", "rahit",
    "rchit", "rmiss", "rcall",
];

impl Config {
    pub fn parse(source: &str, base: &Path) -> Result<Self, String> {
        let mut config: Self = toml::from_str(source).map_err(|e| e.to_string())?;
        for path in &mut config.include_paths {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        }
        if let Some(path) = &mut config.glslang.path {
            if path.as_os_str().is_empty() {
                return Err("glslang.path must name an executable, or be omitted to use the bundled validator".into());
            }
            if path.is_relative() && path.components().count() > 1 {
                *path = base.join(&*path);
            }
        }
        for (extension, stage) in &config.stage_aliases {
            if !extension.starts_with('.') || !STAGES.contains(&stage.as_str()) {
                return Err(format!("Invalid stage alias {extension:?} = {stage:?}; use a dotted extension and a GLSL stage such as frag"));
            }
        }
        if !(1..=60_000).contains(&config.glslang.timeout_ms) {
            return Err("glslang.timeout_ms must be between 1 and 60000".into());
        }
        for definition in &config.macros {
            let name = definition.split('=').next().unwrap_or("");
            if !valid_identifier(name) || definition.contains(['\n', '\r', '\0']) {
                return Err(format!("Invalid macro definition {definition:?}"));
            }
        }
        if let Some(env) = &config.glslang.target_env {
            if ![
                "opengl",
                "vulkan1.0",
                "vulkan1.1",
                "vulkan1.2",
                "vulkan1.3",
                "vulkan1.4",
            ]
            .contains(&env.as_str())
            {
                return Err(format!("Unsupported target_env {env:?}"));
            }
        }
        if let Some(version) = &config.glslang.glsl_version {
            if ![
                "100", "110", "120", "130", "140", "150", "300es", "310es", "320es", "330", "400",
                "410", "420", "430", "440", "450", "460",
            ]
            .contains(&version.as_str())
            {
                return Err(format!("Unsupported GLSL version {version:?}"));
            }
        }
        if !config.glslang.extra_args.is_empty() && config.glslang.path.is_none() {
            return Err("glslang.extra_args requires an explicit glslang.path; use target_env, glsl_version and macros with the bundled validator".into());
        }
        for arg in &config.glslang.extra_args {
            if !arg.starts_with('-')
                || arg.starts_with("-o")
                || arg.starts_with("--output")
                || arg == "--stdin"
                || arg == "-S"
                || arg.starts_with("--depfile")
            {
                return Err(format!("Unsupported extra argument {arg:?}; output, stage and input are managed by glsld"));
            }
        }
        Ok(config)
    }

    pub fn stage(&self, path: &Path) -> Option<String> {
        let filename = path.file_name()?.to_str()?;
        let stage_name = filename.strip_suffix(".glsl").unwrap_or(filename);
        let extension = format!(".{}", stage_name.rsplit('.').next()?);
        self.stage_aliases.get(&extension).cloned().or_else(|| {
            let stage = extension.trim_start_matches('.');
            STAGES.contains(&stage).then(|| stage.to_owned())
        })
    }
}

pub fn valid_identifier(name: &str) -> bool {
    let mut chars = name.bytes();
    chars
        .next()
        .is_some_and(|b| b == b'_' || b.is_ascii_alphabetic())
        && chars.all(|b| b == b'_' || b.is_ascii_alphanumeric())
}
