import { join } from "path";

export interface Shape {
  area(): number;
}

export class Square implements Shape {
  constructor(private side: number) {}
  area(): number {
    return scale(this.side) * this.side;
  }
}

export function scale(n: number): number {
  return n + 1;
}

export function describe(s: Shape): string {
  return join("area", String(s.area()));
}
