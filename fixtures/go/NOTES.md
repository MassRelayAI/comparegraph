# Go fixture notes

Go has no inheritance edges by design (struct embedding and implicit interfaces are not modelled), so expected.json has no `extends` or `implements`. Methods are named without the receiver type (`util.go::Inc`). The test file `util_test.go` yields a `calls` edge, a function-level `tests` edge and a file-level `tests` edge by the `_test` convention.

## Plan 6 A2: packages, declared receivers
`shapes.go`, `helpers.go` and `store/store.go` exercise the Go call ladder. The package (a directory plus its package clause) is the unit:
`Build` calls `helper()` (another file of the same package), `NewBox()` and `b.Grow()` (`b := NewBox()` gives `b` the declared result type
`*Box`), `store.NewRepo()` through the import path `demo/store` (resolved with the module path in `go.mod`) and `r.Put()` (`r` is the declared
result type of `NewRepo`). `Measure(s Shape)` has no call edge: an interface-typed receiver binds nothing. `Convert` has no call edge: `Box(c)` is a
type conversion.
