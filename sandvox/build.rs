use std::{
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    path::PathBuf,
    process::Command,
};

use color_eyre::eyre::{
    Error,
    bail,
};
use dotenvy::dotenv;

fn main() -> Result<(), Error> {
    let _ = dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let mut f = BufWriter::new(File::create(out_dir.join("build_info.rs"))?);

    writeln!(f, "pub const BUILD_INFO: BuildInfo = BuildInfo {{")?;
    writeln!(f, "    target: {:?},", std::env::var("TARGET")?)?;
    writeln!(f, "    opt_level: {:?},", std::env::var("OPT_LEVEL")?)?;
    writeln!(f, "    debug: {:?},", std::env::var("DEBUG")?)?;
    writeln!(f, "    profile: {:?},", std::env::var("PROFILE")?)?;
    writeln!(f, "    git_commit: {:?},", git_commit().ok())?;
    writeln!(f, "    git_branch: {:?},", git_branch().ok())?;
    writeln!(f, "    version: {:?},", std::env::var("CARGO_PKG_VERSION")?)?;
    writeln!(f, "}};")?;

    Ok(())
}

fn git_commit() -> Result<String, Error> {
    let output = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    let output = str::from_utf8(output.stdout.trim_ascii())?;
    if output.is_empty() {
        bail!("`git rev-parse` returned no output")
    }
    Ok(output.to_owned())
}

fn git_branch() -> Result<String, Error> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()?;
    let output = str::from_utf8(output.stdout.trim_ascii())?;
    if output.is_empty() {
        bail!("`git branch --show-current` returned no output")
    }
    Ok(output.to_owned())
}
