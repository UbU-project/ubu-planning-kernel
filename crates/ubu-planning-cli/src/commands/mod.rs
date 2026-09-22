pub mod advisory;
pub mod plan;
pub mod repair;
pub mod validate;

use anyhow::{bail, Result};

pub fn dispatch(args: Vec<String>) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("plan") => plan::run(&args[1..]),
        Some("validate") => validate::run(args.get(1)),
        Some("repair") => repair::run(&args[1..]),
        Some("advisory") => advisory::run(),
        Some(command) => bail!("unknown command '{command}'"),
        None => bail!("expected command: plan, validate, repair, or advisory"),
    }
}

#[derive(Clone, Copy)]
pub enum Strategy {
    Greedy,
    Chunked,
}

pub fn strategy_args(args: &[String], command: &str) -> Result<(String, Strategy)> {
    let path = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("{command} requires a JSON path"))?;
    let strategy = match &args[1..] {
        [] => Strategy::Greedy,
        [flag, value] if flag == "--strategy" => match value.as_str() {
            "greedy" => Strategy::Greedy,
            "chunked" => Strategy::Chunked,
            _ => bail!("unknown strategy '{value}': expected greedy or chunked"),
        },
        _ => bail!("expected {command} <path> [--strategy greedy|chunked]"),
    };
    Ok((path.clone(), strategy))
}
