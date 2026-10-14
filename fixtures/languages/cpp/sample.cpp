#include <string>

int bump(int n) {
    return n + 1;
}

class Counter {
public:
    int add(int n) {
        total += bump(n);
        return total;
    }

private:
    int total = 0;
};

int run() {
    Counter c;
    return c.add(2);
}
