require "json"

def bump(n)
  n + 1
end

class Counter
  def initialize
    @total = 0
  end

  def add(n)
    @total += bump(n)
  end
end

def run
  c = Counter.new
  JSON.generate(total: c.add(2))
end
