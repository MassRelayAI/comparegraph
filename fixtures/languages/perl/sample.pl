use strict;
use List::Util qw(sum);

package Counter;

sub new {
    my ($class) = @_;
    return bless { total => 0 }, $class;
}

sub bump {
    my ($n) = @_;
    return $n + 1;
}

sub add {
    my ($self, $n) = @_;
    $self->{total} += bump($n);
    return $self->{total};
}

package main;

sub run {
    my $c = Counter->new;
    return sum($c->add(2), 1);
}
