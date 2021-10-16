use core::mem;
use std::alloc::{alloc, alloc_zeroed, Layout};
use std::collections::BTreeMap;
use std::convert::{TryFrom, TryInto};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::mem::MaybeUninit;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::sync::atomic::{AtomicIsize, Ordering};

use crate::config::{Container, OutputFormat};
use anyhow::{bail, Context, Error, Result};
use async_std::sync::Mutex;
use libc;
use log::*;
use nix::sys::signal::{kill, Signal};
use nix::sys::stat::stat;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::sys::{signal, wait};
use nix::unistd::Pid;
use nix::{sched, unistd};

use super::ChildState;

const STACK_SIZE: usize = 1024 * 1024;

struct Child {
    pub(crate) name: String,
    pub(crate) file: Option<File>,
    pub(crate) config: Container,

    pub(crate) state: AtomicIsize,

    pub(crate) pid: Pid,
}

impl Child {
    pub fn spawn_child(name: &str, config: &Container) -> Result<Self> {
        let file = if config.has_file() {
            Some(
                // TODO: retry without O_TMPFILE?
                OpenOptions::new()
                    .custom_flags(libc::O_TMPFILE)
                    .write(true)
                    .read(true)
                    .open("/tmp")
                    .context("Failed to open temporary output file")?,
            )
        } else {
            None
        };

        let fd = file.as_ref().map(|f| f.as_raw_fd());
        trace!("created tempfile fd: {:?}", fd);

        let mut func = {
            let name_clone = name;
            let mut func = move || main_child(name_clone, config, &fd);
            func
        };

        let stack = unsafe {
            (alloc_zeroed(Layout::new::<[u8; STACK_SIZE]>()) as *mut [u8; STACK_SIZE]).as_mut()
        }
        .context("Failed to allocate stack")?;

        let pid = sched::clone(
            Box::new(func),
            stack,
            sched::CloneFlags::CLONE_NEWCGROUP
                | sched::CloneFlags::CLONE_NEWNET
                | sched::CloneFlags::CLONE_NEWUTS,
            Some(nix::sys::signal::Signal::SIGCHLD as i32),
        )
        .context("Failed to clone")?;

        Ok(Self {
            name: name.to_owned(),
            file,
            config: config.clone(),
            state: AtomicIsize::new(1),

            pid,
        })
    }

    pub fn start(&self) -> Result<()> {
        if let Err(state) = self
            .state
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |x| {
                let state: Result<ChildState> = x.try_into();
                if state.is_err() {
                    return None;
                }
                let state = state.unwrap();

                if state != ChildState::Created {
                    return None;
                }

                Some(ChildState::Started.into())
            })
        {
            bail!(
                "Child is in state: `{}`",
                ChildState::try_from(state).unwrap_or(ChildState::None)
            );
        }

        signal::kill(self.pid, signal::SIGHUP).context("Failed to start child")
    }

    pub(crate) fn next_state(&self) -> Result<()> {
        if let Err(state) = self
            .state
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |x| {
                let state = ChildState::try_from(x);
                if state.is_err() {
                    return None;
                }
                let state = state.unwrap();
                if state >= ChildState::Stopped {
                    return None;
                }

                Some(isize::from(state) + 1)
            })
        {
            bail!(
                "state is {}",
                ChildState::try_from(state).unwrap_or(ChildState::None)
            );
        }

        Ok(())
    }

    pub(crate) fn set_state(&self, status: ChildState) -> ChildState {
        let old = self.state.swap(status.into(), Ordering::AcqRel);
        old.try_into().unwrap_or(ChildState::None)
    }

    pub(crate) fn set_exit_state(&self, status: isize) -> Result<()> {
        if let Err(state) = self
            .state
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |x| {
                let state = x.try_into();
                if state.is_err() {
                    return None;
                }
                let state: ChildState = state.unwrap();
                if state >= ChildState::Stopped {
                    return None;
                }

                Some(ChildState::Crashed(status).into())
            })
        {
            bail!(
                "State is already '{}', cannot crash anymore",
                ChildState::try_from(state).unwrap_or(ChildState::None)
            );
        }

        Ok(())
    }

    pub fn get_state(&self) -> ChildState {
        self.state
            .load(Ordering::AcqRel)
            .try_into()
            .unwrap_or(ChildState::None)
    }
}

pub struct ContainerManager {
    childs: Mutex<BTreeMap<Pid, Child>>,
    managers: AtomicIsize,
}

impl ContainerManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            childs: Mutex::new(BTreeMap::new()),
            managers: AtomicIsize::new(0),
        })
    }

    pub async fn create_container(&self, name: &str, config: &Container) -> Result<()> {
        if config.is_router() {
            self.managers.fetch_add(1, Ordering::AcqRel);
        }

        let child = Child::spawn_child(name, config)?;
        self.add_child(child).await;
        Ok(())
    }

    pub async fn start_childs(&self) -> Result<()> {
        let childs = self.childs.lock().await;

        for (_, child) in &*childs {
            child.start();
        }

        Ok(())
    }

    pub async fn wait(&self) -> Result<()> {
        let childs = self.childs.lock().await;
        let mut childs_running = childs.len();
        drop(childs);

        while childs_running != 0 {
            match wait::waitpid(Pid::from_raw(-1), Some(wait::WaitPidFlag::WUNTRACED))
                .context("Failed to wait on pids")?
            {
                WaitStatus::Exited(pid, status) => {
                    let child: &Child = match self.get_child(&pid).await {
                        Some(child) => child,
                        None => {
                            info!("Pid {} was not a container", pid);
                            continue;
                        }
                    };
                    debug!("'{}' stopped with code {}", child.name, status);
                    if status != 0 {
                        child.set_exit_state(status as isize);
                    } else {
                        child.set_state(ChildState::Stopped);
                    }
                    childs_running = childs_running - 1;
                }
                WaitStatus::Signaled(_, _, _) => continue,
                v => bail!("Pid status: {:?} was not expetced", v),
            }
        }

        bail!("todo")
    }

    async fn add_child(&self, child: Child) {
        let mut childs = self.childs.lock().await;
        childs.insert(child.pid.clone(), child);
    }

    async fn get_child<'a>(&'a self, pid: &Pid) -> Option<&'a Child> {
        let mut childs = self.childs.lock().await;
        let child = (*childs).get(pid);
        child.map(|x| unsafe { mem::transmute::<&Child, &'a Child>(x) })
    }
}

/*pub struct ContainerManager {
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
*/
fn main_child(name: &str, config: &Container, fd: &Option<RawFd>) -> isize {
    // setup
    unistd::sethostname(name);

    let signal = child_wait_start();
    if signal < 0 {
        return signal;
    } else if signal as i32 == libc::SIGINT {
        return 1;
    } else if signal as i32 != libc::SIGHUP {
        // how?
        return -libc::ENOTRECOVERABLE as _;
    }
    // TODO: setup network interfaces
    // main
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

    // deinit
    return 0;
}

/// Waits on a SIGHUP signal to start the child
fn child_wait_start() -> isize {
    let mut signal_set = unsafe { mem::zeroed() };
    if unsafe { libc::sigemptyset(&mut signal_set) } != 0 {
        return -1;
    }
    if unsafe { libc::sigaddset(&mut signal_set, libc::SIGHUP) } != 0 {
        return -1;
    }
    if unsafe { libc::sigaddset(&mut signal_set, libc::SIGINT) } != 0 {
        return -1;
    }
    // TODO: SIGCHILD?
    let mut recv_signal = 0;
    let ret = unsafe { libc::sigwait(&signal_set, &mut recv_signal) };
    if ret != 0 {
        return -(ret as isize);
    }

    recv_signal as isize
}
