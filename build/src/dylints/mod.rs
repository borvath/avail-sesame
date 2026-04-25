mod metadata;

use crate::SesameBuilder;

const SCRUTINIZER_TOOLCHAIN: &str = "+nightly-2023-08-25";

pub fn run_lints(builder: &SesameBuilder) {
    if metadata::get_dylinting_libraries(&builder.env.cargo_toml).len() > 0 {
        let mut command = builder.command("Sesame Lints", "rustup");
        command
            .arg("run")
            .arg(SCRUTINIZER_TOOLCHAIN.trim_start_matches('+'))
            .arg("cargo")
            .arg("dylint")
            .arg("--all")
            .arg("--workspace")
            .env("RUST_BACKTRACE", "full")
            .env("RUST_LOG", "dylint=trace,dylint_utils=trace");

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