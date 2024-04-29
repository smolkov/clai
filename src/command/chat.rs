use clap::Parser;

#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}
