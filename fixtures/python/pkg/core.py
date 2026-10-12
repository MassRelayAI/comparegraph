def add(a, b):
    return a + b

class Store:
    def __init__(self):
        self.items = []

    def push(self, x):
        self.items.append(x)
        return len(self.items)


class Base:
    def reset(self):
        return 0


class Child(Base):
    def bump(self):
        return add(self.reset(), 1)
