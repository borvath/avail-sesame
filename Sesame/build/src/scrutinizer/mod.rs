use crate::SesameBuilder;

const SCRUTINIZER_CONFIG: &str = "scrutinizer-config.toml";
const SCRUTINIZER_TOOLCHAIN: &str = "+nightly-2023-08-25";

fn run_scrutinizer(builder: &SesameBuilder) {
    let mut command = builder.command("Scrutinizer", "rustup");
    command
        .arg("run")
        .arg(SCRUTINIZER_TOOLCHAIN.trim_start_matches('+'))
        .arg("cargo")
        .arg("scrutinizer")
        .args(["--config-path", SCRUTINIZER_CONFIG])
        .current_dir(&builder.env.package_directory)
        .env("RUST_BACKTRACE", "full")
        .env("RUST_LOG", "scrutinizer=trace,scrutils=trace");

    for var in [
        "CARGO",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_MAKEFLAGS",
        "LD_LIBRARY_PATH",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTDOC",
        "RUSTFLAGS",
        "RUSTUP_TOOLCHAIN",
        "RUSTUP_TOOLCHAIN_SOURCE",
    ] {
        command.env_remove(var);
    }

    let output = command
        .execute()
        .expect("Failed to execute `cargo scrutinizer`.");

    if !output.status.success() {
        panic!("Scrutinizer failed. See the build log for details.");
    }
}

pub fn scrutinize(builder: &SesameBuilder) {
    let profile = &builder.env.profile;
    if profile != "release" {
        builder.logger.info("Scrutinizer", &format!("Skipping scrutinizer in {} profile", profile));
        return;
    }

    if !builder.env.file_exists(SCRUTINIZER_CONFIG) {
        builder.logger.warn("Scrutinizer", &format!(
            "Skipping scrutinizer because `{}` was not found in {}",
            SCRUTINIZER_CONFIG, builder.env.package_directory
        ));
        return;
    }

    builder.logger.warn("Scrutinizer", "Running scrutinizer analysis");
    run_scrutinizer(builder);
    builder.logger.success("Scrutinizer", "Scrutinizer analysis completed");
}
