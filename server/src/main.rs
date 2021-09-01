use std::net::SocketAddr;

use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use async_std::prelude::*;
use async_std::stream::IntoStream;
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
    let mut socket_addresses = Vec::new();

    if addresses.len() == 0 {
        addresses.push("::".to_string());
        // ipv4?
        addresses.push("0.0.0.0".to_string());
    }

    for address in addresses {
        info!("Listening on '[{}]:{}'", address, port);
        let socket_addr = (address.as_str(), port)
            .to_socket_addrs()
            .await
            .context("Failed to parse soket address")?
            .collect::<Vec<SocketAddr>>();
        socket_addresses.push(socket_addr);
    }

    let socket_addresses = socket_addresses.concat();

    let tcp = matches.opt_present("t");

    if tcp {
        let socket = TcpListener::bind(&*socket_addresses)
            .await
            .context("Failed to open TCP socket")?;

        let mut incoming = socket.incoming();

        while let Some(Ok(stream)) = incoming.next().await {
            async_std::task::spawn(async {
                let _ = handle_tcp(stream).await;
            });
        }
    } else {
        let socket = UdpSocket::bind(&*socket_addresses)
            .await
            .context("Failed to open Udp Socket")?;

        let mut buf = [0u8; 1500];

        while let Ok((size, addr)) = socket.recv_from(&mut buf).await {
            debug_assert!(size <= 1500);
            let _ = socket.send_to(&buf[..size], addr).await;

            buf.fill(0);
            // SAFETY: buf is valid for size bytes
            //unsafe { libc::memset(buf.as_ptr() as *mut libc::c_void, 0, size) };
        }
    }

    unreachable!("After loop");
}

async fn handle_tcp(stream: TcpStream) -> io::Result<()> {
    let mut reader = stream.clone();
    let mut writer = stream;

    io::copy(&mut reader, &mut writer).await?;

    Ok(())
}
