mod adapters;
mod cli;
mod core;

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{}", core::ui::format_top_level_error(&error));
            std::process::exit(1);
        }
    }
}

fn run() -> anyhow::Result<i32> {
    let cli = cli::Cli::parse_args();
    let adapter = adapters::codex::CodexAdapter::default();
    cli::run(cli, adapter)
}
