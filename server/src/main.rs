use std::net::SocketAddr;

use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use async_std::prelude::*;
use async_std::stream::IntoStream;
use getopts::Options;
use log::*;

use server::Config;

#[async_std::main]
async fn main() {
    if let Err(e) = main_err().await {
        eprintln!("Error:");
        eprintln!("{:?}", e);
        std::process::exit(2);
    }
}

// TODO: sigint to stop server gracefully?
async fn main_err() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut options = Options::new();
    options.optflagopt("p", "port", "the port to listen at", "PORT"); // required
    options.optflag("t", "tcp", "use tcp");
    options.optmulti("a", "address", "Address to listen att", "ADDRESS");

    options.optflag("V", "version", "Show version info");
    options.optflag("h", "help", "Show this help message");

    let matches = match options.parse(&args[1..]) {
        Ok(m) => m,
        Err(e) => return Err(e).context("Failed to parse cli arguments"),
    };

    if matches.opt_present("h") {
        let brief = format!("Usage: {} [options]", args[0]);
        print!("{}", options.usage(&brief));
        return Ok(());
    }

    env_logger::init();

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

    let port: u16 = match matches.opt_str("p").map(|p| p.parse()) {
        Some(Ok(p)) => p,
        Some(Err(e)) => return Err(e).context("Failed to parse port"),
        None => bail!("Port not set"),
    };

    //let addresses = match matches.opt_count("")
    let mut addresses = matches.opt_strs("a");

    if addresses.len() == 0 {
        addresses.push("::".to_string());
        // ipv4?
        addresses.push("0.0.0.0".to_string());
    }

    let tcp = matches.opt_present("t");

    let mut config = Config::new(port, addresses, tcp);

    config.run().await
}
