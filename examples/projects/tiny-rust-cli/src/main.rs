use clap::Parser;

#[derive(Parser)]
#[command(name = "hello", version = "1.0.0")]
struct Args {
    #[arg(short, long)]
    name: Option<String>,
}

fn main() {
    let args = Args::parse();
    let name = args.name.unwrap_or("world".to_string());
    println!("Hello, {}!", name);
}