package demo

func Add(a int, b int) int {
	return a + b
}

type Counter struct {
	n int
}

func (c *Counter) Inc() int {
	c.n = Add(c.n, 1)
	return c.n
}

func Run() int {
	return Add(1, 2)
}
