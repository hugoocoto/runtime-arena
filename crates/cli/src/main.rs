use clap::Parser;

/// Runtime Arena participant and admin CLI.
#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    eprintln!("arena: no commands implemented yet (see PLAN.md)");
}
