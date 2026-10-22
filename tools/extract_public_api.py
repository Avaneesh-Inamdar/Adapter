"""Build docs/progress/public-api.json from Apple's open-source public headers.

The observed-import chart only counts symbols that the inspected binaries happen
to import. This catalog lists the whole public surface of each subsystem Adapter
implements, taken from pinned commits of apple-oss-distributions repositories.
Headers are downloaded as text, hashed and parsed with regular expressions; they
are never compiled or executed. Re-running with the pinned commits must produce
the same hashes, which is how upstream drift is detected.

    python tools/extract_public_api.py            # fetch and rewrite the catalog
    python tools/extract_public_api.py --check    # fetch and compare, no write
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / 'docs/progress/public-api.json'
HOST = 'https://raw.githubusercontent.com/apple-oss-distributions'
REPOS = {
    'libdispatch': '2361ffb78a76f7ee488cd052eb0bc5c767118bf9',
    'libclosure': '3668b0837f47be3cc1c404fb5e360f4ff178ca13',
    'libpthread': '42d026df5b07825070f60134b980a1ec2552dfee',
    'Libc': '71bbe350ab79eef58113991d817ccc6165061a64',
    'xnu': 'f6217f891ac0bb64f3d375211650a4c1ff8ca1ea',
}
# (group id, label, repo, path, kind). Introspection, base.h and dispatch.h
# declare no callable API, and the introspection hooks are SPI.
GROUPS = [
    ('dispatch-queues', 'libdispatch queues', 'libdispatch', 'dispatch/queue.h', 'dispatch'),
    ('dispatch-objects', 'libdispatch objects', 'libdispatch', 'dispatch/object.h', 'dispatch'),
    ('dispatch-groups', 'libdispatch groups', 'libdispatch', 'dispatch/group.h', 'dispatch'),
    ('dispatch-semaphore', 'libdispatch semaphores', 'libdispatch', 'dispatch/semaphore.h', 'dispatch'),
    ('dispatch-once-time', 'libdispatch once and time', 'libdispatch',
     ['dispatch/once.h', 'dispatch/time.h'], 'dispatch'),
    ('dispatch-blocks', 'libdispatch block objects', 'libdispatch', 'dispatch/block.h', 'dispatch'),
    ('dispatch-sources', 'libdispatch sources', 'libdispatch', 'dispatch/source.h', 'dispatch'),
    ('dispatch-data', 'libdispatch data', 'libdispatch', 'dispatch/data.h', 'dispatch'),
    ('dispatch-io', 'libdispatch I/O', 'libdispatch', 'dispatch/io.h', 'dispatch'),
    ('dispatch-workloop', 'libdispatch workloops', 'libdispatch', 'dispatch/workloop.h', 'dispatch'),
    ('block-runtime', 'Block runtime', 'libclosure', 'Block.h', 'block'),
    ('signals', 'POSIX signals', 'Libc',
     ['include/signal.h', ('xnu', 'bsd/sys/signal.h')], 'c'),
    ('pthread-qos', 'pthread QoS', 'libpthread',
     ['include/pthread/qos.h', 'include/sys/qos.h'], 'c'),
    ('mach-vm', 'Mach VM routines', 'xnu', 'osfmk/mach/mach_vm.defs', 'mig'),
    ('sysctl-calls', 'sysctl calls', 'xnu', 'bsd/sys/sysctl.h', 'names:sysctl,sysctlbyname,sysctlnametomib'),
    ('sysctl-names', 'sysctl names', 'xnu', 'bsd/sys/sysctl.h', 'sysctl'),
    ('libproc', 'libproc', 'xnu', 'libsyscall/wrappers/libproc/libproc.h', 'c'),
    ('spawn', 'posix_spawn', 'xnu', 'libsyscall/wrappers/spawn/spawn.h', 'c'),
    ('process-control', 'fork, exec and wait', 'Libc',
     ['include/unistd.h', ('xnu', 'bsd/sys/wait.h')],
     'names:fork,vfork,execl,execle,execlp,execv,execvP,execvp,getppid,getpgrp,getpgid,setpgid,'
     'setsid,wait,wait3,wait4,waitpid,waitid'),
]


# Routines the Mach definitions compile only for the kernel or private clients.
PRIVATE = {'mig': ('mach_vm_deferred_reclamation', 'mach_vm_update_pointers',
                   'mach_vm_range_create'),
           'c': ('siginfo_user_to_user',)}


def fetch(repo, path):
    url = f'{HOST}/{repo}/{REPOS[repo]}/{path}'
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


def strip_comments(text):
    text = re.sub(r'/\*.*?\*/', ' ', text, flags=re.S)
    text = re.sub(r'//[^\n]*', ' ', text)
    lines, continued = [], False
    for line in text.split('\n'):
        if continued or line.lstrip().startswith('#'):
            continued = line.rstrip().endswith('\\')
            continue
        lines.append(line)
    text = '\n'.join(lines)
    # extern "C" wrappers would otherwise hide every declaration inside them.
    return re.sub(r'extern\s+"C"\s*\{', ' ', text)


def statements(text):
    depth, buffer = 0, []
    for char in strip_comments(text):
        if char == '{':
            depth += 1
        elif char == '}':
            depth = max(0, depth - 1)
            if depth == 0:
                buffer.append(' ;')
            continue
        if depth == 0:
            buffer.append(char)
            if char == ';':
                yield ' '.join(''.join(buffer).split())
                buffer = []


def attribute_free(statement):
    # __API_AVAILABLE(macos(10.10)), __DARWIN_ALIAS(kill) and similar.
    previous = None
    while previous != statement:
        previous = statement
        statement = re.sub(r'\b__\w+\s*\((?:[^()]|\([^()]*\))*\)', ' ', statement)
    return statement


def dispatch_symbols(text):
    functions, data = set(), set()
    inline = set(re.findall(r'DISPATCH_INLINE[^;{]*?\b(dispatch_\w+)\s*\(', strip_comments(text)))
    for statement in statements(text):
        if 'DISPATCH_EXPORT' not in statement or statement.startswith('typedef'):
            continue
        match = re.search(r'\b(dispatch_[a-z]\w*)\s*\(', statement)
        if match:
            functions.add(match[1])
            continue
        declared = re.search(r'\b(_dispatch_\w+)\s*(?:\[[^\]]*\])?\s*;', statement)
        if declared:
            data.add(declared[1])
    functions -= inline
    # Macro-declared data: source types and data destructors.
    for name in re.findall(r'DISPATCH_SOURCE_TYPE_DECL(?:_SWIFT)?\((\w+)', strip_comments(text)):
        data.add('_dispatch_source_type_' + name)
    for name in re.findall(r'^\s*DISPATCH_DATA_DESTRUCTOR_TYPE_DECL\((\w+)\)', strip_comments(text), re.M):
        data.add('_dispatch_data_destructor_' + name)
    return functions | data


def block_symbols(text):
    names = set()
    for statement in statements(text):
        if 'BLOCK_EXPORT' in statement and not statement.startswith('typedef'):
            match = re.search(r'\b(_?Block_\w+)\s*\(', statement) or \
                re.search(r'\b(_NSConcrete\w+)\s*\[', statement)
            if match:
                names.add(match[1])
    return names


def c_symbols(text):
    names = set()
    for statement in statements(text):
        statement = attribute_free(statement)
        if statement.startswith(('typedef', 'struct ', 'union ', 'enum ')) or '(' not in statement:
            continue
        # void (*signal(int, void (*)(int)))(int);
        match = re.search(r'\(\s*\*\s*(?:_\w+\s+)?([a-z_]\w*)\s*\(', statement)
        if not match:
            match = re.search(r'\b([a-z_][a-z0-9_]*)\s*\(', statement)
        if match and not match[1].startswith('_Pragma') and match[1] not in ('defined', 'sizeof'):
            names.add(match[1])
    return names


def mig_symbols(text):
    names = set()
    for match in re.finditer(r'^\s*(?:simple)?routine\s+((?:\w+\()*)(\w+)', text, re.M):
        if not match[2].startswith('_'):
            names.add(match[2])
    return names


def sysctl_names(text):
    """Statically numbered sysctl names plus the selectors the header documents.

    Returns {name: mib or None}. The CTL_*_NAMES tables give each name's second MIB
    component by position; `dummy` slots and interior nodes are skipped.
    """
    # The documented dynamic selectors live in a block comment, so read them first.
    documented = re.findall(r'^\s*\*\s+(hw\.[a-z0-9_.]+)\s+-', text, re.M)
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S).replace('\\\n', '\n')

    def table(macro):
        match = re.search(r'#define\s+' + macro + r'\s*\{(.*?)\n\}', text, re.S)
        if not match:
            raise RuntimeError(f'{macro} not found')
        return re.findall(r'\{\s*(?:"([^"]*)"|0)\s*,\s*(\w+)\s*\}', match[1])

    top = [name for name, _ in table('CTL_NAMES')]
    found = {}
    for macro, prefix in [('CTL_KERN_NAMES', 'kern'), ('CTL_VM_NAMES', 'vm'), ('CTL_HW_NAMES', 'hw'),
                          ('CTL_USER_NAMES', 'user')]:
        for index, (name, kind) in enumerate(table(macro)):
            if name and name != 'dummy' and kind != 'CTLTYPE_NODE':
                found[f'{prefix}.{name}'] = [top.index(prefix), index]
    for name in documented:
        found.setdefault(name, None)
    return found


def explicit_names(kind, text):
    wanted = kind.split(':', 1)[1].split(',')
    return {name for name in wanted if re.search(r'\b' + re.escape(name) + r'\s*\(', text)}


def parse(kind, text):
    if kind.startswith('names:'):
        return explicit_names(kind, text)
    if kind == 'sysctl':
        return sysctl_names(text)
    return {'dispatch': dispatch_symbols, 'block': block_symbols, 'c': c_symbols,
            'mig': mig_symbols}[kind](text)


def build():
    cache, groups = {}, []
    for gid, label, repo, paths, kind in GROUPS:
        paths = [paths] if isinstance(paths, str) else paths
        symbols = set()
        files = []
        for path in paths:
            source_repo, path = path if isinstance(path, tuple) else (repo, path)
            if (source_repo, path) not in cache:
                cache[(source_repo, path)] = fetch(source_repo, path)
            data = cache[(source_repo, path)]
            files.append({'repository': source_repo, 'commit': REPOS[source_repo], 'path': path,
                          'sha256': hashlib.sha256(data).hexdigest()})
            symbols |= set(parse(kind, data.decode('utf-8', errors='replace')))
            if kind == 'sysctl':
                mibs = parse(kind, data.decode('utf-8', errors='replace'))
        symbols = {name for name in symbols if not name.startswith(PRIVATE.get(kind, ()))}
        if kind.startswith('names:') and symbols != set(kind.split(':', 1)[1].split(',')):
            raise RuntimeError(f'{gid}: not every listed name was found in its headers')
        if not symbols:
            raise RuntimeError(f'no symbols parsed for {gid}')
        group = {'id': gid, 'name': label, 'repository': repo, 'commit': REPOS[repo], 'files': files}
        if kind == 'sysctl':
            # sysctl names are not C symbols: no leading underscore, and a separate registry.
            group['registry'] = 'sysctl'
            group['symbols'] = sorted(symbols)
            group['mibs'] = {name: mibs[name] for name in sorted(mibs) if mibs[name]}
        else:
            # Mach-O symbol names carry the C leading underscore.
            group['symbols'] = sorted('_' + name for name in symbols)
        groups.append(group)
    return {'schema_version': 1,
            'note': 'Public-header symbol inventory. Counts what a subsystem exports, not what '
                    'programs import; a symbol with an adapter may implement only part of its contract.',
            'groups': groups}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    catalog = build()
    text = json.dumps(catalog, indent=2) + '\n'
    if args.check:
        if CATALOG.read_text() != text:
            raise SystemExit('public-api.json differs from the pinned upstream headers')
        print('public-api.json matches the pinned upstream headers')
        return
    CATALOG.write_text(text, encoding='utf-8', newline='\n')
    for group in catalog['groups']:
        print(f"{group['name']}: {len(group['symbols'])} symbols")


if __name__ == '__main__':
    main()
