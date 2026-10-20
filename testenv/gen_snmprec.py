#!/usr/bin/env python3
"""Generate snmpsim .snmprec device files for linkscope tests and benchmarks.

Each device gets a MIB-II system group, ifTable, ifXTable (64-bit counters,
ifName, ifAlias, ifHighSpeed) and ipAdEntIfIndex. Counters are either static
(--static, for parity tests where two pollers must read identical values) or
advance with time (snmpsim 'numeric' variation, for benchmarks).

Usage: gen_snmprec.py OUTDIR --devices N --ifaces M [--static]
The community string for device k is "dev<k>".
"""
import argparse, os

def oid_key(oid):
    return tuple(int(x) for x in oid.split('.'))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('outdir')
    ap.add_argument('--devices', type=int, default=4)
    ap.add_argument('--ifaces', type=int, default=8)
    ap.add_argument('--static', action='store_true')
    ap.add_argument('--elapsed', type=int, default=0,
                    help='with --static: advance counters as if this many seconds passed')
    a = ap.parse_args()
    os.makedirs(a.outdir, exist_ok=True)
    for d in range(a.devices):
        rec = []
        add = lambda oid, typ, val: rec.append((oid, typ, val))
        add('1.3.6.1.2.1.1.1.0', '4', f'MockOS 1.0 router dev{d}')
        add('1.3.6.1.2.1.1.2.0', '6', '1.3.6.1.4.1.9.1.1')
        add('1.3.6.1.2.1.1.3.0', '67', str(100000 + d))
        add('1.3.6.1.2.1.1.4.0', '4', 'noc@example.net')
        add('1.3.6.1.2.1.1.5.0', '4', f'dev{d}.lab')
        add('1.3.6.1.2.1.1.6.0', '4', 'Rack 1')
        add('1.3.6.1.2.1.2.1.0', '2', str(a.ifaces))
        for i in range(1, a.ifaces + 1):
            base = '1.3.6.1.2.1.2.2.1'
            down = (i % 7 == 0)
            add(f'{base}.1.{i}', '2', str(i))
            add(f'{base}.2.{i}', '4', f'GigabitEthernet0/{i}')
            add(f'{base}.3.{i}', '2', '6')
            add(f'{base}.4.{i}', '2', '1500')
            add(f'{base}.5.{i}', '66', '1000000000')
            add(f'{base}.6.{i}', '4x', f'0200{d:04x}{i:04x}')
            add(f'{base}.7.{i}', '2', '1')
            add(f'{base}.8.{i}', '2', '2' if down else '1')
            seed_in = (d * 7919 + i * 104729) % 4000000000
            if i == a.ifaces:
                seed_in = 2**32 - 700000  # wraps within a few cycles
            seed_out = (d * 6007 + i * 7727) % 4000000000
            if a.static:
                add(f'{base}.10.{i}', '65', str((seed_in + 1000 * i * a.elapsed) % 2**32))
                add(f'{base}.16.{i}', '65', str((seed_out + 500 * i * a.elapsed) % 2**32))
            else:
                add(f'{base}.10.{i}', '65:numeric', f'rate={1000 * i},initial={seed_in},wrap=1')
                add(f'{base}.16.{i}', '65:numeric', f'rate={500 * i},initial={seed_out},wrap=1')
            x = '1.3.6.1.2.1.31.1.1.1'
            add(f'{x}.1.{i}', '4', f'Gi0/{i}')
            if a.static:
                add(f'{x}.6.{i}', '70', str(seed_in + 10**12 + 1000 * i * a.elapsed))
                add(f'{x}.10.{i}', '70', str(seed_out + 10**12 + 500 * i * a.elapsed))
            else:
                add(f'{x}.6.{i}', '70:numeric', f'rate={1000 * i},initial={seed_in + 10**12}')
                add(f'{x}.10.{i}', '70:numeric', f'rate={500 * i},initial={seed_out + 10**12}')
            add(f'{x}.15.{i}', '66', '1000')
            add(f'{x}.18.{i}', '4', f'uplink {d}-{i}')
            add(f'1.3.6.1.2.1.4.20.1.2.10.{d % 256}.{i}.1', '2', str(i))
        rec.sort(key=lambda r: oid_key(r[0]))
        with open(os.path.join(a.outdir, f'dev{d}.snmprec'), 'w') as f:
            for oid, typ, val in rec:
                f.write(f'{oid}|{typ}|{val}\n')

if __name__ == '__main__':
    main()
