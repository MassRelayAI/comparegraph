import scala.collection.mutable

object Util {
  def bump(n: Int): Int = n + 1
}

class Counter {
  private var total = 0

  def add(n: Int): Int = {
    total += Util.bump(n)
    total
  }
}

object Main {
  def run(): Int = {
    val c = new Counter()
    val m = mutable.Map[Int, Int]()
    c.add(2) + m.size
  }
}
