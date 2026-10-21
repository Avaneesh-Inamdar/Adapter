"""Cross-link actual Mach-O test executables with LLVM, without an Apple SDK.
These are deterministic compatibility probes, not macOS-reference validation.
"""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/generated"
LLVM = Path(os.environ.get("ADAPTER_LLVM", r"C:\Program Files\LLVM\bin"))

def tool(name):
    p = LLVM / (name + ".exe")
    return str(p) if p.is_file() else shutil.which(name) or name

def run(*args):
    result=subprocess.run([str(a) for a in args], capture_output=True, text=True)
    if result.returncode: raise RuntimeError(f"command failed ({result.returncode}): {result.args}\n{result.stdout}\n{result.stderr}")

def build():
    OUT.mkdir(parents=True, exist_ok=True)
    exports = {
        "System": ["puts", "strlen", "malloc", "free", "memcpy", "memcmp", "__cxa_atexit", "__cxa_finalize", "atexit", "_tlv_bootstrap", "pthread_create", "pthread_join", "pthread_self", "pthread_equal", "pthread_key_create", "pthread_key_delete", "pthread_getspecific", "pthread_setspecific", "pthread_once", "__error", "deliberately_missing", "_DefaultRuneLocale", "__stdinp", "__stdoutp", "__stderrp", "environ", "setlocale", "getenv", "setenv", "unsetenv", "__maskrune", "printf", "fprintf", "snprintf", "vsnprintf", "sscanf", "vsscanf", "strcmp", "fread", "fwrite", "getc", "fputc", "fputs", "feof", "ferror", "fflush", "fileno", "fclose", "clearerr", "__srget", "__swbuf", "nexttoward", "exit"],
        "CoreFoundation": ["CFStringCreateWithCString", "CFStringGetLength", "CFStringGetCString", "CFRetain", "CFRelease"],
        "objc": ["objc_msgSend", "_objc_empty_cache"],
    }
    versions = ["-arch", "x86_64", "-platform_version", "macos", "10.13", "10.13"]
    exports["System"] += ["fopen$DARWIN_EXTSN", "fdopen$DARWIN_EXTSN", "stat$INODE64", "fstat$INODE64", "realpath$DARWIN_EXTSN", "open", "close", "pathconf", "dirname", "fgets", "timegm", "time", "gmtime_r", "gettimeofday", "strftime", "strptime", "memset"]
    for library, symbols in exports.items():
        asm = OUT / (library + ".s")
        # dyld's binder name is already undecorated in the Mach-O ABI.
        names = ["_" + s for s in symbols] + (["dyld_stub_binder"] if library == "System" else [])
        asm.write_text(".text\n" + "\n".join(f".globl {s}\n{s}:\nret" for s in names) + "\n")
        run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-c", asm, "-o", OUT / (library + ".o"))
        install = {"System":"/usr/lib/libSystem.B.dylib","CoreFoundation":"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation","objc":"/usr/lib/libobjc.A.dylib"}[library]
        run(tool("ld64.lld"), *versions, "-dylib", "-install_name", install, "-o", OUT / ("lib" + library + ".dylib"), OUT / (library + ".o"))
    for name in ["return37", "hello", "corefoundation", "unknown", "loop", "crash", "abi", "objc", "tls", "exit_callbacks", "thread_state", "libc", "file_time"]:
        extension=".m" if name=="objc" else ".c"
        run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-O0", "-ffreestanding", "-fno-stack-protector", "-fno-asynchronous-unwind-tables", "-c", ROOT / ("tests/fixtures/" + name + extension), "-o", OUT / (name + ".o"))
        libraries = [] if name == "return37" else ["-L", OUT, "-lSystem"]
        if name == "corefoundation": libraries += ["-lCoreFoundation"]
        if name == "objc": libraries += ["-lobjc"]
        run(tool("ld64.lld"), *versions, "-no_fixup_chains", "-no_uuid", "-e", "_main", "-o", OUT / name, OUT / (name + ".o"), *libraries)
        if name in ["hello","corefoundation","tls","thread_state","libc"]:
            run(tool("ld64.lld"),"-arch","x86_64","-platform_version","macos","11.0","11.0","-fixup_chains","-no_uuid","-e","_main","-o",OUT/(name+"-chained"),OUT/(name+".o"),*libraries)
    # A local transitive dylib graph, resolved without ever executing it.
    helper=OUT / "helper.s"
    helper.write_text(".text\n.globl _helper\n_helper:\nmovl $7, %eax\nret\n")
    run(tool("clang"),"-target","x86_64-apple-macos10.13","-c",helper,"-o",OUT/"helper.o")
    run(tool("ld64.lld"),*versions,"-dylib","-install_name","@rpath/libhelper.dylib","-o",OUT/"libhelper.dylib",OUT/"helper.o")
    app=OUT / "local.c"
    app.write_text("extern int helper(void); int main(void) { return helper(); }\n")
    run(tool("clang"),"-target","x86_64-apple-macos10.13","-c",app,"-o",OUT/"local.o")
    run(tool("ld64.lld"),*versions,"-no_fixup_chains","-rpath","@loader_path","-o",OUT/"local",OUT/"local.o","-L",OUT,"-lhelper","-lSystem")
    # A real arm64 object and a universal wrapper exercise architecture selection.
    run(tool("clang"), "-target", "arm64-apple-macos11", "-c", ROOT / "tests/fixtures/return37.c", "-o", OUT / "arm64.o")
    x64, arm = (OUT / "return37").read_bytes(), (OUT / "arm64.o").read_bytes()
    armoff = (4096 + len(x64) + 4095) & ~4095
    fat = struct.pack(">II", 0xcafebabe, 2) + struct.pack(">IIIII",0x1000007,3,4096,len(x64),12) + struct.pack(">IIIII",0x100000c,0,armoff,len(arm),12)
    (OUT / "universal").write_bytes(fat.ljust(4096,b"\0")+x64+b"\0"*(armoff-4096-len(x64))+arm)
    bundle = OUT / "Hello.app/Contents"
    (bundle / "MacOS").mkdir(parents=True,exist_ok=True)
    (bundle / "Resources").mkdir(exist_ok=True)
    (bundle / "Resources/marker.txt").write_text("bundle resource fixture\n")
    (bundle / "Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable":"hello","CFBundleIdentifier":"test.adapter.hello"}))
    shutil.copyfile(OUT / "hello",bundle / "MacOS/hello")
    from build_dylib_fixtures import build as build_dylibs
    build_dylibs()
    from build_tls_abi import build as build_tls_abi
    build_tls_abi()
    manifest={p.relative_to(OUT).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in OUT.rglob('*') if p.is_file() and p.suffix not in [".json",".log",".txt"] and 'reference' not in p.relative_to(OUT).parts}
    (OUT / "manifest.json").write_text(json.dumps(manifest,indent=2)+"\n")
    print("Built Mach-O fixtures with",tool("clang"),"and",tool("ld64.lld"))

if __name__ == "__main__": build()
