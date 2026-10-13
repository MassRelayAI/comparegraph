package demo

import "demo/store"

type Shape interface{ Area() int }

type Box struct{ w int }

func (b *Box) Area() int { return b.w }

func (b *Box) Grow() { b.Area() }

func NewBox() *Box { return &Box{} }

// Measure takes an interface: which Area runs is unknown, so no edge.
func Measure(s Shape) int { return s.Area() }

// Convert uses a type conversion, which is no call.
func Convert(c Box) int {
	n := Box(c)
	return n.w
}

func Build() {
	b := NewBox()
	b.Grow()
	helper()
	r := store.NewRepo()
	r.Put()
}
