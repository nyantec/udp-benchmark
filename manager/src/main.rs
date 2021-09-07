use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use async_std::prelude::*;
use async_std::stream::IntoStream;
use getopts::Options;
use log::*;
use manager::Config;

fn main() {
    if let Err(e) = main_err() {
        eprintln!("Error:");
        eprintln!("{:?}", e);
        std::process::exit(2);
    }
}

fn main_err() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut options = Options::new();

    options.optflag("V", "version", "Show version info");
    options.optflag("h", "help", "Show this help message");

    let matches = options
        .parse(&args[1..])
        .context("Failed to parse cli arguments")?;

    if matches.opt_present("h") {
        let brief = format!("Usage: {} [options] config", args[0]);
        print!("{}", options.usage(&brief));
        return Ok(());
    }

    pretty_env_logger::init();

    if matches.opt_present("V") {
        // TODO: base function/macro?
        eprintln!("{}: Version {}", args[0], env!("CARGO_PKG_VERSION"));
        eprintln!(
            "(C) {}",
            env!("CARGO_PKG_AUTHORS")
                .split(":")
                .collect::<Vec<&str>>()
                .join("\n(C) ")
        );
        return Ok(());
    }

    let configFile = matches.free.get(0).context("No Config file set")?;

    let config = Config::from_file(configFile).context("Failed to build config")?;

    config.run()?;

    Ok(())
}
