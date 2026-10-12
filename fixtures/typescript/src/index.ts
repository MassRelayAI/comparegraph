import { add, Counter } from "./util";
export function main(): number {
  const c = new Counter();
  c.inc();
  return add(1, 2);
}
