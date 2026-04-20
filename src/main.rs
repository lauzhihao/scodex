fn main() {
    match s_core::run_codex_cli() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{}", s_core::core::ui::format_top_level_error(&error));
            std::process::exit(1);
        }
    }
}
