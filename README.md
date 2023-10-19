# proc-mem-to-prom
Machine process memory usage to prometheus

## Building

To build for multiple arbitrary OS versions:

  cargo build -r --target x86_64-unknown-linux-musl

This makes a release build using musl-c, to make a static binary.

The binary can be found at:

  target/x86_64-unknown-linux-musl/release/proc-mem-to-prom
