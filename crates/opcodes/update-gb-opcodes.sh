#!/bin/bash -e

readonly CRATE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO="gbdev/gb-opcodes"

function main() {
  local revision="$1"
  if [ -z "$revision" ]; then
    echo -e "ERROR: revision not specified"
    exit 1
  fi
  echo "Updating gb-opcodes to $revision..."
  mkdir -p "$CRATE_ROOT/gb-opcodes"
  for file in Opcodes.json LICENSE; do
    curl -sSfL -o "$CRATE_ROOT/gb-opcodes/$file" \
      "https://raw.githubusercontent.com/$REPO/$revision/$file"
    echo "$file"
  done
  echo "$revision" > "$CRATE_ROOT/gb-opcodes/REVISION"
  echo "Done!"
}

main "$@"
