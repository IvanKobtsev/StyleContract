use std::process::ExitCode;

use clap::Parser;
use style_contract::{cli::Cli, config::Config};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("style-contract: could not determine working directory: {error}");
            return ExitCode::from(2);
        }
    };
    let config = match Config::from_cli(cli, cwd.clone()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("style-contract: {error:#}");
            return ExitCode::from(2);
        }
    };
    match style_contract::run(&config) {
        Ok(result) => {
            for diagnostic in &result.diagnostics {
                println!("{}", diagnostic.render(&cwd));
            }
            if result.has_errors {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("style-contract: {error:#}");
            ExitCode::from(2)
        }
    }
}
