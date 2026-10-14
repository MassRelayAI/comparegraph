<?php
namespace Demo;

use DateTime;

function bump($n) {
    return $n + 1;
}

class Counter {
    private $total = 0;

    public function add($n) {
        $this->total += bump($n);
        return $this->total;
    }
}

function run() {
    $c = new Counter();
    return $c->add(2);
}
