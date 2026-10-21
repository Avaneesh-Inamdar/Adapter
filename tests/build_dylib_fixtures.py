"""Real LLVM-linked multi-image probes, including reexports and error cases."""
from pathlib import Path
import struct
import shutil
from build_fixtures import ROOT,OUT,tool,run

def compile_source(source,dest):
    run(tool('clang'),'-target','x86_64-apple-macos10.13','-O0','-ffreestanding','-fno-stack-protector','-fno-asynchronous-unwind-tables','-c',source,'-o',dest)

def build():
    folder=OUT/'dylibs';folder.mkdir(parents=True,exist_ok=True)
    for variant,minimum,chain in [('classic','10.13','-no_fixup_chains'),('chained','11.0','-fixup_chains')]:
        dest=folder/variant;dest.mkdir(exist_ok=True)
        common=['-arch','x86_64','-platform_version','macos',minimum,minimum,chain,'-no_uuid','-L',OUT,'-lSystem']
        def dylib(name,source,deps=()):
            obj=dest/(name+'.o');compile_source(source,obj)
            run(tool('ld64.lld'),*common,'-dylib','-install_name','@rpath/lib'+name+'.dylib','-rpath','@loader_path','-o',dest/('lib'+name+'.dylib'),obj,*deps)
        dylib('leaf',ROOT/'tests/fixtures/dylib_leaf.c')
        dylib('middle',ROOT/'tests/fixtures/dylib_middle.c',['-L',dest,'-lleaf'])
        dylib('tls',ROOT/'tests/fixtures/tls_library.c')
        compile_source(ROOT/'tests/fixtures/tls_threads.c',dest/'tls-threads.o')
        run(tool('ld64.lld'),*common,'-rpath','@loader_path','-o',dest/'tls-threads',dest/'tls-threads.o','-L',dest,'-ltls')
        obj=dest/'main.o';compile_source(ROOT/'tests/fixtures/dylib_main.c',obj)
        run(tool('ld64.lld'),*common,'-rpath','@executable_path','-o',dest/'multi',obj,'-L',dest,'-lmiddle','-lleaf')
        # The facade reexports leaf; its consumer imports symbols via the facade.
        run(tool('ld64.lld'),*common,'-dylib','-install_name','@rpath/libfacade.dylib','-rpath','@loader_path','-reexport_library',dest/'libleaf.dylib','-o',dest/'libfacade.dylib')
        source=dest/'facade.c';source.write_text('extern int leaf(void); int main(void) { return leaf(); }\n')
        compile_source(source,dest/'facade.o')
        run(tool('ld64.lld'),*common,'-rpath','@loader_path','-o',dest/'facade',dest/'facade.o','-L',dest,'-lfacade')
    # Two-level lookup must ignore the same symbol in a different loaded dylib.
    dest=folder/'namespace';dest.mkdir(exist_ok=True)
    common=['-arch','x86_64','-platform_version','macos','10.13','10.13','-no_fixup_chains','-no_uuid','-L',OUT,'-lSystem']
    for name,value in [('left',11),('right',29)]:
        implementation=dest/(name+'impl.c');implementation.write_text(f'int collision(void) {{return {value};}}\n')
        compile_source(implementation,dest/(name+'impl.o'))
        run(tool('ld64.lld'),*common,'-dylib','-install_name','@rpath/lib'+name+'impl.dylib','-o',dest/('lib'+name+'impl.dylib'),dest/(name+'impl.o'))
        s=dest/(name+'.c');s.write_text(f'extern int collision(void); int {name}(void) {{return collision();}}\n')
        compile_source(s,dest/(name+'.o'))
        run(tool('ld64.lld'),*common,'-dylib','-install_name','@rpath/lib'+name+'.dylib','-rpath','@loader_path','-o',dest/('lib'+name+'.dylib'),dest/(name+'.o'),'-L',dest,'-l'+name+'impl')
    s=dest/'main.c';s.write_text('extern int left(void),right(void);int main(void){return left()==11 && right()==29 ? 0 : 96;}\n')
    compile_source(s,dest/'main.o');run(tool('ld64.lld'),*common,'-rpath','@loader_path','-o',dest/'namespace',dest/'main.o','-L',dest,'-lleft','-lright')
    # Create a valid linked graph, then mutate equal-length install names into a
    # cycle. No mock parser/loader behavior is used for the execution tests.
    cycle=folder/'cycle';cycle.mkdir(exist_ok=True)
    for name,dependency in [('first','leaf'),('other','first')]:
        s=cycle/(name+'.c');s.write_text('int '+name+'(void){return 1;}\n');compile_source(s,cycle/(name+'.o'))
        deps=['-L',folder/'classic','-lleaf'] if name=='first' else ['-L',cycle,'-lfirst']
        run(tool('ld64.lld'),*common,'-dylib','-install_name','@rpath/lib'+name+'.dylib','-o',cycle/('lib'+name+'.dylib'),cycle/(name+'.o'),*deps)
    # leaf and other have different name lengths, so use a same-length alias.
    data=(cycle/'libfirst.dylib').read_bytes();data=data.replace(b'@rpath/libleaf.dylib',b'@rpath/libloop.dylib');(cycle/'libfirst.dylib').write_bytes(data)
    shutil.copyfile(cycle/'libother.dylib',cycle/'libloop.dylib')
    s=cycle/'main.c';s.write_text('extern int first(void);int main(void){return first();}\n');compile_source(s,cycle/'main.o')
    # Link using the original dependency search paths, then runtime follows cycle.
    run(tool('ld64.lld'),*common,'-rpath','@loader_path','-o',cycle/'cycle',cycle/'main.o','-L',cycle,'-lfirst')
    print('Built multi-image Mach-O fixtures (classic/chained/reexports/namespaces/cycle)')

if __name__=='__main__':build()
