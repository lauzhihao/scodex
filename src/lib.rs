pub mod adapters;
pub mod cli;
pub mod core;

pub fn run_codex_cli() -> anyhow::Result<i32> {
    let cli = cli::Cli::parse_args();
    let adapter = adapters::codex::CodexAdapter::default();
    cli::run(cli, adapter)
}
