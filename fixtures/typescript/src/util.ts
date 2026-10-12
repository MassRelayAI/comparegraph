export function add(a: number, b: number): number { return a + b; }
export function mul(a: number, b: number): number { return a * b; }
export class Counter {
  private n = 0;
  inc(): number { this.n = add(this.n, 1); return this.n; }
}
export class Base {
  reset(): number { return 0; }
}
export class Sub extends Base {
  bump(): number { return add(this.reset(), 1); }
}
