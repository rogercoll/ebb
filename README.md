# ebb

Proactive memory reclaim for cgroup v2 leaf cgroups, driven by pressure stall
information (PSI).

## Motivation

`memory.current` is not the memory a workload needs. It also counts code pages
that were only touched at startup and file cache that was read once, and the
kernel only reclaims those pages when it runs out of free memory. Until then a
container looks bigger than it is, memory that could be used elsewhere sits idle,
and there is no good number to size limits or requests with.

Meta's TMO ([Transparent Memory Offloading in Datacenters, ASPLOS '22](https://par.nsf.gov/biblio/10332477-tmo-transparent-memory-offloading-datacenters))
solves this by reclaiming a little memory on purpose and backing off as soon as
the workload starts to stall on it. The kernel side of TMO is upstream
(`memory.reclaim`, PSI), so this only needs a small userspace daemon. ebb is that
daemon, applied per leaf cgroup (e.g. each container of a pod).

## How it works

- Walks a cgroup v2 tree and keeps the leaf cgroups with processes in them
  (`populated 1` in `cgroup.events`), optionally filtered with oomd-style globs.
  The tree is rescanned every 30s, so new and removed leaves are picked up.
- Every 6s, for each leaf, it reads `memory.current` and the `some` stall totals of
  `memory.pressure` and `io.pressure`, and computes the share of the period tasks
  were stalled.
- Asks the kernel to reclaim `current × ratio × max(0, 1 − pressure / threshold)`
  bytes through `memory.reclaim` (TMO §3.3, defaults `0.0005` and `0.1%`). Reclaim
  shrinks as memory pressure gets closer to the threshold, and stops at it.
- Skips the period while IO pressure is at or above its threshold, since refaults
  can slow the device without showing up as memory stalls.
- `memory.reclaim` doesn't leave any limit behind, so a workload that grows is never
  throttled by ebb, and stopping it needs no cleanup.
- Single thread, epoll with one timerfd per leaf, cgroup files kept open (one
  `pread`/`pwrite` per file per period), no async runtime.

## Usage

```console
$ ebb-d /sys/fs/cgroup/kubepods.slice
$ ebb-d --match '*/cri-containerd-*.scope' --match '*/*/cri-containerd-*.scope' /sys/fs/cgroup/kubepods.slice
```

Each period logs one line per leaf. A scope with 64MiB of page cache, with a high
ratio (`--reclaim-ratio 0.2 --interval 1`) to make it visible, and the IO gate
holding reclaim back while the scope writes to disk:

```
…/ebb-p.scope current=104792064 pressure=0.0000% io_pressure=3.1225% reclaim=0
…/ebb-p.scope current=118824960 pressure=0.0000% io_pressure=0.0000% reclaim=23764992
…/ebb-p.scope current=93708288 pressure=0.0000% io_pressure=0.0000% reclaim=18741657
```

All options can also be set with environment variables (`EBBD_MATCH`,
`EBBD_MEM_RECLAIM_RATIO`, …), see `ebb-d --help`.

## Requirements

- Linux 5.19+ (`memory.reclaim`), cgroup v2 with PSI enabled.
- Write access to the leaves' `memory.reclaim`: root, or a delegated subtree such as
  a systemd user slice.

## Build

```console
$ nix build
$ cargo build --release
```

Related: [Senpai](https://github.com/facebookincubator/senpai), [oomd](https://github.com/facebookincubator/oomd) (`Senpai` plugin).
