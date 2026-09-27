#!/bin/bash

set -e

function main() {
  local os="$1"
  case "$os" in
    ubuntu-*)
      sudo apt-get update
      sudo apt-get install -y cmake bison libpng-dev
      curl -L https://github.com/gbdev/rgbds/releases/download/v1.0.4/rgbds-source.tar.gz > rgbds-source.tar.gz
      tar -xf rgbds-source.tar.gz
      (cd rgbds; cmake -S . -B build -DCMAKE_BUILD_TYPE=Release)
      (cd rgbds; cmake --build build)
      (cd rgbds; sudo cmake --install build)
      rm -rf rgbds
      rm -f rgbds-source.tar.gz
      ;;
    *)
      echo "$os is not supported by install-rgbds.sh"
      exit 1
      ;;
  esac
}

main "$@"