# proc-mem-to-prom
Machine process memory usage to prometheus

## Building

To build for multiple arbitrary OS versions:

  cargo build -r --target x86_64-unknown-linux-musl

This makes a release build using musl-c, to make a static binary.

The binary can be found at:

  target/x86_64-unknown-linux-musl/release/proc-mem-to-prom

## Installing

To install this as a system service, copy the binary to `/usr/sbin/`
and copy the systemd service file to `/etc/systemd/system/`.

Then enable the service with

  systemctl daemon-reload
  systemctl enable proc-mem-to-prom.service
