use std::time::Duration;

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

/// Something the daemon waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    /// A periodic timer fired.
    Tick,
}

/// A registered fd and what its readiness means.
struct Source {
    fd: OwnedFd,
    event: Event,
}

/// Waits on every registered source with a single epoll instance.
pub(crate) struct Poller {
    epoll: OwnedFd,
    sources: Vec<Source>,
    ready: Vec<epoll::Event>,
}

impl Poller {
    /// Creates the poller with a timer that fires every `period`, reported as
    /// `Event::Tick`.
    pub(crate) fn new(period: Duration) -> Result<Self> {
        let epoll = epoll::create(epoll::CreateFlags::CLOEXEC).map_err(os("epoll_create"))?;
        let mut poller = Self {
            epoll,
            sources: Vec::new(),
            ready: Vec::new(),
        };

        let fd = timerfd_create(TimerfdClockId::Monotonic, TimerfdFlags::CLOEXEC)
            .map_err(os("timerfd_create"))?;
        let ts = Timespec {
            tv_sec: period.as_secs() as _,
            tv_nsec: period.subsec_nanos() as _,
        };
        let spec = Itimerspec {
            it_interval: ts,
            it_value: ts,
        };
        timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).map_err(os("timerfd_settime"))?;

        poller.add(fd, Event::Tick)?;
        Ok(poller)
    }

    /// Registers `fd`; its epoll data is its index in `sources`.
    fn add(&mut self, fd: OwnedFd, event: Event) -> Result<()> {
        let key = epoll::EventData::new_u64(self.sources.len() as u64);
        epoll::add(&self.epoll, &fd, key, epoll::EventFlags::IN).map_err(os("epoll_ctl"))?;
        self.sources.push(Source { fd, event });
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
            let source = &self.sources[ready.data.u64() as usize];
            match source.event {
                // Read the expiration count, otherwise the timer stays readable and epoll
                // keeps reporting it.
                Event::Tick => {
                    rustix::io::read(&source.fd, &mut [0; 8]).map_err(os("read timerfd"))?;
                }
            }
            events.push(source.event);
        }
        Ok(events)
    }
}

fn os(op: &'static str) -> impl Fn(Errno) -> Error {
    move |source| Error::Os { op, source }
}
