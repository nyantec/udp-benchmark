use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::Mutex;

use crate::config::{Container, OutputFormat};
use anyhow::{bail, Context, Result};
use log::*;
use nix::sys::wait;
use nix::sys::wait::WaitStatus;
use nix::unistd::Pid;
use nix::{sched, unistd};

const STACK_SIZE: usize = 1024 * 1024;

pub struct ContainerManager {
    containers: HashMap<String, Container>,
    pids: HashMap<Pid, String>,
    output: HashMap<String, File>,
}

impl ContainerManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            containers: HashMap::new(),
            pids: HashMap::new(),
            output: HashMap::new(),
        })
    }

    pub fn create_container(&mut self, name: &str, config: &Container) -> Result<()> {
        self.containers.insert(name.to_owned(), config.to_owned());
        debug!("creating container {}", name);
        // TODO: add output temp file
        let output_file = match config.output_type {
            OutputFormat::None => None,
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
            None,
        )
        .context("Failed to clone")?;

        self.store_pid(name.to_owned(), pid)?;
        output_file.map(|f| self.output.insert(name.to_owned(), f));

        Ok(())
    }

    // TODO: result type
    pub fn wait(&mut self) -> Result<HashMap<String, ()>> {
        let mut res = HashMap::new();
        while self.has_childs() {
            debug!("pids: {:?}", self.pids);
            match wait::waitpid(Pid::from_raw(-1), Some(wait::WaitPidFlag::WUNTRACED))
                .context("Failed to wait on pids")?
            {
                WaitStatus::Exited(pid, status) => {
                    let name = match self.pids.remove(&pid) {
                        Some(name) => name,
                        None => {
                            info!("Pid {} is not an container", pid);
                            continue;
                        }
                    };
                    debug!("{} stopped with code {}", name, status);
                    let config = match self.containers.get(&name) {
                        Some(v) => v,
                        None => {
                            warn!("Container {} does not own a config file", name);
                            continue;
                        }
                    };

                    res.insert(name.to_owned(), ());
                }
                //WaitStatus::Signaled(_, _, _) => {}
                //WaitStatus::Stopped(_, _) => {}
                v => bail!("Pid status {:?} was not expected", v),
            }
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
        std::mem::forget(file);
    }

    nix::unistd::sleep(20);

    return 0;
}
