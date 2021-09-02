use std::collections::HashMap;
use std::process::id;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_std::sync::Mutex;
use log::*;
use serde::Serialize;

#[derive(Debug)]
pub struct Results<'a> {
    pub results: Mutex<HashMap<u64, Vec<ResultsValue<'a>>>>,
    pub targets: HashMap<&'a str, u64>,
}

impl<'a> Results<'a> {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(HashMap::new()),
            targets: HashMap::new(),
        }
    }

    pub async fn prime(&mut self, addresses: &'a Vec<String>, tries: usize) {
        let mut results = self.results.lock().await;
        let mut identifier = 0;

        for address in addresses {
            let mut target = Vec::new();
            for x in 0..tries {
                target.push(ResultsValue::new(x as u64, address));
            }

            results.insert(identifier, target);
            self.targets.insert(address, identifier);
            identifier += 1;
        }
    }

    // TODO: create internal thread, so this is instant
    pub async fn recv_packet(&self, identifier: u64, seq: u64) -> Result<()> {
        let now = Instant::now();
        let mut cache = self.results.lock().await;
        let mut target = cache.get_mut(&identifier).context("identifier not valid")?;
        let mut res = target.get_mut(seq as usize).context("sequence not valid")?;
        res.recieved(seq, now)?;
        Ok(())
    }

    pub async fn start_packet(&self, idenifier: u64, seq: u64) -> Result<()> {
        let now = Instant::now();
        let mut cache = self.results.lock().await;
        let mut target = cache.get_mut(&idenifier).context("identfifier not valid")?;
        let mut res = target.get_mut(seq as usize).context("sequcene not valid")?;
        res.start(seq, now)?;
        Ok(())
    }

    pub async fn finish(self) -> Vec<JsonResults<'a>> {
        let results = self.results.lock().await;
        let mut ret = Vec::new();
        for (identifier, results) in &*results {
            for result in results {
                ret.push(JsonResults {
                    identifier: *identifier,
                    sequence: result.sequence,
                    target: result.target,
                    state: result.state.finish(),
                });
            }
        }

        return ret;
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResultsValue<'a> {
    sequence: u64,
    target: &'a str,
    state: ResultsState,
}

impl<'a> ResultsValue<'a> {
    pub fn new(sequence: u64, target: &'a str) -> Self {
        Self {
            sequence,
            target,
            state: ResultsState::None,
        }
    }

    pub fn recieved(&mut self, sequence: u64, now: Instant) -> Result<()> {
        if self.sequence != sequence {
            bail!("Invalid sequence");
        }

        self.state = match self.state {
            ResultsState::Started(then) => {
                let dur = now.duration_since(then);
                ResultsState::Succeded(dur)
            }
            v => {
                warn!("recv: sequence {} has state {:?}", sequence, v);
                ResultsState::Failed
            }
        };

        Ok(())
    }

    pub fn start(&mut self, sequence: u64, now: Instant) -> Result<()> {
        if self.sequence != sequence {
            bail!("Invalid sequcene");
        }

        self.state = match self.state {
            ResultsState::None => ResultsState::Started(now),
            v => {
                warn!("start: sequence {} has state {:?}", sequence, v);
                ResultsState::Failed
            }
        };

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ResultsState {
    None,
    Started(Instant),
    Succeded(Duration),
    Failed,
}

impl ResultsState {
    pub fn finish(self) -> JsonResultState {
        match self {
            ResultsState::None | ResultsState::Started(_) | ResultsState::Failed => {
                JsonResultState::Failed
            }
            ResultsState::Succeded(dur) => JsonResultState::Succeded(dur),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum JsonResultState {
    Succeded(Duration),
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonResults<'a> {
    identifier: u64,
    sequence: u64,
    target: &'a str,
    state: JsonResultState,
}
