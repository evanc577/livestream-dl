use std::env;
use std::io::Error;

use clap::IntoApp;
use clap_complete::{generate_to, Shell};

include!("src/cli.rs");

fn main() -> Result<(), Error> {
    let outdir = match env::var_os("OUT_DIR") {
        None => return Ok(()),
        Some(outdir) => outdir,
    };

    let mut cmd = Args::command();
    let shells = [
        Shell::Bash,
        Shell::Elvish,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Zsh,
    ];

    for shell in shells {
        generate_to(shell, &mut cmd, "livestream-dl", &outdir)?;
    }

    Ok(())
}
