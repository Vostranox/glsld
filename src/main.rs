use std::path::PathBuf;

fn main() {
    match command() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("glsld: {error}");
            std::process::exit(1);
        }
    }
}

fn command() -> Result<i32, String> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut check = None;
    let mut check_config = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--validate-worker" => {
                let request = absolute(args.next().ok_or("--validate-worker requires a request")?)?;
                if args.next().is_some() {
                    return Err("Unexpected validation-worker arguments".into());
                }
                return glsld::bundled_validator::run(&request);
            }
            "--version" | "-V" => {
                println!("glsld {}", env!("CARGO_PKG_VERSION"));
                return Ok(0);
            }
            "--help" | "-h" => {
                println!("glsld {} — GLSL language server\n\nUsage: glsld [--stdio] [--config PATH]\n       glsld --check FILE [--config PATH]\n       glsld --check-config PATH\n\nWith no arguments, speaks LSP over stdin/stdout.\nConfiguration: nearest glsld.toml inside the workspace.\n--check exits with 1 for errors or validator failures.\n--check-config validates TOML and resolves relative paths.", env!("CARGO_PKG_VERSION"));
                return Ok(0);
            }
            "--stdio" => {}
            "--config" => config = Some(absolute(args.next().ok_or("--config requires a path")?)?),
            "--check" => {
                check = Some(absolute(
                    args.next().ok_or("--check requires a shader path")?,
                )?)
            }
            "--check-config" => {
                check_config = Some(absolute(
                    args.next().ok_or("--check-config requires a path")?,
                )?)
            }
            _ => return Err(format!("Unknown argument {arg:?}; run glsld --help")),
        }
    }
    if let Some(path) = check_config {
        let source =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = glsld::config::Config::parse(&source, path.parent().unwrap())?;
        println!("{}: valid", path.display());
        return Ok(0);
    }
    if let Some(path) = check {
        return glsld::validate::check_file(&path, config).map(|ok| if ok { 0 } else { 1 });
    }
    glsld::server::run(config)
}

fn absolute(path: String) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    Ok(glsld::text::normalize(&if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    }))
}
