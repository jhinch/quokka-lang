# Test ROMs

Hand-written ROMs that the tests in `../` run in the emulator.

Each `*.asm` is the source for the `*.gb` beside it. The compiled ROMs are checked in so that
running the tests does not need RGBDS. After changing an `*.asm`, rebuild with `./build.sh`
(which does need RGBDS) and commit the `*.gb` along with it.
