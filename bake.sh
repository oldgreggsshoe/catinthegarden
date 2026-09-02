#!/bin/bash
  TMPDIR=/home/dad/catingard/tmp-rust \
  .target-baker-triple/release/catinthegarden-baker \
    --output assets/outmaps/test-planet \
    --procedural-terrain \
    --mountain-coverage \
    --width 4096 \
    --height 2048 \
    --dense-level 4 \
    --max-level 18 \
    --erosion-iterations 2048 \
    --seed 0xEA272026
