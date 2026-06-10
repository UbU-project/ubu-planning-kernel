pub mod advisory;
pub mod plan;
pub mod repair;
pub mod validate;

use anyhow::{bail, Result};

pub fn dispatch(args: Vec<String>) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("plan") => plan::run(args.get(1)),
        Some("validate") => validate::run(args.get(1)),
        Some("repair") => repair::run(args.get(1)),
        Some("advisory") => advisory::run(),
        Some(command) => bail!("unknown command '{command}'"),
        None => bail!("expected command: plan, validate, repair, or advisory"),
    }
}
