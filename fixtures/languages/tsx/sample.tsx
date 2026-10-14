import React from "react";

export function label(n: number): string {
  return "item " + n;
}

export class Badge extends React.Component<{ n: number }> {
  render() {
    return <span>{label(this.props.n)}</span>;
  }
}

export function App() {
  return <Badge n={3} />;
}
