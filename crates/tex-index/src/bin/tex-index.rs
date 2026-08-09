#![forbid(unsafe_code)]
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tex_index::{ProcessCommandRunner, TexEnvironmentConfig, TexIndexBuilder};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("build")) {
        return Err("usage: tex-index build --bin-dir <PATH> --output <PATH>".into());
    }
    let mut bin_dir = None;
    let mut output = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing argument value")?;
        match flag.to_str() {
            Some("--bin-dir") => bin_dir = Some(PathBuf::from(value)),
            Some("--output") => output = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument: {}", flag.to_string_lossy()).into()),
        }
    }
    let output = output.ok_or("--output is required")?;
    let config = TexEnvironmentConfig::new_2026(bin_dir.ok_or("--bin-dir is required")?)?;
    let index = TexIndexBuilder::new(config, Arc::new(ProcessCommandRunner)).build()?;
    atomic_write(&output, &index.canonical_json_bytes()?)?;
    println!("environment_id={}", index.environment_id()?);
    println!("packages={}", index.packages().len());
    println!("runtime_files={}", index.runtime_file_count());
    Ok(())
}
fn atomic_write(destination: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .ok_or("output has no filename")?
        .to_string_lossy();
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, destination)?;
        Ok::<_, std::io::Error>(())
    })() {
        let _cleanup = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
