import Foundation

func bump(_ n: Int) -> Int {
    return n + 1
}

class Counter {
    var total = 0

    func add(_ n: Int) -> Int {
        total += bump(n)
        return total
    }
}

func run() -> Int {
    let c = Counter()
    return c.add(2)
}
