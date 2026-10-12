# Go fixture notes

Go has no inheritance edges by design (struct embedding and implicit interfaces are not modelled), so expected.json has no `extends` or `implements`. Methods are named without the receiver type (`util.go::Inc`). The test file `util_test.go` yields a `calls` edge, a function-level `tests` edge and a file-level `tests` edge by the `_test` convention.
