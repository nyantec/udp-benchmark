use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use async_std::prelude::*;
use async_std::stream::IntoStream;
use client::Config;
use getopts::Options;
use log::*;

#[async_std::main]
async fn main() {
    if let Err(e) = main_err().await {
        eprintln!("Error:");
        eprintln!("{:?}", e);
        std::process::exit(2);
    }
}

async fn main_err() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut options = Options::new();
    options.optflag("t", "tcp", "use tcp");
    options.optflagopt("c", "count", "numbers of packages per address", "count");
    options.optflagopt("T", "timeout", "number of seconds until timeout", "seconds");
    options.optflagopt("o", "output", "file to write results into", "FILE");
    // TODO: delay betwen requests
    // TODO: paralel?

    options.optflag("V", "version", "Show version info");
    options.optflag("h", "help", "Show this help message");

    let matches = options
        .parse(&args[1..])
        .context("Failed to parse cli arguments")?;

    if matches.opt_present("h") {
        let brief = format!("Usage: {} [options] addresses", args[0]);
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

    let mut config = Config::new(
        matches.opt_present("t"),
        matches.free.clone(),
        matches
            .opt_str("c")
            .map(|p| p.parse().ok())
            .flatten()
            .unwrap_or(10),
    );

    match matches.opt_str("T").map(|v| v.parse()) {
        Some(Ok(timeout)) => {
            config.set_timeout(timeout);
        }
        Some(v @ Err(_)) => {
            v.context("Failed to parse timeout")?;
        }
        None => (),
    }

    if let Some(output) = matches.opt_str("o") {
        config.set_output(output);
    }

    config.run().await?;

    Ok(())
}
