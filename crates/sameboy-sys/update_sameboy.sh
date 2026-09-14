#!/bin/bash -e

readonly SYS_CRATE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

function main() {
  local tag="$1"
  if [ -z "$tag" ]; then
    echo -e "ERROR: tag not specified"
    exit 1
  fi
  echo "Updating submodule to $tag..."
  (cd "$SYS_CRATE_ROOT/SameBoy"; git checkout "$tag")
  echo "Done!"
  echo "Building boot ROMs..."
  (cd "$SYS_CRATE_ROOT/SameBoy"; make bootroms)
  echo "Done!"
  echo "Copying bootroms.."
  rm -vf "$SYS_CRATE_ROOT/bootroms"/*.bin
  mkdir -p "$SYS_CRATE_ROOT/bootroms"
  cp -v "$SYS_CRATE_ROOT/SameBoy/build/bin/BootROMs"/*.bin "$SYS_CRATE_ROOT/bootroms/"
  echo "Done!"
}

main "$@"