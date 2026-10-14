import kotlin.math.max

fun bump(n: Int): Int {
    return n + 1
}

class Counter {
    private var total = 0

    fun add(n: Int): Int {
        total += bump(n)
        return max(total, 0)
    }
}

fun run(): Int {
    val c = Counter()
    return c.add(2)
}
