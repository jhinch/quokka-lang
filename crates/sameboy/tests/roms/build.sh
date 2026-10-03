#!/bin/bash -e

# Assembles every *.asm in this directory into a *.gb beside it. Requires RGBDS.

readonly ROMS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

function main() {
  local asm
  for asm in "$ROMS_ROOT"/*.asm; do
    local name="${asm%.asm}"
    echo "Building $(basename "$name").gb..."
    rgbasm -o "$name.o" "$asm"
    rgblink -o "$name.gb" "$name.o"
    rgbfix -v -p 0xFF "$name.gb"
    rm -f "$name.o"
  done
  echo "Done!"
}

main "$@"
