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
        "System": ["puts", "strlen", "malloc", "free", "memcpy", "memcmp", "__cxa_atexit", "__cxa_finalize", "atexit", "_tlv_bootstrap", "pthread_create", "pthread_join", "pthread_self", "pthread_equal", "pthread_key_create", "pthread_key_delete", "pthread_getspecific", "pthread_setspecific", "pthread_once", "__error", "deliberately_missing", "_DefaultRuneLocale", "__stdinp", "__stdoutp", "__stderrp", "environ", "setlocale", "getenv", "setenv", "unsetenv", "__maskrune", "printf", "fprintf", "snprintf", "vsnprintf", "sscanf", "vsscanf", "strcmp", "fread", "fwrite", "getc", "fputc", "fputs", "feof", "ferror", "fflush", "fileno", "fclose", "clearerr", "__srget", "__swbuf", "nexttoward", "exit", "_NSGetExecutablePath", "dlopen", "dlsym", "dlclose", "dlerror", "_dyld_image_count", "mach_port_allocate", "mach_port_deallocate", "mach_port_mod_refs", "mach_port_destroy", "mach_port_insert_right", "mach_msg", "bootstrap_check_in", "bootstrap_look_up", "_Block_copy", "_Block_release"],
        "CoreFoundation": ["CFStringCreateWithCString", "CFStringGetLength", "CFStringGetCString", "CFRetain", "CFRelease"],
        "objc": ["objc_msgSend", "_objc_empty_cache"],
        "xpc": ["xpc_retain", "xpc_release", "xpc_get_type", "xpc_equal", "xpc_hash",
            "xpc_dictionary_create", "xpc_dictionary_create_empty", "xpc_dictionary_create_reply",
            "xpc_dictionary_set_value", "xpc_dictionary_get_value", "xpc_dictionary_get_count",
            "xpc_dictionary_set_bool", "xpc_dictionary_set_int64", "xpc_dictionary_set_uint64",
            "xpc_dictionary_set_double", "xpc_dictionary_set_string", "xpc_dictionary_get_bool",
            "xpc_dictionary_get_int64", "xpc_dictionary_get_double", "xpc_dictionary_get_string",
            "xpc_dictionary_get_array", "xpc_dictionary_get_dictionary",
            "xpc_dictionary_get_remote_connection", "xpc_array_create", "xpc_array_append_value",
            "xpc_array_get_count", "xpc_array_get_value", "xpc_array_get_int64",
            "xpc_array_get_string", "xpc_array_set_string", "xpc_array_set_int64",
            "xpc_string_create", "xpc_string_get_string_ptr",
            "xpc_int64_create", "xpc_int64_get_value", "xpc_bool_create", "xpc_bool_get_value",
            "xpc_connection_create_mach_service", "xpc_connection_set_event_handler",
            "xpc_connection_resume", "xpc_connection_cancel", "xpc_connection_send_message",
            "xpc_connection_send_message_with_reply_sync", "xpc_connection_get_name"],
    }
    versions = ["-arch", "x86_64", "-platform_version", "macos", "10.13", "10.13"]
    exports["System"] += ["fopen$DARWIN_EXTSN", "fdopen$DARWIN_EXTSN", "stat$INODE64", "fstat$INODE64", "realpath$DARWIN_EXTSN", "open", "close", "pathconf", "dirname", "fgets", "timegm", "time", "gmtime_r", "gettimeofday", "strftime", "strptime", "memset", "write", "usleep", "_tlv_atexit", "pthread_mutex_init", "pthread_mutex_lock", "pthread_mutex_trylock", "pthread_mutex_unlock", "pthread_mutex_destroy", "pthread_cond_init", "pthread_cond_destroy", "pthread_cond_wait", "pthread_cond_timedwait", "pthread_cond_signal", "pthread_cond_broadcast", "pthread_condattr_init", "pthread_condattr_destroy", "pthread_condattr_getpshared", "pthread_condattr_setpshared"]
    for library, symbols in exports.items():
        asm = OUT / (library + ".s")
        # dyld's binder name is already undecorated in the Mach-O ABI.
        data = {
            "System": {"mach_task_self_": 0x103, "bootstrap_port": 0x203, "_NSConcreteStackBlock": 0, "_NSConcreteGlobalBlock": 0},
            "xpc": {name: 0 for name in ["_xpc_type_connection", "_xpc_type_endpoint", "_xpc_type_null",
                "_xpc_type_bool", "_xpc_type_int64", "_xpc_type_uint64", "_xpc_type_double",
                "_xpc_type_date", "_xpc_type_data", "_xpc_type_string", "_xpc_type_uuid",
                "_xpc_type_fd", "_xpc_type_shmem", "_xpc_type_array", "_xpc_type_dictionary",
                "_xpc_type_error", "_xpc_type_rich_error", "_xpc_bool_true", "_xpc_bool_false",
                "_xpc_error_connection_invalid", "_xpc_error_connection_interrupted",
                "_xpc_error_key_description"]},
        }.get(library, {})
        names = ["_" + s for s in symbols if s not in data] + (["dyld_stub_binder"] if library == "System" else [])
        text = ".text\n" + "\n".join(f".globl {s}\n{s}:\nret" for s in names) + "\n"
        if data:
            entries = []
            for s, v in data.items():
                body = ".zero 32" if s.startswith("_NSConcrete") else f".long {v:#x}"
                entries.append(f".globl _{s}\n_{s}:\n{body}")
            text += ".data\n" + "\n".join(entries) + "\n"
        asm.write_text(text)
        run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-c", asm, "-o", OUT / (library + ".o"))
        install = {"System":"/usr/lib/libSystem.B.dylib","CoreFoundation":"/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation","objc":"/usr/lib/libobjc.A.dylib","xpc":"/usr/lib/system/libxpc.dylib"}[library]
        run(tool("ld64.lld"), *versions, "-dylib", "-install_name", install, "-o", OUT / ("lib" + library + ".dylib"), OUT / (library + ".o"))
    for name in ["return37", "hello", "corefoundation", "unknown", "loop", "crash", "abi", "objc", "tls", "exit_callbacks", "thread_state", "libc", "file_time", "condvars", "cxx_tls", "mach_server", "mach_client", "mach_errors", "blocks", "xpc_server", "xpc_client", "xpc_missing"]:
        extension=".m" if name=="objc" else ".cpp" if name=="cxx_tls" else ".c"
        flags=["-O0","-fno-stack-protector","-fno-asynchronous-unwind-tables"]
        flags+= ["-fno-exceptions","-fno-rtti"] if extension==".cpp" else ["-ffreestanding"]
        if name in ("blocks", "xpc_server", "xpc_client", "xpc_missing"): flags+=["-fblocks"]
        run(tool("clang"), "-target", "x86_64-apple-macos10.13", *flags, "-c", ROOT / ("tests/fixtures/" + name + extension), "-o", OUT / (name + ".o"))
        libraries = [] if name == "return37" else ["-L", OUT, "-lSystem"]
        if name == "corefoundation": libraries += ["-lCoreFoundation"]
        if name == "objc": libraries += ["-lobjc"]
        if name.startswith("xpc_"): libraries = ["-L", OUT, "-lxpc", "-lSystem"]
        run(tool("ld64.lld"), *versions, "-no_fixup_chains", "-no_uuid", "-e", "_main", "-o", OUT / name, OUT / (name + ".o"), *libraries)
        if name in ["hello","corefoundation","tls","thread_state","libc","condvars","cxx_tls"]:
            run(tool("ld64.lld"),"-arch","x86_64","-platform_version","macos","11.0","11.0","-fixup_chains","-no_uuid","-e","_main","-o",OUT/(name+"-chained"),OUT/(name+".o"),*libraries)
    # A `dlopen` plugin and its host exercise the runtime image registry.
    run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-O0", "-fno-stack-protector",
        "-fno-asynchronous-unwind-tables", "-ffreestanding", "-c",
        ROOT / "tests/fixtures/plugin.c", "-o", OUT / "plugin.o")
    for name, extra in (("libplugin.dylib", []), ("libplugin-chained.dylib", ["-fixup_chains"])):
        run(tool("ld64.lld"), *versions, *extra, "-no_uuid", "-dylib",
            "-install_name", "@rpath/libplugin.dylib", "-o", OUT / name,
            OUT / "plugin.o", "-L", OUT, "-lSystem")
    run(tool("ld64.lld"), *versions, "-no_uuid", "-bundle",
        "-o", OUT / "libplugin.bundle", OUT / "plugin.o", "-L", OUT, "-lSystem")
    run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-O0", "-fno-stack-protector",
        "-fno-asynchronous-unwind-tables", "-ffreestanding", "-c",
        ROOT / "tests/fixtures/plugin_tls.c", "-o", OUT / "plugin_tls.o")
    run(tool("ld64.lld"), *versions, "-no_uuid", "-dylib",
        "-install_name", "@rpath/libplugin-tls.dylib", "-o", OUT / "libplugin-tls.dylib",
        OUT / "plugin_tls.o", "-L", OUT, "-lSystem")
    run(tool("clang"), "-target", "x86_64-apple-macos10.13", "-O0", "-fno-stack-protector",
        "-fno-asynchronous-unwind-tables", "-ffreestanding", "-c",
        ROOT / "tests/fixtures/dlopen.c", "-o", OUT / "dlopen.o")
    run(tool("ld64.lld"), *versions, "-no_fixup_chains", "-no_uuid", "-e", "_main",
        "-o", OUT / "dlopen", OUT / "dlopen.o", "-L", OUT, "-lSystem")

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
