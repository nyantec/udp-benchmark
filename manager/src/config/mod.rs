use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;

use crate::os::ContainerManager;
use anyhow::{Context, Result};
use log::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// No output of the unit is expected
    None,

    /// Client json is expected in the output file
    Client,

    /// Output is parsed as literal string
    Passthrough,

    /// This is just a router, and will not exit on its own
    Router,

    /// THis is a router which writes a log file, represented as raw string
    RouterLogging,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    #[serde(default)]
    pub output_type: OutputFormat,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub containers: HashMap<String, Container>,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: &P) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .open(path)
            .context("Failed to open config file")?;

        Ok(serde_json::from_reader(&mut file).context("Failed to parse config")?)
    }

    pub async fn run(self) -> Result<()> {
        let mut manager = ContainerManager::new().context("Failed to create manager")?;

        for (name, config) in &self.containers {
            manager
                .create_container(name, config)
                .await
                .with_context(|| format!("Failed to create container {}", name))?;
        }

        let res = manager.wait()?;

        info!("res: {:?}", res);

        for (name, mut file) in res {
            if let Some(file) = file {
                let mut file: File = file;
                let mut buf = String::new();
                file.read_to_string(&mut buf);
                info!("{}: {}", name, buf);
            }
        }

        /*nix::unistd::sleep(40);

        for (name, file) in manager.get_files() {
            let file: &mut File = file;

            let mut buf = String::new();
            file.read_to_string(&mut buf)
                .with_context(|| format!("Reading output file of {}", name))?;

            info!("{}, {}", name, buf);
        }*/

        Ok(())
    }
}
