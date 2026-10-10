//! Packaging entry points for New Terminal.

mod bundle;
mod install;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  bundle    macOS: target/bundle/New Terminal.app, signed ad hoc
  install   bundle, then replace ~/Applications/New Terminal.app
            (refuses while New Terminal runs)
  help      this text
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map_or("help", String::as_str);
    if let Some(extra) = args.get(1) {
        eprintln!("xtask {task}: unknown option {extra}\n\n{USAGE}");
        return ExitCode::FAILURE;
    }

    let outcome = match task {
        "bundle" => bundle::bundle().map(drop),
        "install" => install::install(),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown task: {other}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask {task}: {message}");
            ExitCode::FAILURE
        }
    }
}
