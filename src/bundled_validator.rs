use glslang_sys::{
    self as sys, glslang_client_t as Client, glslang_messages_t as Messages,
    glslang_profile_t as GlslProfile, glslang_stage_t as ShaderStage,
    glslang_target_client_version_t as ClientVersion, glslang_target_language_t as TargetLanguage,
    glslang_target_language_version_t as SpirvVersion,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    ptr::{self, NonNull},
    sync::OnceLock,
};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INCLUDED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub source: PathBuf,
    pub stage: String,
    pub include_paths: Vec<PathBuf>,
    pub macros: Vec<String>,
    pub target_env: Option<String>,
    pub glsl_version: Option<String>,
}

#[doc(hidden)]
pub fn run(request_path: &Path) -> Result<i32, String> {
    let request: Request = serde_json::from_reader(
        File::open(request_path)
            .map_err(|error| error.to_string())?
            .take(1024 * 1024),
    )
    .map_err(|error| format!("Invalid validation request: {error}"))?;
    let mut output = std::io::stdout().lock();
    match compile(&request) {
        Ok(log) => {
            write!(output, "{log}").map_err(|error| error.to_string())?;
            Ok(0)
        }
        Err(log) => {
            writeln!(output, "{log}").map_err(|error| error.to_string())?;
            Ok(1)
        }
    }
}

fn compile(request: &Request) -> Result<String, String> {
    initialize_compiler()?;
    let source = read_source(&request.source).map_err(|error| error.to_string())?;
    let source = CString::new(with_version(source, request.glsl_version.as_deref())?)
        .map_err(|error| error.to_string())?;
    let preamble = macro_preamble(&request.macros)?;
    let mut includes = Includes {
        root: request.source.parent().unwrap_or(Path::new(".")).into(),
        search_paths: request.include_paths.clone(),
        bytes_read: 0,
    };
    let target = Target::parse(request.target_env.as_deref())?;
    let (version, profile) = request
        .glsl_version
        .as_deref()
        .map(version_profile)
        .transpose()?
        .unwrap_or((100, GlslProfile::None));
    let input = Box::new(sys::glslang_input_t {
        language: sys::glslang_source_t::GLSL,
        stage: stage(&request.stage)?,
        client: target.client,
        client_version: target.version,
        target_language: target.language,
        target_language_version: target.spirv_version,
        code: source.as_ptr(),
        default_version: version,
        default_profile: profile,
        force_default_version_and_profile: 0,
        forward_compatible: 0,
        messages: target.messages,
        resource: &crate::validator_resources::DEFAULT_RESOURCES,
        callbacks: sys::glsl_include_callbacks_t {
            include_system: Some(include_system),
            include_local: Some(include_local),
            free_include_result: Some(free_include),
        },
        callbacks_ctx: ptr::from_mut(&mut includes).cast(),
    });
    let shader = Shader(
        NonNull::new(unsafe { sys::glslang_shader_create(&*input) })
            .ok_or("Cannot allocate shader")?,
    );
    unsafe { sys::glslang_shader_set_preamble(shader.0.as_ptr(), preamble.as_ptr()) };
    let parsed = unsafe {
        sys::glslang_shader_preprocess(shader.0.as_ptr(), &*input) != 0
            && sys::glslang_shader_parse(shader.0.as_ptr(), &*input) != 0
    };
    let mut log = unsafe { native_log(sys::glslang_shader_get_info_log(shader.0.as_ptr())) };
    if !parsed {
        return Err(log);
    }
    let program = Program(
        NonNull::new(unsafe { sys::glslang_program_create() })
            .ok_or("Cannot allocate shader program")?,
    );
    let linked = unsafe {
        sys::glslang_program_add_shader(program.0.as_ptr(), shader.0.as_ptr());
        sys::glslang_program_link(program.0.as_ptr(), target.messages.0) != 0
    };
    log.push_str(&unsafe { native_log(sys::glslang_program_get_info_log(program.0.as_ptr())) });
    if linked {
        Ok(log)
    } else {
        Err(log)
    }
}

fn macro_preamble(definitions: &[String]) -> Result<CString, String> {
    let mut preamble = String::new();
    for definition in definitions {
        let (name, value) = definition.split_once('=').unwrap_or((definition, "1"));
        if !crate::config::valid_identifier(name) || value.contains(['\n', '\r', '\0']) {
            return Err(format!("Invalid macro definition {definition:?}"));
        }
        preamble.push_str(&format!("#define {name} {value}\n"));
    }
    CString::new(preamble).map_err(|error| error.to_string())
}

fn with_version(mut source: String, override_version: Option<&str>) -> Result<String, String> {
    let Some(version) = override_version else {
        return Ok(source);
    };
    let (number, profile) = version_profile(version)?;
    let profile = if profile == GlslProfile::ES {
        " es"
    } else {
        ""
    };
    let (tokens, _) = crate::syntax::lex(&source);
    if let Some(token) = tokens
        .iter()
        .find(|token| token.kind != crate::syntax::TokenKind::Comment)
    {
        if token.kind == crate::syntax::TokenKind::Directive
            && token.text(&source)[1..].split_whitespace().next() == Some("version")
        {
            let blank: String = token
                .text(&source)
                .chars()
                .map(|character| {
                    if matches!(character, '\n' | '\r') {
                        character
                    } else {
                        ' '
                    }
                })
                .collect();
            source.replace_range(token.start..token.end, &blank);
        }
    }
    Ok(format!("#version {number}{profile}\n#line 1\n{source}"))
}

struct Target {
    client: Client,
    version: ClientVersion,
    language: TargetLanguage,
    spirv_version: SpirvVersion,
    messages: Messages,
}

impl Target {
    fn parse(environment: Option<&str>) -> Result<Self, String> {
        let mut target = Self {
            client: Client::None,
            version: ClientVersion::OpenGL450,
            language: TargetLanguage::None,
            spirv_version: SpirvVersion::SPIRV1_0,
            messages: Messages::DISPLAY_ERROR_COLUMN,
        };
        let Some(environment) = environment else {
            return Ok(target);
        };
        target.language = TargetLanguage::SPIRV;
        target.messages |= Messages::SPV_RULES;
        if environment == "opengl" {
            target.client = Client::OpenGL;
            return Ok(target);
        }
        target.client = Client::Vulkan;
        target.messages |= Messages::VULKAN_RULES;
        (target.version, target.spirv_version) = match environment {
            "vulkan1.0" => (ClientVersion::Vulkan1_0, SpirvVersion::SPIRV1_0),
            "vulkan1.1" => (ClientVersion::Vulkan1_1, SpirvVersion::SPIRV1_3),
            "vulkan1.2" => (ClientVersion::Vulkan1_2, SpirvVersion::SPIRV1_5),
            "vulkan1.3" => (ClientVersion::Vulkan1_3, SpirvVersion::SPIRV1_6),
            "vulkan1.4" => (ClientVersion::Vulkan1_4, SpirvVersion::SPIRV1_6),
            other => return Err(format!("Unsupported target environment {other:?}")),
        };
        Ok(target)
    }
}

fn initialize_compiler() -> Result<(), String> {
    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    if *INITIALIZED.get_or_init(|| unsafe { sys::glslang_initialize_process() != 0 }) {
        Ok(())
    } else {
        Err("Cannot initialize bundled glslang".into())
    }
}

struct Shader(NonNull<sys::glslang_shader_t>);
impl Drop for Shader {
    fn drop(&mut self) {
        unsafe { sys::glslang_shader_delete(self.0.as_ptr()) };
    }
}

struct Program(NonNull<sys::glslang_program_t>);
impl Drop for Program {
    fn drop(&mut self) {
        unsafe { sys::glslang_program_delete(self.0.as_ptr()) };
    }
}

unsafe fn native_log(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

fn version_profile(version: &str) -> Result<(i32, GlslProfile), String> {
    let (number, profile) = match version.strip_suffix("es") {
        Some(number) => (number, GlslProfile::ES),
        None => (version, GlslProfile::None),
    };
    number
        .parse()
        .map(|version| (version, profile))
        .map_err(|error| format!("Invalid GLSL version {version:?}: {error}"))
}

fn stage(name: &str) -> Result<ShaderStage, String> {
    match name {
        "vert" => Ok(ShaderStage::Vertex),
        "frag" => Ok(ShaderStage::Fragment),
        "comp" => Ok(ShaderStage::Compute),
        "geom" => Ok(ShaderStage::Geometry),
        "tesc" => Ok(ShaderStage::TesselationControl),
        "tese" => Ok(ShaderStage::TesselationEvaluation),
        "mesh" => Ok(ShaderStage::Mesh),
        "task" => Ok(ShaderStage::Task),
        "rgen" => Ok(ShaderStage::RayGeneration),
        "rint" => Ok(ShaderStage::Intersect),
        "rahit" => Ok(ShaderStage::AnyHit),
        "rchit" => Ok(ShaderStage::ClosestHit),
        "rmiss" => Ok(ShaderStage::Miss),
        "rcall" => Ok(ShaderStage::Callable),
        _ => Err(format!("Unsupported shader stage {name:?}")),
    }
}

fn read_source(path: &Path) -> Result<String, std::io::Error> {
    let mut text = String::new();
    File::open(path)?
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_SOURCE_BYTES || text.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Shader input exceeds 32 MiB or contains a NUL byte",
        ));
    }
    Ok(text)
}

struct Includes {
    root: PathBuf,
    search_paths: Vec<PathBuf>,
    bytes_read: usize,
}

impl Includes {
    fn include(
        &mut self,
        local: bool,
        name: &str,
        includer: &str,
        depth: usize,
    ) -> Option<IncludeAllocation> {
        if depth > 256 || self.bytes_read >= MAX_INCLUDED_BYTES {
            return None;
        }
        let local_directory = if includer.is_empty() {
            self.root.as_path()
        } else {
            Path::new(includer).parent().unwrap_or(&self.root)
        };
        let candidates = local
            .then(|| local_directory.join(name))
            .into_iter()
            .chain(self.search_paths.iter().map(|path| path.join(name)));
        for path in candidates {
            let Ok(data) = read_source(&path) else {
                continue;
            };
            self.bytes_read += data.len();
            if self.bytes_read > MAX_INCLUDED_BYTES {
                return None;
            }
            return IncludeAllocation::new(&path, data);
        }
        None
    }
}

#[repr(C)]
struct IncludeAllocation {
    result: sys::glsl_include_result_t,
    name: CString,
    data: CString,
}

impl IncludeAllocation {
    fn new(path: &Path, data: String) -> Option<Self> {
        let name = CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let data = CString::new(data).ok()?;
        Some(Self {
            result: sys::glsl_include_result_t {
                header_name: name.as_ptr(),
                header_data: data.as_ptr(),
                header_length: data.as_bytes().len(),
            },
            name,
            data,
        })
    }
}

unsafe extern "C" fn include_local(
    context: *mut c_void,
    name: *const c_char,
    includer: *const c_char,
    depth: usize,
) -> *mut sys::glsl_include_result_t {
    unsafe { include_callback(context, name, includer, depth, true) }
}

unsafe extern "C" fn include_system(
    context: *mut c_void,
    name: *const c_char,
    includer: *const c_char,
    depth: usize,
) -> *mut sys::glsl_include_result_t {
    unsafe { include_callback(context, name, includer, depth, false) }
}

unsafe fn include_callback(
    context: *mut c_void,
    name: *const c_char,
    includer: *const c_char,
    depth: usize,
    local: bool,
) -> *mut sys::glsl_include_result_t {
    if context.is_null() || name.is_null() || includer.is_null() {
        return ptr::null_mut();
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let includes = unsafe { &mut *context.cast::<Includes>() };
        let name = unsafe { CStr::from_ptr(name) }.to_str().ok()?;
        let includer = unsafe { CStr::from_ptr(includer) }.to_str().ok()?;
        let allocation = includes.include(local, name, includer, depth)?;
        Some(Box::into_raw(Box::new(allocation)).cast())
    }))
    .ok()
    .flatten()
    .unwrap_or(ptr::null_mut())
}

unsafe extern "C" fn free_include(
    _context: *mut c_void,
    result: *mut sys::glsl_include_result_t,
) -> i32 {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result.cast::<IncludeAllocation>()) });
    }
    0
}
