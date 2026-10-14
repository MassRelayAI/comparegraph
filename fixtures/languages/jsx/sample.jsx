import React from "react";

function title(s) {
  return s.trim();
}

class Panel extends React.Component {
  render() {
    return <div>{title(this.props.text)}</div>;
  }
}

export function Page() {
  return <Panel text=" hi " />;
}
