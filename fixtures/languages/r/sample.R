library(stats)

bump <- function(n) {
  n + 1
}

add <- function(total, n) {
  total + bump(n)
}

run <- function() {
  add(0, 2)
}
