use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}


impl Chat {
    pub fn run(&self) -> Result<()> {
        
        Ok(())
    }
}