use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::sync::Mutex;

use crate::config::{Container, OutputFormat};
use anyhow::{bail, Context, Result};
use log::*;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{sched, unistd};

const STACK_SIZE: usize = 1024 * 1024;

pub struct ContainerManager {
    containers: HashMap<String, Container>,
    pids: HashMap<Pid, String>,
    router_pids: HashMap<Pid, String>,
    output: HashMap<String, File>,
}

impl ContainerManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            containers: HashMap::new(),
            pids: HashMap::new(),
            router_pids: HashMap::new(),
            output: HashMap::new(),
        })
    }

    pub async fn create_container(&mut self, name: &str, config: &Container) -> Result<()> {
        self.containers.insert(name.to_owned(), config.to_owned());
        debug!("creating container {}", name);
        // TODO: add output temp file
        let output_file = match config.output_type {
            OutputFormat::None | OutputFormat::Router => None,
            _ => Some(
                OpenOptions::new()
                    .custom_flags(libc::O_TMPFILE)
                    .write(true)
                    .read(true)
                    .open("/tmp")
                    .context("Failed to open temporary output file")?,
            ),
        };

        trace!(
            "created tempfile fd: {:?}",
            output_file.as_ref().map(|f| f.as_raw_fd())
        );

        let mut func = {
            let name_clone = name;
            let output_fd = output_file.as_ref().map(|f| f.as_raw_fd());
            let mut func = move || main_child(name_clone, config, &output_fd);
            func
        };
        let ref mut stack = [0; STACK_SIZE];
        let pid = sched::clone(
            Box::new(func),
            stack,
            sched::CloneFlags::CLONE_NEWCGROUP
                | sched::CloneFlags::CLONE_NEWNET
                | sched::CloneFlags::CLONE_NEWUTS,
            Some(nix::sys::signal::Signal::SIGCHLD as i32),
        )
        .context("Failed to clone")?;

        if let Some(pid) = match config.output_type {
            OutputFormat::Router | OutputFormat::RouterLogging => {
                self.router_pids.insert(pid, name.to_owned())
            }
            _ => self.pids.insert(pid, name.to_owned()),
        } {
            warn!("A pid for '{}' was already stored", name);
        }
        output_file.map(|f| self.output.insert(name.to_owned(), f));

        Ok(())
    }

    // TODO: result type
    pub fn wait(&mut self) -> Result<HashMap<String, Option<File>>> {
        let mut res = HashMap::new();
        // TODO: async with waiter
        while self.has_childs() {
            debug!("pids: {:?}", self.pids);
            debug!("routers: {:?}", self.router_pids);
            match wait::waitpid(Pid::from_raw(-1), Some(wait::WaitPidFlag::WUNTRACED))
                .context("Failed to wait on pids")?
            {
                WaitStatus::Exited(pid, status) => {
                    let name = match self.pids.remove(&pid) {
                        Some(name) => name,
                        None => {
                            info!("Pid {} is not an container", pid);

                            if let Some(name) = self.router_pids.remove(&pid) {
                                warn!("Router '{}' exited without reqesting it", name);
                            }
                            continue;
                        }
                    };
                    debug!("'{}' stopped with code {}", name, status);
                    let config = match self.containers.get(&name) {
                        Some(v) => v,
                        None => {
                            warn!("Container {} does not own a config file", name);
                            continue;
                        }
                    };

                    res.insert(name.to_owned(), self.output.remove(&name));
                }
                //WaitStatus::Signaled(_, _, _) => {}
                //WaitStatus::Stopped(_, _) => {}
                v => bail!("Pid status {:?} was not expected", v),
            }
        }

        // Stop router threads
        for (pid, name) in &self.router_pids {
            info!("Killing router '{}'", name);

            kill(*pid, Signal::SIGINT)
                .with_context(|| format!("Failed to send SIGINT to '{}'", name));

            waitpid(*pid, None).context("Failed to wait on child");
            res.insert(name.to_owned(), self.output.remove(name));
        }

        return Ok(res);
    }

    pub fn has_childs(&self) -> bool {
        self.pids.len() != 0
    }

    pub fn get_files(&mut self) -> &mut HashMap<String, File> {
        &mut self.output
    }

    fn store_pid(&mut self, name: String, pid: Pid) -> Result<()> {
        if let Some(pid) = self.pids.insert(pid, name) {
            warn!("A pid was already stored");
        }

        Ok(())
    }
}

fn main_child(name: &str, config: &Container, fd: &Option<RawFd>) -> isize {
    unistd::sethostname(name);

    if let Some(fd) = fd {
        let mut file = unsafe { File::from_raw_fd(*fd) };
        file.write(format!("testing for {}", name).as_bytes())
            .unwrap();
        info!("wrote into file for {}", name);
        //file.into_raw_fd();
        std::mem::forget(file);
    }

    if config.output_type == OutputFormat::Router
        || config.output_type == OutputFormat::RouterLogging
    {
        nix::unistd::sleep(60);
    };
    nix::unistd::sleep(2);

    return 0;
}
