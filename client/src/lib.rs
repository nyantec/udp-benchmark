mod results;

use std::sync::atomic::AtomicBool;

use crate::results::{JsonResultState, JsonResults, Results};
use anyhow::{bail, Context, Result};
use async_std::io;
use async_std::net::{
    Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, TcpListener, TcpStream,
    ToSocketAddrs, UdpSocket,
};
use async_std::prelude::*;
use async_std::stream::IntoStream;
use async_std::sync::Mutex;
use log::*;
use packet::{MutableUdpEchoPacket, UdpEcho, UdpEchoPacket};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::process::id;
use std::sync::Arc;

pub struct Config {
    addresses: Vec<String>,
    tcp: bool,
    tries: usize,
    timeout: Option<usize>,
    output: Option<String>,
    namespace: String,
    exit: AtomicBool,
}

impl Config {
    pub fn new(tcp: bool, addresses: Vec<String>, tries: usize) -> Self {
        Self {
            tcp,
            addresses,
            tries,
            timeout: None,
            output: None,
            namespace: module_path!().to_string(),
            exit: AtomicBool::new(false),
        }
    }

    pub fn set_timeout(&mut self, timeout: usize) -> &mut Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn set_output(&mut self, output: String) -> &mut Self {
        self.output = Some(output);
        self
    }

    pub fn set_namespace(&mut self, namespace: String) -> &mut Self {
        self.namespace = namespace;
        self
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut results = Results::new();

        results.prime(&self.addresses, self.tries).await;

        let results = Arc::new(results);

        let mut workers = Vec::new();
        for address in &self.addresses {
            let tcp = self.tcp;
            let identifier = results
                .targets
                .get(address.as_str())
                .context("Failed to find target identifier")?;

            if tcp {
                // TODO
                bail!("TCP not yet implemented");
            } else {
                workers.push(Self::run_udp_target(
                    address,
                    self.tries,
                    *identifier,
                    results.clone(),
                    self.namespace.as_str(),
                ));
                trace!(target: self.namespace.as_str(), "created job for {}", address);
            }
        }

        let future = futures::future::try_join_all(workers);
        //let (future, abortHandle) = futures::future::abortable(future);

        if let Err(e) = if let Some(timeout) = self.timeout {
            let timeouter = async {
                async_std::task::sleep(std::time::Duration::from_secs(timeout as u64)).await;
                bail!("Time exceeded")
            };

            future.race(timeouter).await
        } else {
            future.await
        } {
            warn!(target: self.namespace.as_str(), "Failed to run client: {:?}", e);
        }

        // TODO: error handling
        let results = Arc::try_unwrap(results).unwrap();
        let results = results.finish().await;

        let num_failed = JsonResults::count_failed(&results);
        info!(target: self.namespace.as_str(), "{} requests failed", num_failed);

        if let Some(output) = &self.output {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(output)
                .context("Failed to open output file")?;
            serde_json::to_writer_pretty(&mut file, &results).context("Failed to write json")?;
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&results).context("Failed to create json")?
            );
        }

        Ok(())
    }

    async fn run_udp_target(
        target: &str,
        tries: usize,
        identifier: u64,
        results: Arc<Results<'_>>,
        namespace: &str,
    ) -> Result<()> {
        let address = [
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)),
        ];

        let socket = Arc::new(UdpSocket::bind(address.as_ref()).await?);

        let mut counter = tries;
        let read_half = socket.clone();
        let write_results = results.clone();
        let receiver = async move {
            loop {
                let mut buf = [0u8; 1500];
                read_half.recv(&mut buf).await;
                trace!(target: namespace, "got packet");

                let udp = UdpEchoPacket::new(&buf).unwrap();
                if identifier != udp.get_identifier() {
                    warn!(target: namespace, "invalid identifier in response");
                    continue;
                }

                let seq = udp.get_sequence();
                if let Err(e) = write_results.recv_packet(identifier, seq).await {
                    info!(target: namespace, "failed to store result: {:?}", e);
                }
                counter -= 1;
                if counter == 0 {
                    break;
                }
            }
        };

        let work = async move {
            for x in 0..tries {
                let payload = UdpEcho::new(identifier, x as u64);
                let mut buf = [0u8; 18];
                let mut echo = MutableUdpEchoPacket::new(&mut buf).unwrap();
                echo.populate(&payload);

                socket.send_to(&buf, target).await;
                results.start_packet(identifier, x as u64).await;
                trace!(target: namespace, "send packet {}:{}", identifier, x);
            }
        };

        work.join(receiver).await;

        Ok(())
    }
}
