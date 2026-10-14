const path = require("path");

function double(n) {
  return n * 2;
}

class Counter {
  constructor() {
    this.total = 0;
  }
  add(n) {
    this.total += double(n);
    return this.total;
  }
}

function run() {
  const c = new Counter();
  return path.join("sum", String(c.add(2)));
}

module.exports = { run };
