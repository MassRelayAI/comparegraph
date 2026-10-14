import os


def helper(n):
    return n + 1


class Box:
    def __init__(self, size):
        self.size = size

    def grow(self):
        self.size = helper(self.size)
        return self.size


def main():
    b = Box(2)
    return os.path.join("box", str(b.grow()))
