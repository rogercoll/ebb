use std::{collections::HashMap, time::Duration};

use rustix::{
    buffer::spare_capacity,
    event::epoll,
    fd::OwnedFd,
    io::Errno,
    time::{
        timerfd_create, timerfd_settime, Itimerspec, TimerfdClockId, TimerfdFlags,
        TimerfdTimerFlags, Timespec,
    },
};

use crate::error::{Error, Result};

/// Identifies a tick timer, so it can be told apart and removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Token(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Tick(Token),
    Rescan,
}

struct Source {
    fd: OwnedFd,
    event: Event,
}

/// Waits on every registered source with a single epoll instance.
pub(crate) struct Poller {
    epoll: OwnedFd,
    sources: HashMap<u64, Source>,
    next: u64,
    ready: Vec<epoll::Event>,
}

impl Poller {
    pub(crate) fn new() -> Result<Self> {
        let epoll = epoll::create(epoll::CreateFlags::CLOEXEC).map_err(os("epoll_create"))?;
        Ok(Self {
            epoll,
            sources: HashMap::new(),
            next: 0,
            ready: Vec::new(),
        })
    }

    /// Adds a timer that fires after `first` and then every `period`.
    pub(crate) fn add_tick(&mut self, period: Duration, first: Duration) -> Result<Token> {
        let token = Token(self.next);
        self.add(Self::timer(period, first)?, Event::Tick(token))?;
        Ok(token)
    }

    pub(crate) fn add_rescan(&mut self, period: Duration) -> Result<()> {
        self.add(Self::timer(period, period)?, Event::Rescan)
    }

    pub(crate) fn remove(&mut self, token: Token) -> Result<()> {
        if let Some(source) = self.sources.remove(&token.0) {
            epoll::delete(&self.epoll, &source.fd).map_err(os("epoll_ctl"))?;
        }
        Ok(())
    }

    fn add(&mut self, fd: OwnedFd, event: Event) -> Result<()> {
        let key = self.next;
        epoll::add(
            &self.epoll,
            &fd,
            epoll::EventData::new_u64(key),
            epoll::EventFlags::IN,
        )
        .map_err(os("epoll_ctl"))?;
        self.sources.insert(key, Source { fd, event });
        self.next += 1;
        self.ready.reserve(1);
        Ok(())
    }

    /// Blocks until at least one source is ready and returns what happened.
    pub(crate) fn wait(&mut self) -> Result<Vec<Event>> {
        loop {
            self.ready.clear();
            match epoll::wait(&self.epoll, spare_capacity(&mut self.ready), None) {
                // Interrupted, e.g. by Ctrl-Z / fg: wait again.
                Err(Errno::INTR) => continue,
                result => result.map_err(os("epoll_wait"))?,
            };
            break;
        }
        let mut events = Vec::with_capacity(self.ready.len());
        for ready in &self.ready {
            let Some(source) = self.sources.get(&ready.data.u64()) else {
                continue;
            };
            // Every source is a timer: read the expiration count, otherwise it stays
            // readable and epoll keeps reporting it.
            rustix::io::read(&source.fd, &mut [0; 8]).map_err(os("read timerfd"))?;
            events.push(source.event);
        }
        Ok(events)
    }

    fn timer(period: Duration, first: Duration) -> Result<OwnedFd> {
        let fd = timerfd_create(TimerfdClockId::Monotonic, TimerfdFlags::CLOEXEC)
            .map_err(os("timerfd_create"))?;
        let timespec = |d: Duration| Timespec {
            tv_sec: d.as_secs() as _,
            tv_nsec: d.subsec_nanos() as _,
        };
        let spec = Itimerspec {
            it_interval: timespec(period),
            // A zero first expiry would disarm the timer.
            it_value: timespec(first.max(Duration::from_nanos(1))),
        };
        timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).map_err(os("timerfd_settime"))?;
        Ok(fd)
    }
}

fn os(op: &'static str) -> impl Fn(Errno) -> Error {
    move |source| Error::Os { op, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_in_order_and_removes() {
        let mut poller = Poller::new().unwrap();
        let slow = poller
            .add_tick(Duration::from_secs(60), Duration::from_millis(30))
            .unwrap();
        let fast = poller
            .add_tick(Duration::from_secs(60), Duration::from_millis(10))
            .unwrap();
        let gone = poller
            .add_tick(Duration::from_secs(60), Duration::ZERO)
            .unwrap();
        poller.remove(gone).unwrap();

        assert_eq!(poller.wait().unwrap(), [Event::Tick(fast)]);
        assert_eq!(poller.wait().unwrap(), [Event::Tick(slow)]);
    }
}
