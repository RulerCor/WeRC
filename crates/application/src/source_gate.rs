//! Share immutable source reads, isolate source changes, prioritize foreground work.
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};
use werc_domain::{AppError, AppResult};

#[derive(Default)]
struct State {
    active: bool,
    readers: usize,
    next: u64,
    waiting: VecDeque<u64>,
}

#[derive(Default)]
pub(crate) struct SourceGate {
    state: Mutex<State>,
    changed: Condvar,
}

pub(crate) struct SourcePermit<'a>(&'a SourceGate, bool);

impl SourceGate {
    // Background work and connection/key mutations remain nonblocking. They
    // must not jump ahead of an already waiting foreground query.
    pub(crate) fn try_lock(&self) -> Result<SourcePermit<'_>, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.active || state.readers > 0 || !state.waiting.is_empty() {
            return Err(());
        }
        state.active = true;
        Ok(SourcePermit(self, false))
    }

    pub(crate) fn read(&self, check: impl Fn() -> AppResult<()>) -> AppResult<SourcePermit<'_>> {
        self.wait_for(Duration::from_secs(30), true, check)
    }

    pub(crate) fn exclusive(
        &self,
        check: impl Fn() -> AppResult<()>,
    ) -> AppResult<SourcePermit<'_>> {
        self.wait_for(Duration::from_secs(30), false, check)
    }

    #[cfg(test)]
    fn wait(
        &self,
        timeout: Duration,
        check: impl Fn() -> AppResult<()>,
    ) -> AppResult<SourcePermit<'_>> {
        self.wait_for(timeout, true, check)
    }

    fn wait_for(
        &self,
        timeout: Duration,
        shared: bool,
        check: impl Fn() -> AppResult<()>,
    ) -> AppResult<SourcePermit<'_>> {
        check()?;
        let started = Instant::now();
        let mut state = self.state.lock().map_err(|_| AppError::internal())?;
        let ticket = state.next;
        state.next = state.next.wrapping_add(1);
        state.waiting.push_back(ticket);
        loop {
            let failed = check().err().or_else(|| {
                (started.elapsed() >= timeout)
                    .then(|| AppError::new("BUSY", "来源读取排队超时，请稍后重试"))
            });
            if let Some(error) = failed {
                state.waiting.retain(|id| *id != ticket);
                self.changed.notify_all();
                return Err(error);
            }
            if !state.active
                && (shared || state.readers == 0)
                && state.waiting.front() == Some(&ticket)
            {
                state.waiting.pop_front();
                if shared {
                    state.readers += 1;
                } else {
                    state.active = true;
                }
                self.changed.notify_all();
                return Ok(SourcePermit(self, shared));
            }
            let duration = Duration::from_millis(50).min(timeout.saturating_sub(started.elapsed()));
            state = self
                .changed
                .wait_timeout(state, duration)
                .map_err(|_| AppError::internal())?
                .0;
        }
    }
}

impl Drop for SourcePermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            if self.1 {
                state.readers -= 1;
            } else {
                state.active = false;
            }
            self.0.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    fn wait_queued(gate: &SourceGate, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while gate.state.lock().unwrap().waiting.len() != count {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn foreground_waits_and_background_cannot_overtake_fifo() {
        let gate = Arc::new(SourceGate::default());
        let active = gate.try_lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut threads = Vec::new();
        for id in 0..2 {
            let worker = gate.clone();
            let tx = tx.clone();
            threads.push(std::thread::spawn(move || {
                let _permit = worker.exclusive(|| Ok(())).unwrap();
                tx.send(id).unwrap();
            }));
            wait_queued(&gate, id + 1);
        }
        assert!(gate.try_lock().is_err());
        drop(active);
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(gate.try_lock().is_ok());
    }
    #[test]
    fn cancellation_removes_waiter_without_releasing_active_source() {
        let gate = Arc::new(SourceGate::default());
        let active = gate.try_lock().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let g = gate.clone();
        let c = cancel.clone();
        let thread = std::thread::spawn(move || {
            g.read(|| {
                if c.load(Ordering::SeqCst) {
                    Err(AppError::new("QUERY_CANCELED", "cancel"))
                } else {
                    Ok(())
                }
            })
            .err()
            .unwrap()
            .code
        });
        wait_queued(&gate, 1);
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(thread.join().unwrap(), "QUERY_CANCELED");
        assert!(gate.state.lock().unwrap().waiting.is_empty());
        assert!(gate.try_lock().is_err());
        drop(active);
        assert!(gate.try_lock().is_ok());
    }
    #[test]
    fn timeout_removes_waiter_and_preserves_exclusion() {
        let gate = SourceGate::default();
        let active = gate.try_lock().unwrap();
        assert_eq!(
            gate.wait(Duration::from_millis(20), || Ok(()))
                .err()
                .unwrap()
                .code,
            "BUSY"
        );
        assert!(gate.state.lock().unwrap().waiting.is_empty());
        assert!(gate.try_lock().is_err());
        drop(active);
        assert!(gate.try_lock().is_ok());
    }
    #[test]
    fn cancellation_before_admission_does_not_enter_queue() {
        let gate = SourceGate::default();
        assert_eq!(
            gate.read(|| Err(AppError::new("QUERY_CANCELED", "cancel")))
                .err()
                .unwrap()
                .code,
            "QUERY_CANCELED"
        );
        assert!(gate.try_lock().is_ok());
    }
    #[test]
    fn immutable_readers_overlap_but_exclude_source_mutations() {
        let gate = SourceGate::default();
        let first = gate.read(|| Ok(())).unwrap();
        let second = gate.read(|| Ok(())).unwrap();
        assert_eq!(gate.state.lock().unwrap().readers, 2);
        assert!(gate.try_lock().is_err());
        drop(first);
        assert!(gate.try_lock().is_err());
        drop(second);
        assert!(gate.try_lock().is_ok());
    }
    #[test]
    fn queued_source_change_is_not_starved_by_later_readers() {
        let gate = Arc::new(SourceGate::default());
        let first = gate.read(|| Ok(())).unwrap();
        let (tx, rx) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let g = gate.clone();
        let out = tx.clone();
        let writer = std::thread::spawn(move || {
            let _permit = g.exclusive(|| Ok(())).unwrap();
            out.send("writer").unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        wait_queued(&gate, 1);
        let g = gate.clone();
        let reader = std::thread::spawn(move || {
            let _permit = g.read(|| Ok(())).unwrap();
            tx.send("reader").unwrap();
        });
        wait_queued(&gate, 2);
        drop(first);
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), "writer");
        assert!(rx.try_recv().is_err());
        release.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), "reader");
        writer.join().unwrap();
        reader.join().unwrap();
    }
}
