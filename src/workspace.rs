use crate::{
    config::Config,
    syntax::{self, Include, Index},
    text::{normalize, path_to_uri, uri_to_path},
};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_SOURCE: u64 = 8 * 1024 * 1024;
const MAX_GRAPH_FILES: usize = 256;
const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Source {
    pub uri: String,
    pub path: PathBuf,
    pub text: String,
    pub index: Index,
    pub version: Option<i64>,
}

impl Source {
    pub fn new(uri: String, text: String, version: Option<i64>) -> Result<Self, String> {
        let path = uri_to_path(&uri).ok_or("Only local file:// document URIs are supported")?;
        if text.len() as u64 > MAX_SOURCE {
            return Err("Shader exceeds the 8 MiB source limit".into());
        }
        let index = syntax::index(&text);
        Ok(Self {
            uri,
            path,
            text,
            index,
            version,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProjectConfig {
    pub config: Config,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub sources: Vec<Arc<Source>>,
    pub includes: BTreeMap<(PathBuf, usize), PathBuf>,
    pub config: ProjectConfig,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    pub roots: Vec<PathBuf>,
    pub open: BTreeMap<PathBuf, Arc<Source>>,
    pub explicit_config: Option<PathBuf>,
}

impl Workspace {
    pub fn open(&mut self, uri: String, text: String, version: i64) -> Result<(), String> {
        let source = Arc::new(Source::new(uri, text, Some(version))?);
        self.open.insert(source.path.clone(), source);
        Ok(())
    }
    pub fn source(&self, uri: &str) -> Option<Arc<Source>> {
        self.read(&uri_to_path(uri)?)
    }
    pub fn read(&self, path: &Path) -> Option<Arc<Source>> {
        let path = normalize(path);
        if let Some(source) = self.open.get(&path) {
            return Some(source.clone());
        }
        if std::fs::metadata(&path).ok()?.len() > MAX_SOURCE {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        Source::new(path_to_uri(&path), text, None)
            .ok()
            .map(Arc::new)
    }
    pub fn config_for(&self, path: &Path) -> ProjectConfig {
        let config_path = self.explicit_config.clone().or_else(|| {
            let boundary = self
                .roots
                .iter()
                .filter(|root| path.starts_with(root))
                .max_by_key(|root| root.components().count());
            for dir in path.parent()?.ancestors() {
                let candidate = dir.join("glsld.toml");
                if candidate.is_file() {
                    return Some(candidate);
                }
                if boundary.is_some_and(|root| root == dir) {
                    break;
                }
            }
            None
        });
        let Some(config_path) = config_path else {
            return ProjectConfig {
                config: Config::default(),
                path: None,
                error: None,
            };
        };
        let parsed = std::fs::read_to_string(&config_path)
            .map_err(|e| e.to_string())
            .and_then(|text| Config::parse(&text, config_path.parent().unwrap_or(Path::new("."))));
        match parsed {
            Ok(config) => ProjectConfig {
                config,
                path: Some(config_path),
                error: None,
            },
            Err(error) => {
                let mut config = Config::default();
                config.glslang.enable = false;
                ProjectConfig {
                    config,
                    path: Some(config_path),
                    error: Some(error),
                }
            }
        }
    }
    pub fn resolve_include(
        &self,
        source: &Source,
        include: &Include,
        config: &Config,
    ) -> Option<PathBuf> {
        let name = Path::new(&include.name);
        let mut candidates = Vec::new();
        if name.is_absolute() {
            candidates.push(name.to_path_buf());
        } else {
            if !include.angled {
                candidates.push(source.path.parent()?.join(name));
            }
            candidates.extend(config.include_paths.iter().map(|dir| dir.join(name)));
        }
        candidates
            .into_iter()
            .map(|p| normalize(&p))
            .find(|p| self.open.contains_key(p) || p.is_file())
    }
    pub fn graph(&self, uri: &str) -> Option<Graph> {
        let root = self.source(uri)?;
        let config = self.config_for(&root.path);
        let mut builder = GraphBuilder {
            workspace: self,
            graph: Graph {
                sources: Vec::new(),
                includes: BTreeMap::new(),
                config,
                truncated: false,
            },
            seen: HashSet::new(),
            bytes: 0,
        };
        builder.visit(root);
        Some(builder.graph)
    }
}

struct GraphBuilder<'a> {
    workspace: &'a Workspace,
    graph: Graph,
    seen: HashSet<PathBuf>,
    bytes: usize,
}

impl GraphBuilder<'_> {
    fn visit(&mut self, source: Arc<Source>) {
        if !self.seen.insert(source.path.clone()) {
            return;
        }
        if self.graph.sources.len() >= MAX_GRAPH_FILES
            || self.bytes + source.text.len() > MAX_GRAPH_BYTES
        {
            self.graph.truncated = true;
            return;
        }
        self.bytes += source.text.len();
        self.graph.sources.push(source.clone());
        for include in &source.index.includes {
            if let Some(path) =
                self.workspace
                    .resolve_include(&source, include, &self.graph.config.config)
            {
                self.graph
                    .includes
                    .insert((source.path.clone(), include.start), path.clone());
                if let Some(child) = self.workspace.read(&path) {
                    self.visit(child);
                }
            }
        }
    }
}
