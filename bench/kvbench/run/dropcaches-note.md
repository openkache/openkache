# Dropping page cache on serveroptima1

## Can we drop caches without sudo? NO.

`/proc/sys/vm/drop_caches` is `--w------- root root`, and writing to it
requires `CAP_SYS_ADMIN` (root). Verified on serveroptima1:

```
$ ls -l /proc/sys/vm/drop_caches
--w------- 1 root root 0 /proc/sys/vm/drop_caches
$ echo 1 > /proc/sys/vm/drop_caches
bash: /proc/sys/vm/drop_caches: Permission denied
$ sudo -n true
sudo: a password is required          # no passwordless sudo
```

So the classic `echo 3 > /proc/sys/vm/drop_caches` between runs is
**not available** to us unprivileged.

## Our SSD-forcing mechanism instead: memory.max

`memory_recursiveprot` is enabled and the memory cgroup controller **is**
delegated to the user subtree. Because page cache is charged to
`memory.max` (cache included), capping a workload's cgroup at
`memory.max = M` means that once its resident set + page cache exceeds `M`,
the kernel must evict clean file pages. Any dataset larger than `M` can no
longer be served from page cache, so reads are forced to go to the SSD.

This gives us a per-workload, unprivileged equivalent of a cold cache:
set `M` well below the dataset size and reads will hit `/dev/sda1`. Prove it
with `diskstats.sh sda1` sampled before/after the workload (sectors_read
should climb).

Note: `MemorySwapMax=0` is also set so evicted anonymous pages cannot go to
swap; pressure stays on file cache eviction / SSD reads, not swap.

## cpuset is NOT unprivileged here (why cg-run.sh uses taskset)

Root's `cgroup.subtree_control` delegates only `cpu memory pids` (not
`cpuset`) down to `user.slice`. A `systemd-run --user` unit therefore never
gets a `cpuset.cpus` file, and `-p AllowedCPUs=0-1` is silently ignored
(the process keeps affinity `0-5`). CPU pinning is done with
`taskset -c 0,1` inside the unit instead; memory capping is done by systemd.
