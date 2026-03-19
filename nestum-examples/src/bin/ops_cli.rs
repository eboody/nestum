use clap::Parser;
use nestum_examples::ops_cli::Cli;

fn main() {
    let cli = Cli::parse();
    println!("{}", cli.run());
}
