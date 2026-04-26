mod metadata;

use crate::SesameBuilder;
use std::fs::{copy, create_dir_all, write};
use std::path::{Path, PathBuf};

const DYLINT_VERSION: &str = "2.5.0";
const SERDE_VERSION: &str = "1.0.166";
const SERDE_JSON_VERSION: &str = "1.0.105";

fn dylint_driver_cache_root(builder: &SesameBuilder) -> PathBuf {
    Path::new(&builder.env.out_directory).join("sesame-dylint-drivers")
}

fn dylint_driver_path(builder: &SesameBuilder, toolchain: &str) -> PathBuf {
    dylint_driver_cache_root(builder)
        .join(toolchain)
        .join("dylint-driver")
}

fn dylint_driver_package_dir(builder: &SesameBuilder, toolchain: &str) -> PathBuf {
    Path::new(&builder.env.out_directory)
        .join("sesame-dylint-driver-package")
        .join(toolchain)
}

fn dylint_driver_manifest(toolchain: &str) -> String {
    format!(
        r#"[package]
name = "sesame_dylint_driver_{toolchain}"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "dylint-driver"
path = "src/main.rs"

[dependencies]
anyhow = "1.0"
env_logger = "0.10"
dylint_driver = "={DYLINT_VERSION}"
serde = "={SERDE_VERSION}"
serde_json = "={SERDE_JSON_VERSION}"

[workspace]
"#
    )
}

fn dylint_driver_main_rs() -> &'static str {
    r#"use anyhow::Result;
use std::env;
use std::ffi::OsString;

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<_> = env::args().map(OsString::from).collect();
    dylint_driver::dylint_driver(&args)
}
"#
}

fn dylint_driver_toolchain(toolchain: &str) -> String {
    format!(
        r#"[toolchain]
channel = "{toolchain}"
components = ["llvm-tools-preview", "rustc-dev"]
"#
    )
}

fn dylint_library_toolchain(builder: &SesameBuilder, library_path: &Path) -> String {
    let mut command = builder.command("Sesame Dylint Toolchain", "rustup");
    command
        .arg("show")
        .arg("active-toolchain")
        .current_dir(library_path)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC");

    let output = command
        .execute()
        .expect("Failed to determine Dylint toolchain");
    if !output.status.success() {
        panic!("Failed to determine Dylint toolchain");
    }

    output
        .stdout
        .split_once(' ')
        .map(|(toolchain, _)| toolchain.to_owned())
        .expect("Could not parse Dylint toolchain")
}

fn dylint_driver_rustflags() -> String {
    let sysroot = std::process::Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("Failed to query rustc sysroot");
    let sysroot = String::from_utf8(sysroot.stdout).expect("Invalid rustc sysroot output");
    format!("-C link-args=-Wl,-rpath,{}/lib", sysroot.trim())
}

fn provision_dylint_driver(builder: &SesameBuilder, toolchain: &str) {
    let driver_path = dylint_driver_path(builder, toolchain);
    if driver_path.exists() {
        return;
    }

    let driver_dir = driver_path
        .parent()
        .expect("Dylint driver path must have a parent directory");
    create_dir_all(driver_dir).expect("Failed to create Dylint driver cache directory");

    let package_dir = dylint_driver_package_dir(builder, toolchain);
    let src_dir = package_dir.join("src");
    create_dir_all(&src_dir).expect("Failed to create Dylint driver package source directory");

    write(
        package_dir.join("Cargo.toml"),
        dylint_driver_manifest(toolchain),
    )
    .expect("Failed to write Dylint driver Cargo.toml");
    write(package_dir.join("rust-toolchain.toml"), dylint_driver_toolchain(toolchain))
        .expect("Failed to write Dylint driver rust-toolchain");
    write(package_dir.join("src/main.rs"), dylint_driver_main_rs())
        .expect("Failed to write Dylint driver main.rs");

    let mut command = builder.command("Sesame Dylint Driver", "cargo");
    command
        .arg("build")
        .current_dir(&package_dir)
        .env("RUSTFLAGS", dylint_driver_rustflags());

    let output = command
        .execute()
        .expect("Building pre-pinned Dylint driver failed");
    if !output.status.success() {
        panic!("Building pre-pinned Dylint driver failed");
    }

    copy(package_dir.join("target/debug/dylint-driver"), &driver_path)
        .expect("Failed to copy Dylint driver into cache");
}

fn provision_dylint_drivers(builder: &SesameBuilder) -> PathBuf {
    let libraries = metadata::get_dylinting_libraries(&builder.env.cargo_toml);
    for library in libraries {
        let library_path = Path::new(&builder.env.package_directory).join(library);
        let toolchain = dylint_library_toolchain(builder, &library_path);
        provision_dylint_driver(builder, &toolchain);
    }

    dylint_driver_cache_root(builder)
}

pub fn run_lints(builder: &SesameBuilder) {
    if metadata::get_dylinting_libraries(&builder.env.cargo_toml).len() > 0 {
        let dylint_driver_path = provision_dylint_drivers(builder);
        let mut command = builder.command("Sesame Lints", "cargo");
        command
            .arg("dylint")
            .arg("--all")
            .arg("--workspace")
            .env("DYLINT_DRIVER_PATH", dylint_driver_path)
            .env("RUST_BACKTRACE", "1")
            .env("RUST_LOG", "dylint=warn,dylint_utils=warn");

        let output = command
            .execute()
            .expect("cargo dylint failed");

        if !output.status.success() {
            panic!("Sesame lints failed! See above for manual implementations to replace.");
        }
    }
}

pub fn lint(builder: &SesameBuilder) {
    let profile = &builder.env.profile;
    if profile != "release" {
        builder.logger.info("Sesame lints", &format!("Skipping dylints in {} profile", profile));
        return;
    }

    builder.logger.warn("Sesame lints", "Running dylints");
    run_lints(builder);
    builder.logger.success("Sesame lints", "Sesame lints completed");
}
