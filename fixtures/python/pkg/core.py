def add(a, b):
    return a + b

class Store:
    def __init__(self):
        self.items = []

    def push(self, x):
        self.items.append(x)
        return len(self.items)
