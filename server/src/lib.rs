use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use async_std::prelude::*;
use async_std::stream::IntoStream;
use getopts::Options;
use log::*;

pub struct Config {
    port: u16,
    addresses: Vec<String>,
    tcp: bool,
    namespace: String,
    exit: AtomicBool,
}

impl Config {
    pub fn new(port: u16, addresses: Vec<String>, tcp: bool) -> Self {
        Self {
            port,
            addresses,
            tcp,
            namespace: module_path!().to_string(),
            exit: AtomicBool::new(false),
        }
    }

    pub fn set_namespace(&mut self, namespace: String) {
        self.namespace = namespace;
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut socket_addresses = Vec::new();
        for address in &self.addresses {
            info!("Listening on '[{}]:{}'", address, self.port);
            let socket_addr = (address.as_str(), self.port)
                .to_socket_addrs()
                .await
                .context("Failed to parse soket address")?
                .collect::<Vec<SocketAddr>>();
            socket_addresses.push(socket_addr);
        }

        let socket_addresses = socket_addresses.concat();

        let exit_flag = &self.exit;
        let exiter = async move {
            loop {
                if exit_flag.load(Ordering::Relaxed) {
                    return true;
                }
                async_std::task::sleep(std::time::Duration::from_millis(500)).await;
            }
        };

        if self.tcp {
            let socket = TcpListener::bind(&*socket_addresses)
                .await
                .context("Failed to open TCP socket")?;

            let mut incoming = socket.incoming();

            let namespace = self.namespace.clone();
            let worker = async move {
                loop {
                    if let Some(Ok(stream)) = incoming.next().await {
                        let namespace = namespace.clone();
                        async_std::task::spawn(async move {
                            if let Err(e) = Self::handle_tcp(stream).await {
                                error!(target: namespace.as_str(), "failed to copy tcp: {}", e);
                            }
                        });
                    }
                }
            };

            worker.race(exiter).await;
        } else {
            let socket = UdpSocket::bind(&*socket_addresses)
                .await
                .context("Failed to open UDP socket")?;

            let worker = async move {
                let mut buf = [0u8; 1500];

                loop {
                    if let Ok((size, addr)) = socket.recv_from(&mut buf).await {
                        debug_assert!(size <= buf.len());
                        let _ = socket.send_to(&buf[..size], addr).await;

                        buf.fill(0);
                        // SAFETY: buf is valid for size bytes
                        //unsafe { libc::memset(buf.as_ptr() as *mut libc::c_void, 0, size) };
                    }
                }
            };

            worker.race(exiter).await;
        }

        bail!("The loop should not exit")
    }

    async fn handle_tcp(stream: TcpStream) -> io::Result<()> {
        let mut reader = stream.clone();
        let mut writer = stream;

        io::copy(&mut reader, &mut writer).await?;

        Ok(())
    }
}
