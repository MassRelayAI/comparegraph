import 'dart:math';

int bump(int n) {
  return n + 1;
}

class Counter {
  int total = 0;

  int add(int n) {
    total += bump(n);
    return max(total, 0);
  }
}

int run() {
  var c = Counter();
  return c.add(2);
}
