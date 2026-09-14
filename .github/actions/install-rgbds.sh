#!/bin/bash

set -e

function main() {
  local os="$1"
  case "$os" in
    ubuntu-*)
      sudo apt-get update
      # sudo apt-get install
      curl -L https://github.com/gbdev/rgbds/releases/download/v1.0.3/rgbds-source.tar.gz > rgbds-source.tar.gz
      mkdir -p rgbds-source
      tar -xf rgbds-source.tar.gz --directory rgbds-source
      (cd rgbds-source; cmake -S . -B build -DCMAKE_BUILD_TYPE=Release)
      (cd rgbds-source; cmake --build build)
      (cd rgbds-source; sudo cmake --install build)
      rm -rf rgbds-source
      ;;
    *)
      echo "$os is not supported by install-rgbds.sh"
      exit 1
      ;;
  esac
}

main "$@"