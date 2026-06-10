mod commands;

use anyhow::Result;

fn main() -> Result<()> {
    commands::dispatch(std::env::args().skip(1).collect())
}
