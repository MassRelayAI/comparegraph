import { basename } from "node:path";

export function shout(s) {
  return s.toUpperCase();
}

export class Greeter {
  hello(name) {
    return shout("hello " + basename(name));
  }
}

export function main() {
  return new Greeter().hello("a/b");
}
