# arctic

[![Crates.io](https://img.shields.io/crates/v/arctic-map.svg)](https://crates.io/crates/arctic-map)
[![Docs](https://docs.rs/arctic-map/badge.svg)](https://docs.rs/arctic-map/latest/arctic)

{{readme}}

## Benchmarks

Here are some [YCSB workload](https://github.com/brianfrankcooper/YCSB/wiki/Core-Workloads)
scalability results, measured and plotted using [index-bench](https://github.com/nwtnni/index-bench).
We insert 100M random keys of different types with u64 values,
and then (for non-load workloads) record the throughput of 100M operations,
using the default Zipfian skewness of 0.99 (top 10 keys receive ~17% of requests).

These were run on a [Chameleon](https://www.chameleoncloud.org/)
compute_icelake_r650 instance ([example](https://www.chameleoncloud.org/hardware/node/sites/tacc/clusters/chameleon/nodes/dde004bf-b99b-4c0a-b2d4-d5537378626a/)) with 80 physical cores.
For reproducibility, we pin threads to cores, interleave memory across NUMA sockets,
disable hyper-threading and turbo-boost, and set the CPU scaling governor to performance.

As for baselines,
[art](https://dl.acm.org/doi/10.1109/ICDE.2013.6544812)
(using [ROWEX](https://dl.acm.org/doi/10.1145/2933349.2933352)),
[fb_tree](https://dl.acm.org/doi/10.14778/3725688.3725691),
[hot](https://dl.acm.org/doi/10.1145/3183713.3196896),
and [wormhole](https://dl.acm.org/doi/10.1145/3302424.3303955)
are research systems written in C/C++ and integrated (hackily)
via [cxx](https://github.com/dtolnay/cxx). The remainder
are published Rust crates (including masstree, which is the
[Rust port](https://github.com/consistent-milk12/masstree)
rather than the [original](https://github.com/kohler/masstree-beta)).

### YCSB-Load (100% insert)

![Plot of YCSB-Load results](img/load.png)

### YCSB-A (50% read, 50% update)

![Plot of YCSB-A results](img/a.png)

### YCSB-B (95% read, 5% update)

![Plot of YCSB-B results](img/b.png)

### YCSB-C (100% read)

![Plot of YCSB-C results](img/c.png)

### YCSB-D (95% read, 5% insert, skewed toward latest)

![Plot of YCSB-D results](img/d.png)

### YCSB-E (95% scan, 5% insert)

![Plot of YCSB-E results](img/e.png)

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
