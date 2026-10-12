from .core import add, Store

def main():
    s = Store()
    s.push(add(1, 2))
    return s
