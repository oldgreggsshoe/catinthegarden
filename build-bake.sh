#!/bin/bash
  TMPDIR=/home/dad/catingard/tmp-rust \
  CARGO_TARGET_DIR=/home/dad/catingard/.target-baker-triple \
  cargo build --release -p catinthegarden-baker
