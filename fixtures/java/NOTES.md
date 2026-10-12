# Java fixture notes

Two different methods are both called `area` (`Shape.area`, `Square.area`, in different files). `SquareTest.checkArea(Square s)` calls `s.area()`; our layer resolves it to `Square.area` from the declared parameter type (`typed_receiver_method` in resolve.rs). No type inference: a receiver whose type is not declared in the caller's own lines (a field, a `var`, a chained call) still falls back to a name guess at 0.5.

Vendored-extractor miss: Java object creation (`new Square()`) is not reported as a call site, so there is no `calls -> Square` edge for constructors; the fixture passes `Square` as a parameter so it does not depend on one. Same-package types need no import, so there are no `imports` edges.
