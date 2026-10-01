use std::{io::IsTerminal, process::ExitCode};

use clap::Parser;
use style_contract::{
    cli::{Cli, OutputStyle},
    config::Config,
    diagnostic::render_rich,
};

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
            match config.output {
                OutputStyle::Minimal => {
                    for diagnostic in &result.diagnostics {
                        println!("{}", diagnostic.render(&config.cwd));
                    }
                }
                OutputStyle::Rich => {
                    let color =
                        std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
                    let rendered = render_rich(&result.diagnostics, &config.cwd, color);
                    if !rendered.is_empty() {
                        println!("{rendered}");
                    }
                }
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
