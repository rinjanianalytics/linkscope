#!/usr/bin/perl -w
# Reference harness: runs the original MRTG readcfg (+ optional cfgcheck)
# and prints the resulting data structures as canonical JSON.
use strict;
use lib '/root/mrtg/src/lib/mrtg2';
use MRTG_lib;
use JSON::PP;
my ($cfgfile, $mode) = @ARGV;
my (@routers, %cfg, %rcfg, @target);
$SIG{__WARN__} = sub { print STDERR @_ };
eval { readcfg($cfgfile, \@routers, \%cfg, \%rcfg); 1 } or do {
    print JSON::PP->new->canonical->encode({ error => "$@" }); exit 0 };
my %raw = (global => {%cfg}, targets => [@routers], per_target => {});
for my $k (keys %rcfg) { $raw{per_target}{$k} = { %{$rcfg{$k}} } }
my $out = { raw => \%raw };
if (($mode // '') eq 'check') {
    eval { cfgcheck(\@routers, \%cfg, \%rcfg, \@target, {check => 1}); 1 } or do {
        $out->{check_error} = "$@" };
    my @t;
    for my $t (@target) {
        my %c = map { $_ => $t->{$_} } grep { !/snmpoptions/ } keys %$t;
        push @t, \%c;
    }
    $out->{sources} = \@t;
    $out->{expr} = { map { $_ => $rcfg{target}{$_} } keys %{$rcfg{target} || {}} };
    $out->{unique} = { map { $_ => $rcfg{uniqueTarget}{$_} } keys %{$rcfg{uniqueTarget} || {}} };
}
print JSON::PP->new->canonical->encode($out);
