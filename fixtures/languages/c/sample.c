#include <stdio.h>

struct counter {
    int total;
};

static int bump(int n) {
    return n + 1;
}

int add(struct counter *c, int n) {
    c->total += bump(n);
    return c->total;
}

int main(void) {
    struct counter c = {0};
    printf("%d\n", add(&c, 2));
    return 0;
}
