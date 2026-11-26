//! Machine line state, by polling `machine.list` (`herdr-core-protocol.md` §1 "Remote machines").
//!
//! The core sends no events for it. Like herdr-app, the watcher polls every 2.5 s, and at once
//! on every new connection and whenever terminals come or go.

use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use herdr_proto::methods::MachineList;
use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{MIN_CORE, Mismatch, Version};

use crate::Wake;
use crate::error::Error;
use crate::supervisor::{Poke, SupervisorHandle};

/// How often herdr-app polls `machine.list`.
pub const POLL_EVERY: Duration = Duration::from_millis(2500);

/// One machine and whether this client can use it.
#[derive(Clone, Debug, PartialEq)]
pub struct MachineStatus {
    pub view: MachineView,
    /// Rule 10 for a machine: its hc version (when last seen) must be at least the minimum, and it
    /// must not need an update. Calls through it fail otherwise.
    pub compatible: Result<(), Mismatch>,
}

impl MachineStatus {
    pub fn new(view: MachineView, min: Version) -> Self {
        let have = &view.machine.version;
        let too_old = || Mismatch::TooOld {
            have: have.clone(),
            need: min,
        };
        let compatible = if view.state == MachineState::NeedsUpdate {
            Err(too_old())
        } else {
            match Version::parse(have) {
                Some(version) if version >= min => Ok(()),
                // A machine never reached has no version yet: nothing to hold against it.
                None if have.is_empty() => Ok(()),
                _ => Err(too_old()),
            }
        };
        MachineStatus { view, compatible }
    }
}

/// What a poll found, sent when it differs from the last one (and once per connection).
#[derive(Clone, Debug, PartialEq)]
pub struct Machines {
    pub generation: u64,
    pub machines: Vec<MachineStatus>,
}

/// What a poll that failed said, when it failed for a reason other than a lost connection.
#[derive(Clone, Debug)]
pub enum MachineUpdate {
    Machines(Machines),
    Failed { generation: u64, error: Error },
}

/// Polls `machine.list` on its own thread over the supervisor's current connection.
pub struct MachineWatcher {
    shared: Arc<Shared>,
    poke: Poke,
    updates: mpsc::Receiver<MachineUpdate>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    poked: bool,
    stop: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MachineWatcher {
    /// Starts polling every `every` (use [`POLL_EVERY`]). `wake` is called when an update is
    /// ready.
    pub fn start(supervisor: SupervisorHandle, every: Duration, wake: Wake) -> MachineWatcher {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                poked: true,
                stop: false,
            }),
            changed: Condvar::new(),
        });
        let poke: Poke = {
            let shared = shared.clone();
            Arc::new(move || {
                lock(&shared.state).poked = true;
                shared.changed.notify_all();
            })
        };
        supervisor.on_poke(&poke);
        let (tx, updates) = mpsc::channel();
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("hc-machines".into())
                .spawn(move || run(&shared, &supervisor, every, &tx, &wake))
                .expect("spawning the machine watcher thread")
        };
        MachineWatcher {
            shared,
            poke,
            updates,
            thread: Some(thread),
        }
    }

    /// Polls now, as after `machine.add` or `machine.connect`.
    pub fn poke(&self) {
        (self.poke)();
    }

    pub fn try_recv(&self) -> Option<MachineUpdate> {
        self.updates.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<MachineUpdate> {
        self.updates.recv_timeout(timeout).ok()
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        lock(&self.shared.state).stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for MachineWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    shared: &Shared,
    supervisor: &SupervisorHandle,
    every: Duration,
    updates: &mpsc::Sender<MachineUpdate>,
    wake: &Wake,
) {
    let mut last: Option<Machines> = None;
    loop {
        {
            let state = lock(&shared.state);
            let (mut state, _) = shared
                .changed
                .wait_timeout_while(state, every, |s| !s.poked && !s.stop)
                .unwrap_or_else(PoisonError::into_inner);
            if state.stop {
                return;
            }
            state.poked = false;
        }
        let Some(connection) = supervisor.connection() else {
            continue;
        };
        let generation = connection.generation();
        let update = match connection.call(MachineList::default()).wait() {
            Ok(list) => {
                let machines = Machines {
                    generation,
                    machines: list
                        .machines
                        .into_iter()
                        .map(|view| MachineStatus::new(view, MIN_CORE))
                        .collect(),
                };
                if last.as_ref() == Some(&machines) {
                    continue;
                }
                last = Some(machines.clone());
                MachineUpdate::Machines(machines)
            }
            // The supervisor reports a lost connection; the next one polls again.
            Err(error) if error.is_lost() => continue,
            Err(error) => MachineUpdate::Failed { generation, error },
        };
        let _ = updates.send(update);
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_proto::types::Machine;

    fn view(version: &str, state: MachineState) -> MachineView {
        MachineView {
            state,
            since_ms: None,
            why: None,
            machine: Machine {
                name: "devbox".into(),
                version: version.into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_machine_is_compatible_at_the_minimum_version() {
        let ok = |v, s| MachineStatus::new(view(v, s), MIN_CORE).compatible;
        assert_eq!(ok("0.1.22", MachineState::Connected), Ok(()));
        assert_eq!(ok("0.2.0", MachineState::Unreachable), Ok(()));
        assert_eq!(ok("", MachineState::Connecting), Ok(()));
        assert!(matches!(
            ok("0.1.21", MachineState::Connected),
            Err(Mismatch::TooOld { .. })
        ));
        assert!(matches!(
            ok("0.1.22", MachineState::NeedsUpdate),
            Err(Mismatch::TooOld { .. })
        ));
        assert!(matches!(
            ok("garbage", MachineState::Connected),
            Err(Mismatch::TooOld { .. })
        ));
    }
}
