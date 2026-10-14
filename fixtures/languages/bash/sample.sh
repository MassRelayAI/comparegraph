#!/usr/bin/env bash
source ./lib.sh

bump() {
  echo $(($1 + 1))
}

add() {
  local n
  n=$(bump "$1")
  echo "$n"
}

main() {
  add 2
}

main
