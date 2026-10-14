using System;

namespace Demo
{
    public class Counter
    {
        private int total;

        static int Bump(int n)
        {
            return n + 1;
        }

        public int Add(int n)
        {
            total += Bump(n);
            return total;
        }

        public static int Run()
        {
            var c = new Counter();
            return c.Add(2);
        }
    }
}
