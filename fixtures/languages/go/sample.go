package sample

import "strings"

type Counter struct {
	total int
}

func bump(n int) int {
	return n + 1
}

func (c *Counter) Add(n int) int {
	c.total += bump(n)
	return c.total
}

func Run() string {
	c := &Counter{}
	c.Add(2)
	return strings.ToUpper("done")
}
