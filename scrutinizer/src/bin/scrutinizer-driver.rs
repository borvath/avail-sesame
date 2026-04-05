use env_logger::Target;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{exit, Command};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().skip(1).any(|arg| arg == "-V" || arg == "-vV" || arg == "--version") {
        let rustc_path = args
            .get(1)
            .filter(|arg| Path::new(arg).file_stem() == Some("rustc".as_ref()))
            .map(|arg| arg.as_str());
        let forwarded_args = if rustc_path.is_some() {
            args.iter().skip(2)
        } else {
            args.iter().skip(1)
        };
        let status = Command::new(rustc_path.unwrap_or("rustc"))
            .args(forwarded_args)
            .status()
            .expect("failed to query rustc version");
        exit(status.code().unwrap_or(1));
    }

    env_logger::builder()
        .target(Target::Pipe(Box::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open("scrutinizer.log")
                .unwrap(),
        )))
        .init();
    rustc_plugin::driver_main(scrutinizer::ScrutinizerPlugin);
}
